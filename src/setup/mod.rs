//! Setup run by the agent ([ADR-007]): the config, Telegram credentials and login by QR, the
//! WhatsApp bridge and the mode. The owner only scans QR codes and, for an account with a 2FA
//! password, types it in a terminal. Approving each live message stays in the terminal.
//!
//! [ADR-007]: ../../docs/decisions/ADR-007-agent-runs-setup.md

pub mod approval;
pub mod bridge_process;
pub mod config_file;
pub mod qr;
pub mod telegram_qr;
pub mod whatsapp_bridge;
mod whatsapp_flow;

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::adapters::telegram::read_credentials;
use crate::adapters::telegram_client;
use crate::config::{Config, load_config_from};
use crate::error::Error;
use crate::plan::Mode;
use crate::secrets::{Env, process_env, read_secret_file};
use config_file::{ensure_private_dir, section, update_config, write_private_file};
use telegram_qr::{Paths, Progress, QrLogin};
use whatsapp_bridge::{BridgePaths, DEFAULT_PORT, port_from_base_url};

const WAIT: Duration = Duration::from_secs(25);
const TELEGRAM_PAGE: &str = "messaging-mcp-telegram-login.html";
const WHATSAPP_PAGE: &str = "messaging-mcp-whatsapp-pairing.html";

const CREATE_TELEGRAM_APP: &str = "Create a Telegram API application: open https://my.telegram.org \
in a browser, sign in with the user's phone number (Telegram sends the code to the user's \
Telegram app, so ask the user for it), open API development tools, create an application with any \
title and short name, then call telegram_set_credentials with its api_id and api_hash. Never show \
the api_hash to the user or anyone else.";

fn setup_error(code: &'static str, message: impl Into<String>) -> Error {
    Error::new(code, message)
}

/// The setup state kept by one MCP server process.
pub struct Setup {
    config_path: PathBuf,
    config_dir: PathBuf,
    downloads: PathBuf,
    telegram: Mutex<Option<QrLogin>>,
}

impl Setup {
    pub fn new(config_path: &Path, env: Env<'_>) -> Result<Self, Error> {
        let config_path = std::path::absolute(config_path)
            .map_err(|_| setup_error("CONFIG_READ_FAILED", "Cannot resolve the config path."))?;
        let config_dir = config_path
            .parent()
            .ok_or_else(|| setup_error("CONFIG_READ_FAILED", "Cannot resolve the config folder."))?
            .to_path_buf();
        let downloads = env("HOME")
            .map(|home| Path::new(&home).join("Downloads"))
            .filter(|path| path.is_dir())
            .unwrap_or_else(|| config_dir.clone());
        Ok(Self {
            config_path,
            config_dir,
            downloads,
            telegram: Mutex::new(None),
        })
    }

    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    fn telegram_paths(&self, config: &Config) -> Paths {
        let session = config
            .telegram
            .as_ref()
            .and_then(|telegram| telegram.session_file.clone())
            .unwrap_or_else(|| self.config_dir.join("telegram-session.json"));
        Paths {
            page: self.downloads.join(TELEGRAM_PAGE),
            session,
            pending: self.config_dir.join("telegram-pending-session.json"),
        }
    }

    fn bridge(&self) -> BridgePaths {
        BridgePaths::new(&self.config_dir)
    }

    fn whatsapp_port(config: &Config) -> u16 {
        config
            .whatsapp
            .as_ref()
            .and_then(|whatsapp| port_from_base_url(&whatsapp.base_url))
            .unwrap_or(DEFAULT_PORT)
    }

    /// Saves the Telegram application's credentials: the hash in a private file, the ID in the
    /// config. The session and its expected user are set by `telegram_login`.
    pub fn set_telegram_credentials(&self, api_id: i32, api_hash: &str) -> Result<Config, Error> {
        let api_hash = api_hash.trim();
        let hash_is_valid = api_hash.len() == 32 && api_hash.bytes().all(|b| b.is_ascii_hexdigit());
        if api_id <= 0 || !hash_is_valid {
            return Err(setup_error(
                "TELEGRAM_CREDENTIALS_INVALID",
                "api_id is a positive number and api_hash 32 hexadecimal characters.",
            ));
        }
        ensure_private_dir(&self.config_dir)?;
        let hash_file = self.config_dir.join("telegram-api-hash");
        write_private_file(&hash_file, api_hash.as_bytes())?;
        let session_file = self.config_dir.join("telegram-session.json");
        update_config(&self.config_path, |config| {
            let telegram = section(config, "telegram");
            telegram.insert("apiId".into(), json!(api_id));
            telegram.insert("apiHashFile".into(), json!(hash_file));
            telegram
                .entry("sessionFile")
                .or_insert_with(|| json!(session_file));
        })
    }

    /// Switches between dry-run and live. Live plans still need the owner's approval.
    pub fn set_mode(&self, mode: Mode) -> Result<Config, Error> {
        update_config(&self.config_path, |config| {
            config.insert("mode".into(), json!(mode));
        })
    }

