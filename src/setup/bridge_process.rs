//! Building and running the WhatsApp bridge. It needs `git`, Go and a C compiler, and it
//! touches the network and the owner's account, so the offline suite leaves it out of the
//! coverage floor.
//!
//! The bridge draws its pairing QR code only when its output is a terminal. A small supervisor
//! (`messaging-mcp whatsapp-bridge-supervise`) gives it a pseudo-terminal and copies what it
//! prints to `bridge.log`, where the setup tools read the code. The supervisor runs in its own
//! process group, so the bridge keeps running when the MCP client closes.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use rustix::process::{Pid, test_kill_process};
use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};

use crate::error::Error;
use crate::setup::config_file::{ensure_private_dir, write_private_file};
use crate::setup::qr;
use crate::setup::whatsapp_bridge::{BridgePaths, COMMIT, Health, REPOSITORY};

const MAX_LOG_BYTES: u64 = 1_000_000;

fn bridge_error(message: impl Into<String>) -> Error {
    Error::new("WHATSAPP_BRIDGE_FAILED", message)
}

fn tool_available(tool: &str) -> bool {
    Command::new(tool)
        .arg("version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn run_logged(command: &mut Command, log: &Path, what: &str) -> Result<(), Error> {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(log)
        .map_err(|_| bridge_error("Could not write the build log."))?;
    let error_file = file
        .try_clone()
        .map_err(|_| bridge_error("Could not write the build log."))?;
    let status = command
        .stdin(Stdio::null())
        .stdout(file)
        .stderr(error_file)
        .status()
        .map_err(|_| bridge_error(format!("Could not run {what}.")))?;
    if status.success() {
        Ok(())
    } else {
        Err(bridge_error(format!(
            "{what} failed. Details are in {}.",
            log.display()
        )))
    }
}

/// Clones the pinned commit and builds the bridge. Takes a few minutes on the first run.
pub fn build(paths: &BridgePaths) -> Result<(), Error> {
    for tool in ["git", "go"] {
        if !tool_available(tool) {
            return Err(bridge_error(format!(
                "`{tool}` is not installed. On macOS: `brew install {tool}`."
            )));
        }
    }
    ensure_private_dir(&paths.root)?;
    let log = paths.build_log();
    let source = paths.source();
    if !source.join(".git").exists() {
        let _ = fs::remove_dir_all(&source);
        run_logged(
            Command::new("git")
                .args(["clone", "--quiet", REPOSITORY])
                .arg(&source),
            &log,
            "git clone",
        )?;
    }
    run_logged(
        Command::new("git")
            .arg("-C")
            .arg(&source)
            .args(["checkout", "--quiet", COMMIT]),
        &log,
        "git checkout of the pinned commit",
    )?;
    let binary = paths.binary();
    if let Some(parent) = binary.parent() {
        ensure_private_dir(parent)?;
    }
    run_logged(
        Command::new("go")
            .args(["build", "-o"])
            .arg(&binary)
            .arg(".")
            .current_dir(source.join("whatsapp-bridge"))
            .env("CGO_ENABLED", "1"),
        &log,
        "go build",
    )
}

/// The supervisor's process ID, when it is still running.
pub fn running_pid(paths: &BridgePaths) -> Option<i32> {
    let text = fs::read_to_string(paths.pid()).ok()?;
    let pid: i32 = text.trim().parse().ok()?;
    let alive = Pid::from_raw(pid).is_some_and(|pid| test_kill_process(pid).is_ok());
    alive.then_some(pid)
}

/// Starts the supervisor, detached, unless one is already running. It keeps `page` showing the
/// current pairing QR code.
pub fn start(paths: &BridgePaths, config_path: &Path, page: &Path) -> Result<(), Error> {
    if running_pid(paths).is_some() {
        return Ok(());
    }
    ensure_private_dir(&paths.run_dir())?;
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

/// Stops the supervisor and, with it, the bridge.
pub fn stop(paths: &BridgePaths) -> bool {
    let Some(pid) = running_pid(paths).and_then(Pid::from_raw) else {
        return false;
    };
    let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::TERM);
    let _ = fs::remove_file(paths.pid());
    true
}

/// Runs the bridge on a pseudo-terminal until it exits, copying its output to the log.
/// This is the body of `messaging-mcp whatsapp-bridge-supervise`.
pub fn supervise(paths: &BridgePaths, port: u16, page: &Path) -> Result<(), Error> {
    let failed = || bridge_error("The bridge could not be started on a pseudo-terminal.");
    let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).map_err(|_| failed())?;
    grantpt(&master).map_err(|_| failed())?;
    unlockpt(&master).map_err(|_| failed())?;
    let name = ptsname(&master, Vec::new()).map_err(|_| failed())?;
    let slave = OpenOptions::new()
        .read(true)
        .write(true)
        .open(name.to_string_lossy().as_ref())
        .map_err(|_| failed())?;
    let mut child = Command::new(paths.binary())
        .current_dir(paths.run_dir())
        .env("WEBHOOK_ENABLED", "false")
        .env("WHATSAPP_AUTO_DOWNLOAD_MEDIA", "false")
        .env("FORWARD_SELF", "false")
        .env("WHATSAPP_BRIDGE_PORT", port.to_string())
        .env("WHATSAPP_DEVICE_NAME", "Messaging MCP")
        .stdin(slave.try_clone().map_err(|_| failed())?)
        .stdout(slave.try_clone().map_err(|_| failed())?)
        .stderr(slave)
        .spawn()
        .map_err(|_| failed())?;
    let pid = std::process::id().to_string();
    write_private_file(&paths.pid(), pid.as_bytes())?;
    copy_output(File::from(master), &paths.log(), page);
    let stopped = qr::status_page(
        qr::Tone::Notice,
        "The WhatsApp bridge stopped",
        "Ask your assistant to run whatsapp_setup to start it again.",
    );
    let _ = write_private_file(page, stopped.as_bytes());
    let _ = child.wait();
    let _ = fs::remove_file(paths.pid());
    Ok(())
}

