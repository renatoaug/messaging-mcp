use std::fs::{DirBuilder, OpenOptions};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, Row, params};
use rustix::fs::OFlags;

use crate::config::{Recipient, TelegramPeer};
use crate::error::Error;
use crate::plan::{Channel, Mode, Plan, PlanError, Receipt, Status};
use crate::secrets::is_owner_only;

const DEFAULT_BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// One entry per schema version. `PRAGMA user_version` records how many have run.
const MIGRATIONS: &[&str] = &["
    CREATE TABLE plan (
        id TEXT PRIMARY KEY,
        mode TEXT NOT NULL,
        recipient_id TEXT NOT NULL,
        recipient_label TEXT NOT NULL,
        sender TEXT NOT NULL,
        channel TEXT NOT NULL,
        target TEXT NOT NULL,
        text TEXT NOT NULL,
        fingerprint TEXT NOT NULL,
        status TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        expires_at INTEGER NOT NULL,
        approved_at INTEGER,
        attempted_at INTEGER,
        completed_at INTEGER,
        receipt_accepted INTEGER NOT NULL DEFAULT 0,
        receipt_provider_message_id TEXT,
        error_code TEXT,
        error_message TEXT
    );
    CREATE INDEX plan_attempted_at ON plan (attempted_at);
    CREATE TABLE recipient (
        id TEXT PRIMARY KEY,
        label TEXT NOT NULL,
        channel TEXT NOT NULL,
        target TEXT NOT NULL,
        created_at INTEGER NOT NULL
    );
"];

const PLAN_COLUMNS: &str = "id, mode, recipient_id, recipient_label, sender, channel, target, \
    text, fingerprint, status, created_at, expires_at, approved_at, attempted_at, completed_at, \
    receipt_accepted, receipt_provider_message_id, error_code, error_message";

fn state_error() -> Error {
    Error::new(
        "STATE_ERROR",
        "The local state could not be read or written. Check this plan's status before taking \
         further action; never assume that an earlier attempt was not sent.",
    )
}

fn busy_error() -> Error {
    Error::new(
        "STATE_BUSY",
        "State is busy. Check this plan's status before taking further action; never assume that \
         an earlier attempt was not sent.",
    )
}

fn permissions_error(message: &str) -> Error {
    Error::new("STATE_PERMISSIONS", message)
}

fn sql_error(_: rusqlite::Error) -> Error {
    state_error()
}

fn open_private_database(directory: &Path) -> Result<Connection, Error> {
    let directory_error = "stateDir must be an owner-only directory (0700), not a symlink.";
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)
        .map_err(|_| permissions_error(directory_error))?;
    let metadata = directory
        .symlink_metadata()
        .map_err(|_| permissions_error(directory_error))?;
    if !metadata.is_dir() || !is_owner_only(&metadata) {
        return Err(permissions_error(directory_error));
    }
    let path = directory.join("plans.sqlite");
    let file_error = "The state database must be an owner-only regular file (0600).";
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(OFlags::NOFOLLOW.bits() as i32)
        .open(&path)
        .map_err(|_| permissions_error(file_error))?;
    let metadata = file.metadata().map_err(|_| permissions_error(file_error))?;
    if !metadata.is_file() || !is_owner_only(&metadata) {
        return Err(permissions_error(file_error));
    }
    Connection::open(&path).map_err(sql_error)
}

fn migrate(connection: &Connection) -> Result<(), Error> {
    connection
        .execute_batch("BEGIN IMMEDIATE")
        .map_err(|_| busy_error())?;
    let result = (|| -> rusqlite::Result<()> {
        let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        for (index, migration) in MIGRATIONS
            .iter()
            .enumerate()
            .skip(usize::try_from(version).unwrap_or(0))
        {
            connection.execute_batch(migration)?;
            connection.execute_batch(&format!("PRAGMA user_version = {}", index + 1))?;
        }
        Ok(())
    })();
    let end = if result.is_ok() { "COMMIT" } else { "ROLLBACK" };
    connection.execute_batch(end).map_err(sql_error)?;
    result.map_err(sql_error)
}

/// The durable ledger of plans, shared by every client on this machine.
pub struct PlanStore {
    connection: Mutex<Connection>,
}

impl PlanStore {
    pub fn open(directory: &Path) -> Result<Self, Error> {
        Self::open_with_busy_timeout(directory, DEFAULT_BUSY_TIMEOUT)
    }