    /// Starts or continues the Telegram QR login, waiting up to 25 seconds for progress.
    pub async fn telegram_login(&self, _loaded: &Config) -> Result<Value, Error> {
        // The file, not the copy the server loaded at start: the owner may have edited it since,
        // and a stale "already logged in" would hide a login that just finished.
        let fresh = load_config_from(&self.config_path)?;
        let config = &fresh;
        let settings = config.telegram.as_ref().ok_or_else(|| {
            setup_error(
                "TELEGRAM_UNCONFIGURED",
                "Call telegram_set_credentials first.",
            )
        })?;
        let paths = self.telegram_paths(config);
        let mut guard = self.telegram.lock().await;
        if guard.is_none() && paths.session.exists() && settings.expected_user_id.is_some() {
            return Ok(json!({
                "status": "logged_in",
                "userId": settings.expected_user_id,
                "notice": "Already logged in. To log in again, remove the session file first.",
            }));
        }
        // A finished login keeps its outcome until it is reported; only a login that ended
        // without one starts over.
        let restart = guard.as_ref().is_none_or(|login| {
            login.is_finished()
                && matches!(
                    login.progress(),
                    Progress::Starting | Progress::WaitingForScan
                )
        });
        if restart && paths.session.exists() {
            // A session saved by an earlier login whose outcome was never recorded: adopt it
            // after checking with Telegram who it belongs to, instead of asking for a new scan.
            if let Ok(user_id) = telegram_client::session_user_id(settings, &paths.session).await {
                update_config(&self.config_path, |config| {
                    section(config, "telegram").insert("expectedUserId".into(), json!(user_id));
                })?;
                return Ok(json!({ "status": "logged_in", "userId": user_id }));
            }
        }
        if restart {
            let credentials = read_credentials(settings, &process_env).map_err(|_| {
                setup_error(
                    "TELEGRAM_UNCONFIGURED",
                    "Call telegram_set_credentials first.",
                )
            })?;
            *guard = Some(QrLogin::start(credentials, paths));
        }
        let progress = match guard.as_mut() {
            Some(login) if login.is_finished() => login.progress(),
            Some(login) => login.wait(WAIT).await,
            None => Progress::Starting,
        };
        let page = self.downloads.join(TELEGRAM_PAGE);
        Ok(match progress {
            Progress::Starting => {
                json!({ "status": "starting", "next": "Call telegram_login again." })
            }
            Progress::WaitingForScan => json!({
                "status": "waiting_for_scan",
                "page": page,
                "next": format!(
                    "Open the page for the user (on macOS: `open {}`) and ask them to scan it: \
                     Telegram > Settings > Devices > Link Desktop Device. Then call \
                     telegram_login again to wait for the scan.",
                    page.display()
                ),
            }),
            Progress::PasswordRequired => {
                *guard = None;
                json!({
                    "status": "password_required",
                    "next": "This account has a Telegram 2FA password. Ask the user to run \
                             `messaging-mcp telegram-password` in their own terminal and type it \
                             there. Never ask for the password in chat.",
                })
            }
            Progress::Done { user_id } => {
                *guard = None;
                let session = self.telegram_paths(config).session;
                update_config(&self.config_path, |config| {
                    let telegram = section(config, "telegram");
                    telegram.insert("expectedUserId".into(), json!(user_id));
                    telegram.insert("sessionFile".into(), json!(session));
                })?;
                json!({ "status": "logged_in", "userId": user_id })
            }
            Progress::Failed(message) => {
                *guard = None;
                return Err(setup_error("TELEGRAM_LOGIN_FAILED", message));
            }
        })
    }

    /// Reports the bridge's state, waiting up to 25 seconds for it to change.
    pub async fn whatsapp_status(&self, config: &Config) -> Result<Value, Error> {
        if bridge_process::running_pid(&self.bridge()).is_none() {
            return Ok(json!({
                "status": "not_running",
                "next": "Call whatsapp_setup to start the bridge.",
            }));
        }
        self.whatsapp_wait(Self::whatsapp_port(config), true).await
    }

    /// Stops the bridge. Pairing is kept: starting it again reconnects without a QR code.
    pub fn whatsapp_stop(&self) -> Value {
        json!({ "stopped": bridge_process::stop(&self.bridge()) })
    }

    /// What is configured, what is missing, and the next steps in order.
    pub async fn status(&self, config: &Config, recipients: usize) -> Value {
        let telegram = config.telegram.as_ref();
        let credentials = telegram.is_some_and(|t| read_credentials(t, &process_env).is_ok());
        let paths = self.telegram_paths(config);
        let logged_in =
            paths.session.exists() && telegram.is_some_and(|t| t.expected_user_id.is_some());
        let bridge = self.bridge();
        let running = bridge_process::running_pid(&bridge).is_some();
        let port = Self::whatsapp_port(config);
        let health = match (running, read_secret_file(&bridge.token())) {
            (true, Ok(token)) => bridge_process::health(port, &token).await,
            _ => None,
        };
        let paired = health.as_ref().is_some_and(|h| h.connected && h.paired);
        let mut next = Vec::new();
        if !credentials {
            next.push(CREATE_TELEGRAM_APP.to_string());
        } else if !logged_in {
            next.push("Call telegram_login and show the user the QR page.".to_string());
        }
        if !paired {
            next.push("Call whatsapp_setup to start and pair the WhatsApp bridge.".to_string());
        }
        if config.mode == Mode::DryRun && (logged_in || paired) {
            next.push("Call set_mode with live when the user wants real messages.".to_string());
        }
        json!({
            "configPath": self.config_path,
            "mode": config.mode,
            "telegram": {
                "credentials": credentials,
                "loggedIn": logged_in,
                "userId": telegram.and_then(|t| t.expected_user_id.clone()),
            },
            "whatsapp": {
                "bridgeInstalled": std::env::current_exe().ok().and_then(|exe| whatsapp_bridge::bridge_binary(&exe)).is_some_and(|binary| binary.is_file()),
                "bridgeRunning": running,
                "paired": paired,
                "account": config.whatsapp.as_ref().and_then(|w| w.account_label.clone()),
            },
            "allowedRecipients": recipients,
            "liveSendsRequireTerminalApproval": true,
            "next": next,
        })
    }
}
