use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use url::Url;

use crate::error::Error;
use crate::plan::{Channel, Mode};
use crate::secrets::{Env, process_env};

pub const CONFIG_ENV: &str = "MESSAGING_MCP_CONFIG";
pub const CONFIG_FILE_NAME: &str = "messaging-mcp.config.json";
const MAX_RECIPIENTS: usize = 1000;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum TelegramPeer {
    #[default]
    #[serde(rename = "self")]
    Myself,
    User {
        id: String,
        #[serde(rename = "accessHash")]
        access_hash: String,
    },
    Chat {
        id: String,
    },
    Channel {
        id: String,
        #[serde(rename = "accessHash")]
        access_hash: String,
    },
}

impl TelegramPeer {
    fn is_valid(&self) -> bool {
        match self {
            Self::Myself => true,
            Self::Chat { id } => is_positive_id(id),
            Self::User { id, access_hash } | Self::Channel { id, access_hash } => {
                is_positive_id(id) && access_hash.parse::<i64>().is_ok()
            }
        }
    }

    pub fn display(&self) -> String {
        match self {
            Self::Myself => "Saved Messages (self)".to_string(),
            Self::User { id, .. } => format!("user:{id}"),
            Self::Chat { id } => format!("chat:{id}"),
            Self::Channel { id, .. } => format!("channel:{id}"),
        }
    }
}

/// A destination the owner authorized. Explicit identifiers avoid guessing from a display name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "channel", rename_all = "lowercase", deny_unknown_fields)]
pub enum Recipient {
    Whatsapp {
        id: String,
        label: String,
        target: String,
    },
    Telegram {
        id: String,
        label: String,
        target: TelegramPeer,
    },
}

impl Recipient {
    pub fn id(&self) -> &str {
        match self {
            Self::Whatsapp { id, .. } | Self::Telegram { id, .. } => id,
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Whatsapp { label, .. } | Self::Telegram { label, .. } => label,
        }
    }

    pub fn channel(&self) -> Channel {
        match self {
            Self::Whatsapp { .. } => Channel::Whatsapp,
            Self::Telegram { .. } => Channel::Telegram,
        }
    }

    pub fn display_target(&self) -> String {
        match self {
            Self::Whatsapp { target, .. } => target.clone(),
            Self::Telegram { target, .. } => target.display(),
        }
    }

    /// Checks the rules JSON parsing cannot express. Used for the config and for `allow`.
    pub fn validate(&self) -> Result<(), Error> {
        let label_chars = self.label().chars().count();
        if !is_recipient_id(self.id()) || label_chars == 0 || label_chars > 100 {
            return Err(Error::new(
                "CONFIG_INVALID",
                "A recipient needs an ID of lowercase letters, digits, _ or - and a 1-100 \
                 character label.",
            ));
        }
        let target_is_valid = match self {
            Self::Whatsapp { target, .. } => is_whatsapp_jid(target),
            Self::Telegram { target, .. } => target.is_valid(),
        };
        if target_is_valid {
            Ok(())
        } else {
            Err(Error::new(
                "CONFIG_INVALID",
                format!("Recipient {} has an invalid target.", self.id()),
            ))
        }
    }
}

fn is_digits(value: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_positive_id(value: &str) -> bool {
    !value.starts_with('0') && is_digits(value, 1, 19) && value.parse::<i64>().is_ok()
}

fn is_recipient_id(value: &str) -> bool {
    let mut bytes = value.bytes();
    let first_is_valid = bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit());
    first_is_valid
        && value.len() <= 64
        && bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
        })
}

fn is_whatsapp_jid(value: &str) -> bool {
    if let Some(number) = value.strip_suffix("@s.whatsapp.net") {
        return !number.starts_with('0') && is_digits(number, 8, 15);
    }
    let Some(group) = value.strip_suffix("@g.us") else {
        return false;
    };
    match group.split_once('-') {
        Some((creator, created)) => is_digits(creator, 5, 30) && is_digits(created, 5, 20),
        None => is_digits(group, 5, 30),
    }
}

