//! Telegram login by QR code, run by the agent: the owner only scans the code in their Telegram
//! app (Settings > Devices > Link Desktop Device). Network only, so the offline suite leaves it
//! out of the coverage floor.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use grammers_client::Client;
use grammers_session::Session;
use grammers_session::storages::MemorySession;
use grammers_session::updates::UpdatesLike;
use grammers_tl_types as tl;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use crate::adapters::telegram::{Credentials, write_session};
use crate::adapters::telegram_client::single_attempt_pool;
use crate::setup::config_file::write_private_file;
use crate::setup::qr;

/// Where a QR login stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Progress {
    Starting,
    WaitingForScan,
    PasswordRequired,
    Done { user_id: String },
    Failed(&'static str),
}

/// The `tg://` link a Telegram app opens when it scans the code.
pub fn login_url(token: &[u8]) -> String {
    format!("tg://login?token={}", URL_SAFE_NO_PAD.encode(token))
}

pub struct Paths {
    pub page: PathBuf,
    pub session: PathBuf,
    pub pending: PathBuf,
}

/// A login running in the background of the MCP server.
pub struct QrLogin {
    progress: watch::Receiver<Progress>,
    task: JoinHandle<()>,
}

impl QrLogin {
    pub fn start(credentials: Credentials, paths: Paths) -> Self {
        let (sender, progress) = watch::channel(Progress::Starting);
        let task = tokio::spawn(async move {
            let outcome = run(&credentials, &paths, &sender).await;
            let (tone, title, message) = match &outcome {
                Progress::Done { .. } => (
                    qr::Tone::Success,
                    "You're connected",
                    "Telegram is linked to Messaging MCP. You can close this tab.",
                ),
                Progress::PasswordRequired => (
                    qr::Tone::Notice,
                    "One more step",
                    "Your Telegram account has a 2FA password. Type it in a terminal with \
                     messaging-mcp telegram-password.",
                ),
                Progress::Failed(reason) => (qr::Tone::Failure, "Telegram login stopped", *reason),
                Progress::Starting | Progress::WaitingForScan => {
                    (qr::Tone::Failure, "Telegram login stopped", "")
                }
            };
            let _ = write_private_file(
                &paths.page,
                qr::status_page(tone, title, message).as_bytes(),
            );
            let _ = sender.send(outcome);
        });
        Self { progress, task }
    }

    pub fn progress(&self) -> Progress {
        self.progress.borrow().clone()
    }

    /// Waits up to `limit` for the login to leave its current state.
    pub async fn wait(&mut self, limit: Duration) -> Progress {
        let current = self.progress();
        let _ = tokio::time::timeout(
            limit,
            self.progress.wait_for(|progress| *progress != current),
        )
        .await;
        self.progress()
    }

    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(0))
}

fn write_page(path: &std::path::Path, token: &[u8]) -> bool {
    let Some(matrix) = qr::encode(&login_url(token)) else {
        return false;
    };
    let html = qr::page(
        "Scan with Telegram",
        &[
            "Open <b>Telegram</b> on your phone.",
            "Go to <b>Settings</b>, then <b>Devices</b>.",
            "Tap <b>Link Desktop Device</b> and point it here.",
        ],
        &qr::svg(&matrix),
    );
    write_private_file(path, html.as_bytes()).is_ok()
}

/// Waits until Telegram reports a scan, or the token expires, whichever comes first.
async fn wait_for_scan(updates: &mut mpsc::Receiver<UpdatesLike>, expires: i32) {
    let seconds = (i64::from(expires) - unix_now()).clamp(5, 60);
    let deadline = tokio::time::sleep(Duration::from_secs(u64::try_from(seconds).unwrap_or(30)));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            () = &mut deadline => return,
            update = updates.recv() => match update {
                Some(UpdatesLike::Updates(tl::enums::Updates::UpdateShort(short)))
                    if matches!(short.update, tl::enums::Update::LoginToken) => return,
                Some(_) => {}
                None => return,
            },
        }
    }
}

fn finish(
    authorization: &tl::enums::auth::Authorization,
    session: &MemorySession,
    paths: &Paths,
) -> Progress {
    let tl::enums::auth::Authorization::Authorization(authorization) = authorization else {
        return Progress::Failed("This phone number has no Telegram account.");
    };
    let tl::enums::User::User(user) = &authorization.user else {
        return Progress::Failed("Telegram returned no user.");
    };
    if user.bot {
        return Progress::Failed("Bot accounts are not supported.");
    }
    if write_session(session, &paths.session).is_err() {
        return Progress::Failed("The session could not be saved.");
    }
    Progress::Done {
        user_id: user.id.to_string(),
    }
}

async fn run(
    credentials: &Credentials,
    paths: &Paths,
    progress: &watch::Sender<Progress>,
) -> Progress {
    let session = Arc::new(MemorySession::default());
    let mut pool = single_attempt_pool(Arc::clone(&session), credentials.api_id);
    let handle = pool.handle.thin.clone();
    let client = Client::new(pool.handle);
    let runner = tokio::spawn(pool.runner.run());
    let export = tl::functions::auth::ExportLoginToken {
        api_id: credentials.api_id,
        api_hash: credentials.api_hash.clone(),
        except_ids: Vec::new(),
    };
    let outcome = loop {
        let token = match client.invoke(&export).await {
            Ok(token) => token,
            Err(error) if error.is("SESSION_PASSWORD_NEEDED") => {
                break password_required(&session, paths);
            }
            Err(_) => break Progress::Failed("Telegram refused the login request."),
        };
        let token = match token {
            tl::enums::auth::LoginToken::MigrateTo(migrate) => {
                if session.set_home_dc_id(migrate.dc_id).await.is_err() {
                    break Progress::Failed("Could not switch datacenter.");
                }
                let import = tl::functions::auth::ImportLoginToken {
                    token: migrate.token,
                };
                match client.invoke(&import).await {
                    Ok(token) => token,
                    Err(error) if error.is("SESSION_PASSWORD_NEEDED") => {
                        break password_required(&session, paths);
                    }
                    Err(_) => break Progress::Failed("Telegram refused the login request."),
                }
            }
            other => other,
        };
        match token {
            tl::enums::auth::LoginToken::Token(token) => {
                if !write_page(&paths.page, &token.token) {
                    break Progress::Failed("The QR page could not be written.");
                }
                let _ = progress.send(Progress::WaitingForScan);
                wait_for_scan(&mut pool.updates, token.expires).await;
            }
            tl::enums::auth::LoginToken::Success(success) => {
                break finish(&success.authorization, &session, paths);
            }
            tl::enums::auth::LoginToken::MigrateTo(_) => {
                break Progress::Failed("Telegram asked to switch datacenter twice.");
            }
        }
    };
    handle.quit();
    runner.abort();
    outcome
}

/// The account has a 2FA password. The authorization key is kept so `telegram-password` can
/// finish the login from a terminal, where the password never passes through the agent.
fn password_required(session: &MemorySession, paths: &Paths) -> Progress {
    if write_session(session, &paths.pending).is_err() {
        return Progress::Failed("The pending session could not be saved.");
    }
    Progress::PasswordRequired
}