fn open_log(path: &Path) -> Option<File> {
    OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .ok()
}

/// Keeps the page in step with the last QR code in `recent` output.
fn refresh_page(recent: &str, page: &Path, shown: &mut Option<qr::Matrix>) {
    let Some(matrix) = qr::parse_last_half_block_code(recent) else {
        return;
    };
    if shown.as_ref() == Some(&matrix) {
        return;
    }
    let html = qr::page(
        "Scan with WhatsApp",
        &[
            "Open <b>WhatsApp</b> on your phone.",
            "Go to <b>Settings</b>, then <b>Linked devices</b>.",
            "Tap <b>Link a device</b> and point it here.",
        ],
        &qr::svg(&matrix),
    );
    if write_private_file(page, html.as_bytes()).is_ok() {
        *shown = Some(matrix);
    }
}

/// Copies the terminal's output to the log, starting the log over when it grows past 1 MB, and
/// keeps the QR page current. The bridge may print message contents, so the log stays
/// owner-only and small.
fn copy_output(mut master: File, log_path: &Path, page: &Path) {
    let Some(mut log) = open_log(log_path) else {
        return;
    };
    let mut written = 0u64;
    let mut buffer = [0u8; 8192];
    let mut recent = String::new();
    let mut shown = None;
    // A read error means the bridge closed its terminal: it has exited.
    while let Ok(read) = master.read(&mut buffer) {
        if read == 0 {
            break;
        }
        if written > MAX_LOG_BYTES {
            match open_log(log_path) {
                Some(fresh) => log = fresh,
                None => return,
            }
            written = 0;
        }
        if log.write_all(&buffer[..read]).is_err() {
            return;
        }
        written += read as u64;
        recent.push_str(&String::from_utf8_lossy(&buffer[..read]));
        if recent.len() > 64_000 {
            let cut = recent.len() - 32_000;
            let cut = (cut..recent.len())
                .find(|i| recent.is_char_boundary(*i))
                .unwrap_or(0);
            recent.drain(..cut);
        }
        refresh_page(&recent, page, &mut shown);
    }
}

/// Asks the bridge for its state, with its token, over loopback only.
pub async fn health(port: u16, token: &str) -> Option<Health> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(3))
        .build()
        .ok()?;
    let response = client
        .get(format!("http://127.0.0.1:{port}/api/health"))
        .bearer_auth(token)
        .send()
        .await
        .ok()?;
    let body = response.bytes().await.ok()?;
    serde_json::from_slice(&body).ok()
}

/// The paired account's JID, read from the bridge's device table.
pub fn device_jid(paths: &BridgePaths) -> Option<String> {
    let connection = rusqlite::Connection::open_with_flags(
        paths.device_database(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .ok()?;
    connection
        .query_row("SELECT jid FROM whatsmeow_device LIMIT 1", [], |row| {
            row.get(0)
        })
        .ok()
}
