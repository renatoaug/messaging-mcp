//! `messaging-mcp approval`: the owner chooses how live messages are approved. Turning on
//! `client-prompt` also makes Claude Code (an "ask" rule) and Codex (`approval_mode = "prompt"`)
//! ask before `send_message`, so an auto-approving mode cannot skip the prompt. Run by the owner (or the installer, after asking them),
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

/// Codex's user config, when Codex is installed (its folder exists).
pub fn codex_config(env: Env<'_>) -> Option<PathBuf> {
    let dir = Path::new(&env("HOME")?).join(".codex");
    dir.is_dir().then(|| dir.join("config.toml"))
}

fn codex_error() -> Error {
    Error::new(
        "CODEX_CONFIG_UNREADABLE",
        "Codex's config.toml could not be read as TOML. Nothing was changed there.",
    )
}

/// Makes Codex prompt before `send_message`, keeping the rest of its config and comments.
/// Only when messaging-mcp is registered there. Returns whether it changed anything.
pub fn set_codex_prompt(config: &Path) -> Result<bool, Error> {
    let Ok(text) = std::fs::read_to_string(config) else {
        return Ok(false);
    };
    let mut document: toml_edit::DocumentMut = text.parse().map_err(|_| codex_error())?;
    let Some(server) = document
        .get_mut("mcp_servers")
        .and_then(|servers| servers.get_mut("messaging-mcp"))
        .and_then(toml_edit::Item::as_table_like_mut)
    else {
        return Ok(false);
    };
    // An implicit table: only [mcp_servers.messaging-mcp.tools.send_message] is written.
    let mut implicit = toml_edit::Table::new();
    implicit.set_implicit(true);
    let tools = server
        .entry("tools")
        .or_insert(toml_edit::Item::Table(implicit))
        .as_table_like_mut()
        .ok_or_else(codex_error)?;
    let send = tools
        .entry("send_message")
        .or_insert(toml_edit::table())
        .as_table_like_mut()
        .ok_or_else(codex_error)?;
    if send.get("approval_mode").and_then(toml_edit::Item::as_str) == Some("prompt") {
        return Ok(false);
    }
    send.insert("approval_mode", toml_edit::value("prompt"));
    write_private_file(config, document.to_string().as_bytes())?;
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
        (Approval::Terminal | Approval::Dialog, _) => json!("unchanged"),
    };
    let codex = match (approval, codex_config(env)) {
        (Approval::ClientPrompt, Some(path)) => json!({
            "config": path,
            "promptAdded": set_codex_prompt(&path)?,
        }),
        (Approval::ClientPrompt, None) => json!("Codex not found; nothing to change there."),
        (Approval::Terminal | Approval::Dialog, _) => json!("unchanged"),
    };
    let mut clients = json!({ "claudeCode": claude, "codex": codex });
    if approval == Approval::ClientPrompt {
        // The prompt is the yes only while a person answers it.
        clients["warning"] = json!(
            "Answer these prompts yourself: in Codex, keep 'Ask for approval' (not 'Approve for \
             me'), and never choose 'always allow' for send_message in either client. An \
             automatic reviewer or an always-allow would send without asking you."
        );
    }
    Ok((config, clients))
}
