//! The WhatsApp setup flow: start the bridge, wait for the pairing scan, record the account. It
//! drives the bridge process and the owner's phone, so the offline suite leaves it out of the
//! coverage floor; the live test exercises it.

use std::time::Duration;

use serde_json::{Value, json};

use super::config_file::{section, update_config};
use super::whatsapp_bridge::{BRIDGE_BINARY, bridge_binary, phone_from_jid};
use super::{Setup, WAIT, WHATSAPP_PAGE, bridge_process, setup_error};
use crate::config::Config;
use crate::error::Error;
use crate::secrets::read_secret_file;

impl Setup {
    /// Starts the bridge, and waits up to 25 seconds for a QR code or a connection.
    pub async fn whatsapp_setup(&self, config: &Config) -> Result<Value, Error> {
        let exe = std::env::current_exe()
            .map_err(|_| setup_error("WHATSAPP_BRIDGE_FAILED", "Could not find messaging-mcp."))?;
        let installed = bridge_binary(&exe).is_some_and(|binary| binary.is_file());
        if !installed {
            return Err(setup_error(
                "WHATSAPP_BRIDGE_MISSING",
                format!(
                    "{BRIDGE_BINARY} is not installed next to messaging-mcp. Install it from the \
                     release archive, or build it from the repository: `cd whatsapp-bridge && \
                     go build -o {} .`",
                    exe.parent()
                        .map(|dir| dir.join(BRIDGE_BINARY).display().to_string())
                        .unwrap_or_default()
                ),
            ));
        }
        let bridge = self.bridge();
        let port = Self::whatsapp_port(config);
        let token = bridge.token();
        update_config(&self.config_path, |config| {
            let whatsapp = section(config, "whatsapp");
            whatsapp.insert(
                "baseUrl".into(),
                json!(format!("http://127.0.0.1:{port}/api")),
            );
            whatsapp.insert("tokenFile".into(), json!(token));
        })?;
        bridge_process::start(
            &bridge,
            &self.config_path,
            &self.downloads.join(WHATSAPP_PAGE),
        )?;
        self.whatsapp_wait(port, false).await
    }

    /// Waits for the bridge to connect. With `for_scan`, a visible QR page does not end the wait:
    /// the call waits the whole 25 seconds for the owner to scan it.
    pub(super) async fn whatsapp_wait(&self, port: u16, for_scan: bool) -> Result<Value, Error> {
        let bridge = self.bridge();
        let page = self.downloads.join(WHATSAPP_PAGE);
        let started = tokio::time::Instant::now();
        loop {
            let health = match read_secret_file(&bridge.token()) {
                Ok(token) => bridge_process::health(port, &token).await,
                Err(_) => None,
            };
            if let Some(health) = health.filter(|health| health.connected && health.paired) {
                return self.whatsapp_connected(&health.jid);
            }
            if bridge_process::running_pid(&bridge).is_none() && started.elapsed() > WAIT / 5 {
                return Err(setup_error(
                    "WHATSAPP_BRIDGE_STOPPED",
                    format!(
                        "The bridge stopped. Its log is {}. Call whatsapp_setup to start it again.",
                        bridge.log().display()
                    ),
                ));
            }
            let waiting = page.exists() && started.elapsed() > Duration::from_secs(2);
            if waiting && (!for_scan || started.elapsed() > WAIT) {
                return Ok(json!({
                    "status": "waiting_for_scan",
                    "page": page,
                    "next": format!(
                        "Open the page for the user (on macOS: `open {}`) and ask them to scan \
                         it: WhatsApp > Settings > Linked devices > Link a device. Then call \
                         whatsapp_status to wait for the pairing.",
                        page.display()
                    ),
                }));
            }
            if started.elapsed() > WAIT && !waiting {
                return Ok(json!({
                    "status": "starting",
                    "log": bridge.log(),
                    "next": "Call whatsapp_status again.",
                }));
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    /// Starts the bridge again when it was paired before and is not running, for example after
    /// the machine restarted. It reconnects without a QR code. A bridge that never finished
    /// pairing is left alone, so no QR page appears unasked.
    pub fn resume_whatsapp(&self, config: &Config) {
        let paired_before = config
            .whatsapp
            .as_ref()
            .is_some_and(|whatsapp| whatsapp.account_label.is_some());
        let bridge = self.bridge();
        let session = bridge.store().join("session.db");
        let installed = std::env::current_exe()
            .ok()
            .and_then(|exe| bridge_binary(&exe))
            .is_some_and(|binary| binary.is_file());
        if paired_before
            && installed
            && session.is_file()
            && bridge_process::running_pid(&bridge).is_none()
        {
            let page = self.downloads.join(WHATSAPP_PAGE);
            let _ = bridge_process::start(&bridge, &self.config_path, &page);
        }
    }

    /// Records which account is paired, as the bridge reports it.
    fn whatsapp_connected(&self, jid: &str) -> Result<Value, Error> {
        let account = phone_from_jid(jid);
        if let Some(account) = &account {
            update_config(&self.config_path, |config| {
                let whatsapp = section(config, "whatsapp");
                whatsapp.insert("accountLabel".into(), json!(account));
            })?;
        }
        Ok(json!({ "status": "connected", "account": account }))
    }
}
