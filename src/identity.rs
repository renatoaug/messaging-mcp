use std::sync::Arc;

use serde_json::json;

use crate::config::Config;
use crate::error::Error;
use crate::plan::{Channel, Mode};
use crate::secrets::{Env, secret_from};
use crate::service::sha256_hex;

pub const WHATSAPP_TOKEN_ENV: &str = "WHATSAPP_BRIDGE_TOKEN";
pub const TELEGRAM_API_ID_ENV: &str = "TELEGRAM_API_ID";
pub const TELEGRAM_API_HASH_ENV: &str = "TELEGRAM_API_HASH";
pub const TELEGRAM_SESSION_ENV: &str = "TELEGRAM_SESSION";

/// Who a plan would send as. The fingerprint binds a plan to the credentials without exposing them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SenderIdentity {
    pub label: String,
    pub fingerprint: String,
}

pub type IdentifySender = Arc<dyn Fn(Channel) -> Result<SenderIdentity, Error> + Send + Sync>;

fn unconfigured(message: &str) -> Error {
    Error::new("SENDER_UNCONFIGURED", message)
}

pub fn identify_sender(
    config: &Config,
    channel: Channel,
    env: Env<'_>,
) -> Result<SenderIdentity, Error> {
    if config.mode == Mode::DryRun {
        return Ok(SenderIdentity {
            label: "Simulator (no account)".to_string(),
            fingerprint: "dry-run".to_string(),
        });
    }
    match channel {
        Channel::Whatsapp => identify_whatsapp(config, env),
        Channel::Telegram => identify_telegram(config, env),
    }
}

fn identify_whatsapp(config: &Config, env: Env<'_>) -> Result<SenderIdentity, Error> {
    let settings = config.whatsapp.as_ref();
    let account_label = settings.and_then(|settings| settings.account_label.as_deref());
    let (Some(settings), Some(account_label)) = (settings, account_label) else {
        return Err(unconfigured(
            "Live WhatsApp requires accountLabel and a bridge token for the manually verified \
             account.",
        ));
    };
    let token = secret_from(env, WHATSAPP_TOKEN_ENV, settings.token_file.as_deref())?;
    let material = json!({
        "channel": "whatsapp",
        "baseUrl": settings.base_url,
        "label": account_label,
        "token": token,
    });
    Ok(SenderIdentity {
        label: format!("WhatsApp: {account_label} (operator-verified)"),
        fingerprint: sha256_hex(material.to_string().as_bytes()),
    })
}

fn identify_telegram(config: &Config, env: Env<'_>) -> Result<SenderIdentity, Error> {
    let settings = config.telegram.as_ref();
    let expected = settings.and_then(|settings| settings.expected_user_id.as_deref());
    let (Some(settings), Some(expected)) = (settings, expected) else {
        return Err(unconfigured(
            "Live Telegram requires expectedUserId for the authorized personal account.",
        ));
    };
    let api_id = env(TELEGRAM_API_ID_ENV).or(settings.api_id.map(|id| id.to_string()));
    let material = json!({
        "channel": "telegram",
        "expectedUserId": expected,
        "apiId": api_id,
        "apiHash": secret_from(env, TELEGRAM_API_HASH_ENV, settings.api_hash_file.as_deref())?,
        "session": secret_from(env, TELEGRAM_SESSION_ENV, settings.session_file.as_deref())?,
    });
    Ok(SenderIdentity {
        label: format!("Telegram user:{expected}"),
        fingerprint: sha256_hex(material.to_string().as_bytes()),
    })
}