    /// Like [`Self::open`], choosing how long to wait for another process's write lock.
    pub fn open_with_busy_timeout(directory: &Path, busy: Duration) -> Result<Self, Error> {
        let connection = open_private_database(directory)?;
        connection.busy_timeout(busy).map_err(sql_error)?;
        connection
            .execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
            .map_err(sql_error)?;
        migrate(&connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, Error> {
        self.connection.lock().map_err(|_| state_error())
    }

    pub fn get(&self, id: &str) -> Result<Option<Plan>, Error> {
        select_plan(&*self.lock()?, id)
    }

    pub fn get_recipient(&self, id: &str) -> Result<Option<Recipient>, Error> {
        select_recipient(&*self.lock()?, id)
    }

    pub fn list_recipients(&self) -> Result<Vec<Recipient>, Error> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare("SELECT id, label, channel, target FROM recipient ORDER BY id")
            .map_err(sql_error)?;
        let rows = statement.query_map([], raw_recipient).map_err(sql_error)?;
        rows.map(|row| row.map_err(sql_error).and_then(recipient_from_raw))
            .collect()
    }

    /// Runs `operation` in a `BEGIN IMMEDIATE` transaction, rolling back when it fails.
    pub fn transaction<T>(
        &self,
        operation: impl FnOnce(&Transaction<'_>) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let connection = self.lock()?;
        connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|_| busy_error())?;
        let result = operation(&Transaction {
            connection: &connection,
        })
        .and_then(|value| {
            connection.execute_batch("COMMIT").map_err(sql_error)?;
            Ok(value)
        });
        if result.is_err() {
            // Fails harmlessly when COMMIT already ended the transaction.
            let _ = connection.execute_batch("ROLLBACK");
        }
        result
    }
}

/// The operations available inside [`PlanStore::transaction`].
pub struct Transaction<'a> {
    connection: &'a Connection,
}

impl Transaction<'_> {
    pub fn get(&self, id: &str) -> Result<Option<Plan>, Error> {
        select_plan(self.connection, id)
    }

    pub fn get_recipient(&self, id: &str) -> Result<Option<Recipient>, Error> {
        select_recipient(self.connection, id)
    }

    pub fn insert(&self, plan: &Plan) -> Result<(), Error> {
        self.connection
            .execute(
                &format!(
                    "INSERT INTO plan ({PLAN_COLUMNS}) VALUES \
                     (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)"
                ),
                params![
                    plan.id,
                    plan.mode.as_str(),
                    plan.recipient_id,
                    plan.recipient_label,
                    plan.sender,
                    plan.channel.as_str(),
                    plan.target,
                    plan.text,
                    plan.fingerprint,
                    plan.status.as_str(),
                    plan.created_at,
                    plan.expires_at,
                    plan.approved_at,
                    plan.attempted_at,
                    plan.completed_at,
                    plan.receipt.is_some(),
                    plan.receipt
                        .as_ref()
                        .and_then(|r| r.provider_message_id.as_deref()),
                    plan.error.as_ref().map(|e| e.code.as_str()),
                    plan.error.as_ref().map(|e| e.message.as_str()),
                ],
            )
            .map_err(sql_error)?;
        Ok(())
    }

    pub fn update(&self, plan: &Plan) -> Result<(), Error> {
        self.connection
            .execute(
                "UPDATE plan SET status=?, approved_at=?, attempted_at=?, completed_at=?, \
                 receipt_accepted=?, receipt_provider_message_id=?, error_code=?, \
                 error_message=? WHERE id=?",
                params![
                    plan.status.as_str(),
                    plan.approved_at,
                    plan.attempted_at,
                    plan.completed_at,
                    plan.receipt.is_some(),
                    plan.receipt
                        .as_ref()
                        .and_then(|r| r.provider_message_id.as_deref()),
                    plan.error.as_ref().map(|e| e.code.as_str()),
                    plan.error.as_ref().map(|e| e.message.as_str()),
                    plan.id,
                ],
            )
            .map_err(sql_error)?;
        Ok(())
    }

    pub fn attempts_since(&self, timestamp: i64) -> Result<i64, Error> {
        self.connection
            .query_row(
                "SELECT COUNT(*) FROM plan WHERE attempted_at >= ?",
                params![timestamp],
                |row| row.get(0),
            )
            .map_err(sql_error)
    }

    pub fn insert_recipient(&self, recipient: &Recipient, created_at: i64) -> Result<(), Error> {
        let target = match recipient {
            Recipient::Whatsapp { target, .. } => target.clone(),
            Recipient::Telegram { target, .. } => {
                serde_json::to_string(target).map_err(|_| state_error())?
            }
        };
        self.connection
            .execute(
                "INSERT INTO recipient (id, label, channel, target, created_at) \
                 VALUES (?,?,?,?,?)",
                params![
                    recipient.id(),
                    recipient.label(),
                    recipient.channel().as_str(),
                    target,
                    created_at
                ],
            )
            .map_err(sql_error)?;
        Ok(())
    }

    pub fn delete_recipient(&self, id: &str) -> Result<bool, Error> {
        let deleted = self
            .connection
            .execute("DELETE FROM recipient WHERE id = ?", params![id])
            .map_err(sql_error)?;
        Ok(deleted > 0)
    }
}

