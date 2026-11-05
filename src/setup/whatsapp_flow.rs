//! The WhatsApp setup flow: build, start, wait for the pairing scan, record the account. It
//! drives `git`, Go, the bridge process and the owner's phone, so the offline suite leaves it out
//! of the coverage floor; the first live test exercises it.

use std::time::Duration;

use serde_json::{Value, json};
use tokio::task::JoinHandle;

use super::config_file::{section, update_config};
use super::whatsapp_bridge::phone_from_jid;
use super::{Setup, WAIT, WHATSAPP_PAGE, bridge_process, setup_error};
use crate::config::Config;
use crate::error::Error;
use crate::secrets::read_secret_file;

impl Setup {
    /// Builds the bridge when needed, starts it, and waits up to 25 seconds for a QR code or a
    /// connection.
    pub async fn whatsapp_setup(&self, config: &Config) -> Result<Value, Error> {
        let bridge = self.bridge();
        if !bridge.binary().exists() {
            let mut build = self.build.lock().await;
            if build.as_ref().is_none_or(JoinHandle::is_finished) {
                let paths = bridge.clone();
                let previous = build.take();
                if let Some(previous) = previous
                    && let Ok(Err(error)) = previous.await
                {
                    return Err(error);
                }
                if !bridge.binary().exists() {
                    *build = Some(tokio::task::spawn_blocking(move || {
                        bridge_process::build(&paths)
                    }));
                }
            }
            let started = tokio::time::Instant::now();
            while build.as_ref().is_some_and(|job| !job.is_finished()) && started.elapsed() < WAIT {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            if let Some(job) = build.as_ref().filter(|job| !job.is_finished()) {
                let _ = job;
                return Ok(json!({
                    "status": "building",
                    "log": bridge.build_log(),
                    "next": "Building the bridge takes a few minutes the first time. Call \
                             whatsapp_setup again.",
                }));
            }
            if let Some(job) = build.take() {
                job.await
                    .map_err(|_| setup_error("WHATSAPP_BRIDGE_FAILED", "The build stopped."))??;
            }
        }
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
        self.whatsapp_wait(port).await
    }

    pub(super) async fn whatsapp_wait(&self, port: u16) -> Result<Value, Error> {
        let bridge = self.bridge();
        let page = self.downloads.join(WHATSAPP_PAGE);
        let started = tokio::time::Instant::now();
        loop {
            let health = match read_secret_file(&bridge.token()) {
                Ok(token) => bridge_process::health(port, &token).await,
                Err(_) => None,
            };
            if let Some(health) = health.filter(|health| health.connected && health.paired) {
                let _ = health;
                let html = super::qr::status_page(
                    super::qr::Tone::Success,
                    "You're connected",
                    "WhatsApp is linked to Messaging MCP. You can close this tab.",
                );
                let _ = super::config_file::write_private_file(&page, html.as_bytes());
                return self.whatsapp_connected();
            }
            if page.exists() && started.elapsed() > Duration::from_secs(2) {
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
            if started.elapsed() > WAIT {
                return Ok(json!({
                    "status": "starting",
                    "log": bridge.log(),
                    "next": "Call whatsapp_status again.",
                }));
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    /// Records which account is paired, read from the bridge's own device table.
    fn whatsapp_connected(&self) -> Result<Value, Error> {
        let account = bridge_process::device_jid(&self.bridge())
            .as_deref()
            .and_then(phone_from_jid);
        if let Some(account) = &account {
            update_config(&self.config_path, |config| {
                let whatsapp = section(config, "whatsapp");
                whatsapp.insert("accountLabel".into(), json!(account));
            })?;
        }
        Ok(json!({ "status": "connected", "account": account }))
    }
}
