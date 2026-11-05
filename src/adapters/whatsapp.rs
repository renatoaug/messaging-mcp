use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::{Client, Response, StatusCode, header, redirect};
use serde_json::{Value, json};

use crate::config::{Recipient, WhatsAppConfig, validate_bridge_url};
use crate::error::{DeliveryError, Error, Outcome};
use crate::identity::WHATSAPP_TOKEN_ENV;
use crate::plan::{ChannelAdapter, Receipt};
use crate::secrets::{process_env, secret_from};

const MAX_RESPONSE_BYTES: usize = 16_384;
const MIN_TOKEN_CHARS: usize = 16;

/// Only these statuses are guaranteed to reject before the bridge's send handler runs.
const REJECTED: [StatusCode; 5] = [
    StatusCode::BAD_REQUEST,
    StatusCode::UNAUTHORIZED,
    StatusCode::FORBIDDEN,
    StatusCode::NOT_FOUND,
    StatusCode::METHOD_NOT_ALLOWED,
];

pub type ReadToken = Arc<dyn Fn() -> Result<String, Error> + Send + Sync>;

fn uncertain(message: &str) -> DeliveryError {
    DeliveryError::new("WHATSAPP_OUTCOME_UNKNOWN", message, Outcome::Unknown)
}

/// Sends through the separately managed Go bridge, over loopback HTTP, with a single attempt.
pub struct WhatsAppAdapter {
    client: Client,
    send_url: String,
    read_token: ReadToken,
}

impl WhatsAppAdapter {
    pub fn new(config: WhatsAppConfig) -> Result<Self, Error> {
        let token_file = config.token_file.clone();
        let read_token: ReadToken =
            Arc::new(move || secret_from(&process_env, WHATSAPP_TOKEN_ENV, token_file.as_deref()));
        Self::with_token(config, read_token)
    }

    pub fn with_token(config: WhatsAppConfig, read_token: ReadToken) -> Result<Self, Error> {
        let base_url = validate_bridge_url(&config.base_url)?;
        let client = Client::builder()
            .redirect(redirect::Policy::none())
            // A proxy variable in the environment must never receive the bridge token.
            .no_proxy()
            .timeout(Duration::from_millis(config.timeout_ms))
            .build()
            .map_err(|_| Error::new("WHATSAPP_CLIENT_FAILED", "Could not create the client."))?;
        Ok(Self {
            client,
            send_url: format!("{base_url}/send"),
            read_token,
        })
    }

    fn token(&self) -> Result<String, DeliveryError> {
        (self.read_token)()
            .ok()
            .filter(|token| token.len() >= MIN_TOKEN_CHARS && !token.contains(['\r', '\n']))
            .ok_or_else(|| {
                DeliveryError::new(
                    "WHATSAPP_TOKEN_UNAVAILABLE",
                    "Configure a private WhatsApp bridge token. No request was sent.",
                    Outcome::NotSent,
                )
            })
    }
}

/// Reads a small response body, with the request timeout still running.
async fn read_limited(mut response: Response) -> Option<Vec<u8>> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    Some(body)
}

fn rejected_or_uncertain(status: StatusCode) -> DeliveryError {
    if REJECTED.contains(&status) {
        DeliveryError::new(
            "WHATSAPP_REJECTED",
            format!(
                "Bridge rejected the request (HTTP {}). No automatic retry.",
                status.as_u16()
            ),
            Outcome::NotSent,
        )
    } else {
        uncertain(&format!(
            "Bridge returned HTTP {}. Delivery is uncertain; verify the chat.",
            status.as_u16()
        ))
    }
}

#[async_trait]
impl ChannelAdapter for WhatsAppAdapter {
    async fn send(
        &self,
        recipient: &Recipient,
        text: &str,
        _plan_id: &str,
    ) -> Result<Receipt, DeliveryError> {
        let Recipient::Whatsapp { target, .. } = recipient else {
            return Err(DeliveryError::new(
                "CHANNEL_MISMATCH",
                "Wrong adapter for recipient.",
                Outcome::NotSent,
            ));
        };
        let token = self.token()?;
        let response = self
            .client
            .post(&self.send_url)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(json!({ "recipient": target, "message": text }).to_string())
            .send()
            .await
            .map_err(|_| {
                uncertain(
                    "WhatsApp request timed out or failed. It may have been sent. Verify the \
                     chat; do not retry automatically.",
                )
            })?;
        let status = response.status();
        if !status.is_success() {
            return Err(rejected_or_uncertain(status));
        }
        let body = read_limited(response)
            .await
            .and_then(|body| serde_json::from_slice::<Value>(&body).ok())
            .ok_or_else(|| {
                uncertain("Unexpected bridge response. Verify the chat before any new message.")
            })?;
        if body.get("success") != Some(&Value::Bool(true)) {
            return Err(uncertain(
                "Bridge did not confirm success. Verify the chat; no retry.",
            ));
        }
        // The bridge returns neither a message ID nor a delivery receipt.
        Ok(Receipt {
            accepted: true,
            provider_message_id: None,
        })
    }
}