fn default_state_dir() -> PathBuf {
    PathBuf::from(".messaging-mcp-state")
}

fn default_plan_ttl() -> i64 {
    600
}

/// Who approves a live message when the MCP client cannot show an approval dialog.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Approval {
    /// Only `messaging-mcp approve` in a terminal.
    #[default]
    Terminal,
    /// The client's own permission prompt for `send_message`, which shows the destination and
    /// the whole message because the call has to repeat them. The owner must keep that prompt
    /// on (an "ask" rule); an "always allow" or an automatic reviewer turns it off.
    ClientPrompt,
    /// MCP's approval dialog (elicitation), for clients that actually show it. Codex declares it
    /// but declined it without showing anything on 2026-11-05, so it is never assumed.
    Dialog,
}

fn default_max_sends() -> i64 {
    6
}

fn default_bridge_url() -> String {
    "http://127.0.0.1:8080/api".to_string()
}

fn default_whatsapp_timeout() -> u64 {
    15_000
}

fn default_telegram_timeout() -> u64 {
    20_000
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WhatsAppConfig {
    pub account_label: Option<String>,
    #[serde(default = "default_bridge_url")]
    pub base_url: String,
    pub token_file: Option<PathBuf>,
    #[serde(default = "default_whatsapp_timeout")]
    pub timeout_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TelegramConfig {
    pub expected_user_id: Option<String>,
    pub api_id: Option<i32>,
    pub api_hash_file: Option<PathBuf>,
    pub session_file: Option<PathBuf>,
    #[serde(default = "default_telegram_timeout")]
    pub timeout_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub mode: Mode,
    #[serde(default = "default_state_dir")]
    pub state_dir: PathBuf,
    #[serde(default = "default_plan_ttl")]
    pub plan_ttl_seconds: i64,
    #[serde(default = "default_max_sends")]
    pub max_sends_per_minute: i64,
    /// Who approves a live message when the client has no approval dialog.
    #[serde(default)]
    pub approval: Approval,
    #[serde(default)]
    pub recipients: Vec<Recipient>,
    pub whatsapp: Option<WhatsAppConfig>,
    pub telegram: Option<TelegramConfig>,
}

/// Accepts only a literal loopback address, so a bridge token can never travel elsewhere.
///
/// No DNS names (`localhost` can be overridden), credentials or other paths.
pub fn validate_bridge_url(value: &str) -> Result<String, Error> {
    let error = || {
        Error::new(
            "CONFIG_INVALID",
            "WhatsApp baseUrl must be http://127.0.0.1:PORT/api or http://[::1]:PORT/api.",
        )
    };
    let url = Url::parse(value).map_err(|_| error())?;
    let host_is_loopback = matches!(url.host_str(), Some("127.0.0.1" | "[::1]"));
    let path_is_api = matches!(url.path(), "/api" | "/api/");
    if url.scheme() != "http"
        || !host_is_loopback
        || url.port().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !path_is_api
    {
        return Err(error());
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

fn check_range(value: i64, min: i64, max: i64, field: &str) -> Result<(), Error> {
    if (min..=max).contains(&value) {
        Ok(())
    } else {
        Err(Error::new(
            "CONFIG_INVALID",
            format!("{field} must be between {min} and {max}."),
        ))
    }
}

fn check_timeout(value: u64, field: &str) -> Result<(), Error> {
    check_range(
        i64::try_from(value).unwrap_or(i64::MAX),
        100,
        120_000,
        field,
    )
}

fn check_optional_label(label: Option<&String>) -> Result<(), Error> {
    match label.map(|value| value.chars().count()) {
        Some(0) => Err(Error::new(
            "CONFIG_INVALID",
            "whatsapp.accountLabel cannot be empty.",
        )),
        Some(count) if count > 100 => Err(Error::new(
            "CONFIG_INVALID",
            "whatsapp.accountLabel is over 100 characters.",
        )),
        _ => Ok(()),
    }
}

fn check_recipients(recipients: &[Recipient]) -> Result<(), Error> {
    if recipients.len() > MAX_RECIPIENTS {
        return Err(Error::new("CONFIG_INVALID", "Too many recipients."));
    }
    for (index, recipient) in recipients.iter().enumerate() {
        recipient.validate()?;
        if recipients[..index]
            .iter()
            .any(|other| other.id() == recipient.id())
        {
            return Err(Error::new(
                "CONFIG_INVALID",
                "Recipient IDs must be unique.",
            ));
        }
    }
    Ok(())
}

fn absolute(path: &Path, base: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

/// Parses a config. Parse errors report only the position: they could quote a private value.
pub fn parse_config(input: &str, base_dir: &Path) -> Result<Config, Error> {
    let mut config: Config = serde_json::from_str(input).map_err(|error| {
        Error::new(
            "CONFIG_INVALID",
            format!(
                "Invalid configuration at line {}, column {}.",
                error.line(),
                error.column()
            ),
        )
    })?;
    check_range(config.plan_ttl_seconds, 30, 86_400, "planTtlSeconds")?;
    check_range(config.max_sends_per_minute, 1, 60, "maxSendsPerMinute")?;
    check_recipients(&config.recipients)?;
    config.state_dir = absolute(&config.state_dir, base_dir);
    if let Some(whatsapp) = &mut config.whatsapp {
        check_optional_label(whatsapp.account_label.as_ref())?;
        check_timeout(whatsapp.timeout_ms, "whatsapp.timeoutMs")?;
        whatsapp.base_url = validate_bridge_url(&whatsapp.base_url)?;
        whatsapp.token_file = whatsapp
            .token_file
            .as_deref()
            .map(|file| absolute(file, base_dir));
    }
    if let Some(telegram) = &mut config.telegram {
        check_timeout(telegram.timeout_ms, "telegram.timeoutMs")?;
        if telegram
            .expected_user_id
            .as_deref()
            .is_some_and(|id| !is_positive_id(id))
        {
            return Err(Error::new(
                "CONFIG_INVALID",
                "telegram.expectedUserId must be numeric.",
            ));
        }
        telegram.api_hash_file = telegram
            .api_hash_file
            .as_deref()
            .map(|file| absolute(file, base_dir));
        telegram.session_file = telegram
            .session_file
            .as_deref()
            .map(|file| absolute(file, base_dir));
    }
    Ok(config)
}

/// Where the config lives without `--config` or `MESSAGING_MCP_CONFIG`:
/// `$XDG_CONFIG_HOME/messaging-mcp/`, else `~/.config/messaging-mcp/`.
pub fn default_config_dir(env: Env<'_>) -> Option<PathBuf> {
    env("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env("HOME")
                .filter(|home| !home.is_empty())
                .map(|home| Path::new(&home).join(".config"))
        })
        .map(|base| base.join("messaging-mcp"))
}

/// The config file in use: `path`, then `MESSAGING_MCP_CONFIG`, then the default location.
pub fn resolve_config_path(path: Option<&Path>, env: Env<'_>) -> Result<PathBuf, Error> {
    if let Some(path) = path {
        return Ok(path.to_path_buf());
    }
    if let Some(path) = env(CONFIG_ENV).filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    default_config_dir(env)
        .map(|dir| dir.join(CONFIG_FILE_NAME))
        .ok_or_else(|| Error::new("CONFIG_READ_FAILED", format!("Set {CONFIG_ENV} or HOME.")))
}

/// Loads a config file.
pub fn load_config(path: Option<&Path>) -> Result<Config, Error> {
    load_config_from(&resolve_config_path(path, &process_env)?)
}

pub fn load_config_from(path: &Path) -> Result<Config, Error> {
    let read_error = || {
        Error::new(
            "CONFIG_READ_FAILED",
            format!("Cannot read the configuration. Set {CONFIG_ENV} to the config file path."),
        )
    };
    let input = fs::read_to_string(path).map_err(|_| read_error())?;
    let absolute_path = std::path::absolute(path).map_err(|_| read_error())?;
    let base_dir = absolute_path.parent().ok_or_else(read_error)?;
    parse_config(&input, base_dir)
}
