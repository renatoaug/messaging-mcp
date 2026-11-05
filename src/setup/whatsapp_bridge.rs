//! Where the WhatsApp bridge lives and how to read its state. Building and running it is in
//! `bridge_process.rs`; this file holds what the offline suite can check.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The bridge's repository and the commit whose HTTP contract the adapter was checked against.
pub const REPOSITORY: &str = "https://github.com/verygoodplugins/whatsapp-mcp.git";
pub const COMMIT: &str = "a1af56c43a8e8ec4196f4c65e520a12d44bf79cc";
pub const DEFAULT_PORT: u16 = 8080;

/// Everything the bridge owns, under `<config folder>/whatsapp-bridge/`.
#[derive(Clone, Debug)]
pub struct BridgePaths {
    pub root: PathBuf,
}

impl BridgePaths {
    pub fn new(config_dir: &Path) -> Self {
        Self {
            root: config_dir.join("whatsapp-bridge"),
        }
    }
    /// The cloned repository.
    pub fn source(&self) -> PathBuf {
        self.root.join("src")
    }
    pub fn binary(&self) -> PathBuf {
        self.root.join("bin").join("whatsapp-bridge")
    }
    /// The bridge's working folder: it keeps `store/` (session, token, history) here.
    pub fn run_dir(&self) -> PathBuf {
        self.root.join("run")
    }
    pub fn token(&self) -> PathBuf {
        self.run_dir().join("store").join(".bridge-token")
    }
    pub fn device_database(&self) -> PathBuf {
        self.run_dir().join("store").join("whatsapp.db")
    }
    /// Everything the bridge prints, including the pairing QR code.
    pub fn log(&self) -> PathBuf {
        self.root.join("bridge.log")
    }
    pub fn build_log(&self) -> PathBuf {
        self.root.join("build.log")
    }
    /// The supervisor's process ID.
    pub fn pid(&self) -> PathBuf {
        self.root.join("bridge.pid")
    }
}

/// The parts of `GET /api/health` we use.
#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
pub struct Health {
    #[serde(default)]
    pub connected: bool,
    #[serde(default)]
    pub paired: bool,
}

/// The paired account's number from a whatsmeow device JID such as `5511999999999:12@s.whatsapp.net`.
pub fn phone_from_jid(jid: &str) -> Option<String> {
    let user = jid.split('@').next()?.split(':').next()?;
    let valid = (8..=15).contains(&user.len()) && user.bytes().all(|b| b.is_ascii_digit());
    valid.then(|| format!("+{user}"))
}

/// The port in a `http://127.0.0.1:PORT/api` URL.
pub fn port_from_base_url(base_url: &str) -> Option<u16> {
    url::Url::parse(base_url).ok()?.port()
}
