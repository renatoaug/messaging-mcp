//! Where the WhatsApp bridge lives and how to read its answers. Running it is in
//! `bridge_process.rs`; this file holds what the offline suite can check.
//!
//! The bridge is our own small Go program on `whatsmeow`, in `whatsapp-bridge/` (ADR-008). It is
//! installed next to `messaging-mcp` as `messaging-mcp-whatsapp-bridge`.

use std::path::{Path, PathBuf};

use serde::Deserialize;

pub const BRIDGE_BINARY: &str = "messaging-mcp-whatsapp-bridge";
pub const DEFAULT_PORT: u16 = 8080;

/// The bridge binary installed next to `exe`.
pub fn bridge_binary(exe: &Path) -> Option<PathBuf> {
    exe.parent().map(|dir| dir.join(BRIDGE_BINARY))
}

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
    /// The device session and the token, kept by the bridge (0700).
    pub fn store(&self) -> PathBuf {
        self.root.join("store")
    }
    pub fn token(&self) -> PathBuf {
        self.store().join(".bridge-token")
    }
    /// What the bridge prints: connection events, never message contents.
    pub fn log(&self) -> PathBuf {
        self.root.join("bridge.log")
    }
    /// The supervisor's process ID.
    pub fn pid(&self) -> PathBuf {
        self.root.join("bridge.pid")
    }
}

/// `GET /api/health`.
#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
pub struct Health {
    #[serde(default)]
    pub connected: bool,
    #[serde(default)]
    pub paired: bool,
    /// The paired account, such as `5511999999999:12@s.whatsapp.net`.
    #[serde(default)]
    pub jid: String,
}

/// `GET /api/qr`: the pairing code to show, empty once paired.
#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
pub struct Pairing {
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub paired: bool,
    #[serde(default)]
    pub connected: bool,
}

/// The paired account's number from a device JID such as `5511999999999:12@s.whatsapp.net`.
pub fn phone_from_jid(jid: &str) -> Option<String> {
    let user = jid.split('@').next()?.split(':').next()?;
    let valid = (8..=15).contains(&user.len()) && user.bytes().all(|b| b.is_ascii_digit());
    valid.then(|| format!("+{user}"))
}

/// The port in a `http://127.0.0.1:PORT/api` URL.
pub fn port_from_base_url(base_url: &str) -> Option<u16> {
    url::Url::parse(base_url).ok()?.port()
}
