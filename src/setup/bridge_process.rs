//! Running the WhatsApp bridge. It touches the owner's account and a phone, so the offline suite
//! leaves it out of the coverage floor.
//!
//! `whatsapp_setup` starts a small supervisor (`messaging-mcp whatsapp-bridge-supervise`) in its
//! own process group, so the bridge keeps running when the MCP client closes. The supervisor
//! runs the bridge, keeps the pairing page in step with the bridge's current QR code, and leaves
//! a result page when the bridge pairs or stops.

use std::fs::{self, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use rustix::process::{Pid, test_kill_process};
use serde::de::DeserializeOwned;

use crate::error::Error;
use crate::secrets::read_secret_file;
use crate::setup::config_file::{ensure_private_dir, write_private_file};
use crate::setup::qr::{self, Tone};
use crate::setup::whatsapp_bridge::{BridgePaths, Health, Pairing, bridge_binary};

/// Exit codes the bridge uses (`whatsapp-bridge/main.go`).
const EXIT_PAIRING_TIMED_OUT: i32 = 4;
const EXIT_LOGGED_OUT: i32 = 5;

fn bridge_error(message: impl Into<String>) -> Error {
    Error::new("WHATSAPP_BRIDGE_FAILED", message)
}

/// The supervisor's process ID, when it is still running.
pub fn running_pid(paths: &BridgePaths) -> Option<i32> {
    let text = fs::read_to_string(paths.pid()).ok()?;
    let pid: i32 = text.trim().parse().ok()?;
    let alive = Pid::from_raw(pid).is_some_and(|pid| test_kill_process(pid).is_ok());
    alive.then_some(pid)
}

/// Starts the supervisor, detached, unless one is already running.
pub fn start(paths: &BridgePaths, config_path: &Path, page: &Path) -> Result<(), Error> {
    if running_pid(paths).is_some() {
        return Ok(());
    }
    ensure_private_dir(&paths.root)?;
    let exe = std::env::current_exe().map_err(|_| bridge_error("Could not find messaging-mcp."))?;
    Command::new(exe)
        .arg("whatsapp-bridge-supervise")
        .arg("--config")
        .arg(config_path)
        .arg("--page")
        .arg(page)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|_| bridge_error("Could not start the bridge supervisor."))?;
    Ok(())
}

/// Stops the supervisor and, with it, the bridge: they share a process group.
pub fn stop(paths: &BridgePaths) -> bool {
    let Some(pid) = running_pid(paths).and_then(Pid::from_raw) else {
        return false;
    };
    let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::TERM);
    let _ = fs::remove_file(paths.pid());
    true
}

async fn get<T: DeserializeOwned>(port: u16, token: &str, route: &str) -> Option<T> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .ok()?;
    let response = client
        .get(format!("http://127.0.0.1:{port}/api/{route}"))
        .bearer_auth(token)
        .send()
        .await
        .ok()?;
    let body = response.bytes().await.ok()?;
    serde_json::from_slice(&body).ok()
}

/// Asks the bridge for its state, with its token, over loopback only.
pub async fn health(port: u16, token: &str) -> Option<Health> {
    get(port, token, "health").await
}

async fn pairing(port: u16, token: &str) -> Option<Pairing> {
    get(port, token, "qr").await
}

fn pairing_page(code: &str) -> Option<String> {
    let matrix = qr::encode(code)?;
    Some(qr::page(
        "Scan with WhatsApp",
        &[
            "Open <b>WhatsApp</b> on your phone.",
            "Go to <b>Settings</b>, then <b>Linked devices</b>.",
            "Tap <b>Link a device</b> and point it here.",
        ],
        &qr::svg(&matrix),
    ))
}

fn final_page(exit: Option<i32>) -> String {
    match exit {
        Some(EXIT_PAIRING_TIMED_OUT) => qr::status_page(
            Tone::Notice,
            "The code expired",
            "Nobody scanned it in about 10 minutes. Ask your assistant to run whatsapp_setup for \
             a new one.",
        ),
        Some(EXIT_LOGGED_OUT) => qr::status_page(
            Tone::Notice,
            "WhatsApp was unlinked",
            "This device was removed in WhatsApp. Ask your assistant to run whatsapp_setup to \
             link it again.",
        ),
        _ => qr::status_page(
            Tone::Notice,
            "The WhatsApp bridge stopped",
            "Ask your assistant to run whatsapp_setup to start it again.",
        ),
    }
}

/// How many times a pairing that timed out is started again: WhatsApp offers about 2.5 minutes
/// of codes per attempt, so four attempts give the owner about 10 minutes to scan.
const PAIRING_ATTEMPTS: u32 = 4;

fn spawn_bridge(binary: &Path, paths: &BridgePaths, port: u16) -> Result<Child, Error> {
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(paths.log())
        .map_err(|_| bridge_error("Could not write the bridge log."))?;
    let log_errors = log
        .try_clone()
        .map_err(|_| bridge_error("Could not write the bridge log."))?;
    Command::new(binary)
        .arg("-store")
        .arg(paths.store())
        .arg("-port")
        .arg(port.to_string())
        .stdin(Stdio::null())
        .stdout(log)
        .stderr(log_errors)
        .spawn()
        .map_err(|_| bridge_error("Could not start the bridge."))
}

/// Keeps the page in step with the bridge until it exits, and returns its exit code.
async fn follow(child: &mut Child, paths: &BridgePaths, port: u16, page: &Path) -> Option<i32> {
    let mut shown = String::new();
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            return status.code();
        }
        let state = match read_secret_file(&paths.token()) {
            Ok(token) => pairing(port, &token).await,
            Err(_) => None,
        };
        if let Some(state) = state {
            if state.paired && state.connected && shown != "connected" {
                let done = qr::status_page(
                    Tone::Success,
                    "You're connected",
                    "WhatsApp is linked to Messaging MCP. You can close this tab.",
                );
                if write_private_file(page, done.as_bytes()).is_ok() {
                    shown = "connected".to_string();
                }
            } else if !state.code.is_empty()
                && state.code != shown
                && let Some(html) = pairing_page(&state.code)
                && write_private_file(page, html.as_bytes()).is_ok()
            {
                shown = state.code;
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// Runs the bridge, keeping the page current, and starts it again when a pairing times out
/// unscanned, up to [`PAIRING_ATTEMPTS`] times. This is the body of
/// `messaging-mcp whatsapp-bridge-supervise`.
pub async fn supervise(paths: &BridgePaths, port: u16, page: &Path) -> Result<(), Error> {
    let exe = std::env::current_exe().map_err(|_| bridge_error("Could not find messaging-mcp."))?;
    let binary = bridge_binary(&exe).ok_or_else(|| bridge_error("No bridge binary."))?;
    // A fresh log for each supervisor; attempts within it append.
    let _ = fs::remove_file(paths.log());
    write_private_file(&paths.pid(), std::process::id().to_string().as_bytes())?;
    let mut attempts = 1;
    let exit = loop {
        let mut child = spawn_bridge(&binary, paths, port)?;
        let exit = follow(&mut child, paths, port, page).await;
        if exit == Some(EXIT_PAIRING_TIMED_OUT) && attempts < PAIRING_ATTEMPTS {
            attempts += 1;
            continue;
        }
        break exit;
    };
    let _ = write_private_file(page, final_page(exit).as_bytes());
    let _ = fs::remove_file(paths.pid());
    Ok(())
}