struct RawPlan {
    plan: Plan,
    mode: String,
    channel: String,
    status: String,
}

fn raw_plan(row: &Row<'_>) -> rusqlite::Result<RawPlan> {
    let accepted: bool = row.get("receipt_accepted")?;
    let error_code: Option<String> = row.get("error_code")?;
    let error_message: Option<String> = row.get("error_message")?;
    Ok(RawPlan {
        mode: row.get("mode")?,
        channel: row.get("channel")?,
        status: row.get("status")?,
        plan: Plan {
            id: row.get("id")?,
            mode: Mode::DryRun,
            recipient_id: row.get("recipient_id")?,
            recipient_label: row.get("recipient_label")?,
            sender: row.get("sender")?,
            channel: Channel::Whatsapp,
            target: row.get("target")?,
            text: row.get("text")?,
            fingerprint: row.get("fingerprint")?,
            created_at: row.get("created_at")?,
            expires_at: row.get("expires_at")?,
            status: Status::Pending,
            approved_at: row.get("approved_at")?,
            attempted_at: row.get("attempted_at")?,
            completed_at: row.get("completed_at")?,
            receipt: if accepted {
                Some(Receipt {
                    accepted: true,
                    provider_message_id: row.get("receipt_provider_message_id")?,
                })
            } else {
                None
            },
            error: error_code.map(|code| PlanError {
                code,
                message: error_message.unwrap_or_default(),
            }),
        },
    })
}

fn select_plan(connection: &Connection, id: &str) -> Result<Option<Plan>, Error> {
    if !is_plan_id(id) {
        return Err(Error::new("PLAN_ID_INVALID", "Invalid plan ID."));
    }
    let raw = connection
        .query_row(
            &format!("SELECT {PLAN_COLUMNS} FROM plan WHERE id = ?"),
            params![id],
            raw_plan,
        )
        .optional()
        .map_err(sql_error)?;
    raw.map(|raw| {
        let (Some(mode), Some(channel), Some(status)) = (
            Mode::parse(&raw.mode),
            Channel::parse(&raw.channel),
            Status::parse(&raw.status),
        ) else {
            return Err(state_error());
        };
        Ok(Plan {
            mode,
            channel,
            status,
            ..raw.plan
        })
    })
    .transpose()
}

fn is_plan_id(id: &str) -> bool {
    id.strip_prefix("msg_").is_some_and(|hash| {
        hash.len() == 32
            && hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

type RawRecipient = (String, String, String, String);

fn raw_recipient(row: &Row<'_>) -> rusqlite::Result<RawRecipient> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
}

fn recipient_from_raw((id, label, channel, target): RawRecipient) -> Result<Recipient, Error> {
    match Channel::parse(&channel) {
        Some(Channel::Whatsapp) => Ok(Recipient::Whatsapp { id, label, target }),
        Some(Channel::Telegram) => {
            let target: TelegramPeer = serde_json::from_str(&target).map_err(|_| state_error())?;
            Ok(Recipient::Telegram { id, label, target })
        }
        None => Err(state_error()),
    }
}

fn select_recipient(connection: &Connection, id: &str) -> Result<Option<Recipient>, Error> {
    let raw = connection
        .query_row(
            "SELECT id, label, channel, target FROM recipient WHERE id = ?",
            params![id],
            raw_recipient,
        )
        .optional()
        .map_err(sql_error)?;
    raw.map(recipient_from_raw).transpose()
}
