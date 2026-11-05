//! `messaging-mcp approval`: the owner chooses how live messages are approved. Turning on
//! `client-prompt` also adds Claude Code's "ask" rule for `send_message`, so the prompt cannot be
//! skipped by an auto-approving mode. Run by the owner (or the installer, after asking them),
//! never by an MCP tool.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::config::{Approval, Config};
use crate::error::Error;
use crate::secrets::Env;
use crate::setup::config_file::{section, update_config, write_private_file};

/// The rule that makes Claude Code ask before every send.
pub const CLAUDE_ASK_RULE: &str = "mcp__messaging-mcp__send_message";

/// Claude Code's user settings, when Claude Code is installed (its folder exists).
pub fn claude_settings(env: Env<'_>) -> Option<PathBuf> {
    let dir = Path::new(&env("HOME")?).join(".claude");
    dir.is_dir().then(|| dir.join("settings.json"))
}

fn settings_error() -> Error {
    Error::new(
        "CLAUDE_SETTINGS_UNREADABLE",
        "Claude Code's settings.json is not valid JSON. Nothing was changed there.",
    )
}

/// Adds the ask rule to Claude Code's settings, keeping everything else. Returns whether it added
/// it (false when it was already there).
pub fn add_claude_ask_rule(settings: &Path) -> Result<bool, Error> {
    let mut root: Value = match std::fs::read_to_string(settings) {
        Ok(text) => serde_json::from_str(&text).map_err(|_| settings_error())?,
        Err(_) => json!({}),
    };
    let object = root.as_object_mut().ok_or_else(settings_error)?;
    let ask = section(object, "permissions")
        .entry("ask")
        .or_insert_with(|| json!([]));
    let list = ask.as_array_mut().ok_or_else(settings_error)?;
    if list.iter().any(|rule| rule == CLAUDE_ASK_RULE) {
        return Ok(false);
    }
    list.push(json!(CLAUDE_ASK_RULE));
    let mut text = serde_json::to_string_pretty(&root).map_err(|_| settings_error())?;
    text.push('\n');
    write_private_file(settings, text.as_bytes())?;
    Ok(true)
}

/// Sets the approval mode in the config and, for `client-prompt`, Claude Code's ask rule.
pub fn set_approval(
    config_path: &Path,
    approval: Approval,
    env: Env<'_>,
) -> Result<(Config, Value), Error> {
    let config = update_config(config_path, |config: &mut Map<String, Value>| {
        config.insert("approval".into(), json!(approval));
    })?;
    let claude = match (approval, claude_settings(env)) {
        (Approval::ClientPrompt, Some(settings)) => json!({
            "settings": settings,
            "askRuleAdded": add_claude_ask_rule(&settings)?,
        }),
        (Approval::ClientPrompt, None) => json!("Claude Code not found; nothing to change there."),
        (Approval::Terminal, _) => json!("unchanged"),
    };
    Ok((config, claude))
}
