#![allow(clippy::unwrap_used)]

//! Live sends are approved in the MCP client's own dialog. These tests run the real binary with
//! a client that answers the dialog, and with one that cannot show it. The Telegram session is
//! deliberately unusable, so an approved plan fails before any network call.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, Mutex};

use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, ClientConfig, ElicitRequestParams, ElicitResult,
    ElicitationAction, ElicitationCapability, FormElicitationCapability, Implementation,
};
use rmcp::service::{RequestContext, RunningService};
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use rmcp::{ClientHandler, ErrorData, RoleClient, ServiceExt};
use serde_json::{Value, json};

const BINARY: &str = env!("CARGO_BIN_EXE_messaging-mcp");
const TEXT: &str = "Oi!\nSegunda linha com **asteriscos**.";

/// A client that answers every approval dialog with `answer` and remembers what it was shown.
#[derive(Clone)]
struct Approver {
    answer: Option<bool>,
    shown: Arc<Mutex<Vec<String>>>,
}

impl ClientHandler for Approver {
    fn get_info(&self) -> ClientConfig {
        let mut capabilities = ClientCapabilities::default();
        if self.answer.is_some() {
            let mut elicitation = ElicitationCapability::default();
            elicitation.form = Some(FormElicitationCapability::new());
            capabilities.elicitation = Some(elicitation);
        }
        ClientConfig::new(capabilities, Implementation::new("approval-test", "1.0.0"))
    }

    async fn create_elicitation(
        &self,
        request: ElicitRequestParams,
        _context: RequestContext<RoleClient>,
    ) -> Result<ElicitResult, ErrorData> {
        if let ElicitRequestParams::FormElicitationParams { message, .. } = request {
            self.shown.lock().unwrap().push(message);
        }
        Ok(match self.answer {
            Some(true) => {
                ElicitResult::new(ElicitationAction::Accept).with_content(json!({ "send": true }))
            }
            _ => ElicitResult::new(ElicitationAction::Decline),
        })
    }
}

fn private_file(path: &Path, contents: &str) {
    fs::write(path, contents).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

/// A live config with a Telegram account whose session is not a real one.
fn live_config_with(dir: &Path, approval: &str) -> std::path::PathBuf {
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).unwrap();
    private_file(&dir.join("hash"), "0123456789abcdef0123456789abcdef");
    private_file(&dir.join("session.json"), "{}");
    let config = dir.join("config.json");
    let input = json!({
        "mode": "live",
        "approval": approval,
        "stateDir": dir.join("state"),
        "recipients": [{ "id": "me", "label": "Saved Messages", "channel": "telegram",
                         "target": { "kind": "self" } }],
        "telegram": { "apiId": 1, "apiHashFile": dir.join("hash"),
                      "sessionFile": dir.join("session.json"), "expectedUserId": "777" },
    });
    fs::write(&config, input.to_string()).unwrap();
    config
}

async fn connect(dir: &Path, approver: Approver) -> RunningService<RoleClient, Approver> {
    connect_with(dir, approver, "dialog").await
}

async fn connect_with(
    dir: &Path,
    approver: Approver,
    approval: &str,
) -> RunningService<RoleClient, Approver> {
    let config = live_config_with(dir, approval);
    let home = dir.to_path_buf();
    let kept: Vec<(String, String)> = ["PATH", "LLVM_PROFILE_FILE"]
        .iter()
        .filter_map(|name| {
            std::env::var(name)
                .ok()
                .map(|value| (name.to_string(), value))
        })
        .collect();
    let transport = TokioChildProcess::new(tokio::process::Command::new(BINARY).configure(|cmd| {
        cmd.env_clear()
            .env("HOME", &home)
            .envs(kept)
            .arg("mcp")
            .arg("--config")
            .arg(&config)
            .current_dir(&home);
    }))
    .unwrap();
    approver.serve(transport).await.unwrap()
}

fn arguments(value: Value) -> serde_json::Map<String, Value> {
    value.as_object().cloned().unwrap()
}

/// Prepares the message and asks to send it; returns (send result text, is error).
async fn prepare_and_send(client: &RunningService<RoleClient, Approver>) -> (Value, bool) {
    let prepared = client
        .call_tool(
            CallToolRequestParams::new("prepare_message").with_arguments(arguments(
                json!({ "recipientId": "me", "text": TEXT, "idempotencyKey": "approval_test_1" }),
            )),
        )
        .await
        .unwrap();
    let plan: Value = serde_json::from_str(&prepared.content[0].as_text().unwrap().text).unwrap();
    let sent = client
        .call_tool(
            CallToolRequestParams::new("send_message").with_arguments(arguments(
                json!({ "planId": plan["id"], "recipientId": "me", "text": TEXT }),
            )),
        )
        .await
        .unwrap();
    let value = serde_json::from_str(&sent.content[0].as_text().unwrap().text).unwrap();
    (value, sent.is_error == Some(true))
}

#[tokio::test]
async fn an_approved_message_goes_to_the_adapter_after_showing_the_whole_text() {
    let dir = tempfile::tempdir().unwrap();
    let shown = Arc::new(Mutex::new(Vec::new()));
    let client = connect(
        dir.path(),
        Approver {
            answer: Some(true),
            shown: shown.clone(),
        },
    )
    .await;
    let (result, is_error) = prepare_and_send(&client).await;
    // Approved, then refused by the unusable session before any network call.
    assert!(!is_error, "{result}");
    assert_eq!(result["status"], "failed");
    assert_eq!(result["error"]["code"], "TELEGRAM_UNAVAILABLE");
    let messages = shown.lock().unwrap().clone();
    assert_eq!(messages.len(), 1);
    assert!(messages[0].contains(TEXT) && messages[0].contains("Telegram user:777"));
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn a_declined_message_is_not_sent_and_stays_pending() {
    let dir = tempfile::tempdir().unwrap();
    let shown = Arc::new(Mutex::new(Vec::new()));
    let client = connect(
        dir.path(),
        Approver {
            answer: Some(false),
            shown: shown.clone(),
        },
    )
    .await;
    let (result, is_error) = prepare_and_send(&client).await;
    assert!(is_error);
    assert_eq!(result["code"], "APPROVAL_DECLINED");
    assert_eq!(shown.lock().unwrap().len(), 1);
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn with_client_prompt_a_client_that_declares_the_dialog_is_not_asked_twice() {
    // Codex declares the dialog but declines it unseen: the permission prompt is the approval.
    let dir = tempfile::tempdir().unwrap();
    let shown = Arc::new(Mutex::new(Vec::new()));
    let approver = Approver {
        answer: Some(false),
        shown: shown.clone(),
    };
    let client = connect_with(dir.path(), approver, "client-prompt").await;
    let (result, is_error) = prepare_and_send(&client).await;
    assert!(!is_error, "{result}");
    assert_eq!(result["error"]["code"], "TELEGRAM_UNAVAILABLE");
    assert!(shown.lock().unwrap().is_empty());
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn in_terminal_mode_nothing_is_sent_from_the_client() {
    let dir = tempfile::tempdir().unwrap();
    let shown = Arc::new(Mutex::new(Vec::new()));
    let approver = Approver {
        answer: Some(true),
        shown: shown.clone(),
    };
    let client = connect_with(dir.path(), approver, "terminal").await;
    let (result, is_error) = prepare_and_send(&client).await;
    assert!(is_error);
    assert_eq!(result["code"], "APPROVAL_REQUIRED");
    assert!(shown.lock().unwrap().is_empty());
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn a_client_without_a_dialog_cannot_send_live() {
    let dir = tempfile::tempdir().unwrap();
    let shown = Arc::new(Mutex::new(Vec::new()));
    let client = connect(
        dir.path(),
        Approver {
            answer: None,
            shown: shown.clone(),
        },
    )
    .await;
    let (result, is_error) = prepare_and_send(&client).await;
    assert!(is_error);
    assert_eq!(result["code"], "APPROVAL_REQUIRED");
    assert!(shown.lock().unwrap().is_empty());
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn with_client_prompt_approval_the_permission_prompt_is_the_approval() {
    let dir = tempfile::tempdir().unwrap();
    let shown = Arc::new(Mutex::new(Vec::new()));
    let approver = Approver {
        answer: None,
        shown: shown.clone(),
    };
    let client = connect_with(dir.path(), approver, "client-prompt").await;
    let (result, is_error) = prepare_and_send(&client).await;
    // Approved by the call itself, then refused by the unusable session before any network call.
    assert!(!is_error, "{result}");
    assert_eq!(result["error"]["code"], "TELEGRAM_UNAVAILABLE");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn a_send_that_does_not_repeat_the_prepared_message_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let shown = Arc::new(Mutex::new(Vec::new()));
    let approver = Approver {
        answer: Some(true),
        shown: shown.clone(),
    };
    let client = connect_with(dir.path(), approver, "client-prompt").await;
    let prepared = client
        .call_tool(
            CallToolRequestParams::new("prepare_message").with_arguments(arguments(
                json!({ "recipientId": "me", "text": TEXT, "idempotencyKey": "mismatch_test" }),
            )),
        )
        .await
        .unwrap();
    let plan: Value = serde_json::from_str(&prepared.content[0].as_text().unwrap().text).unwrap();
    let sent = client
        .call_tool(
            CallToolRequestParams::new("send_message").with_arguments(arguments(
                json!({ "planId": plan["id"], "recipientId": "me", "text": "something else" }),
            )),
        )
        .await
        .unwrap();
    assert_eq!(sent.is_error, Some(true));
    assert!(
        sent.content[0]
            .as_text()
            .unwrap()
            .text
            .contains("SEND_MISMATCH")
    );
    assert!(shown.lock().unwrap().is_empty());
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn in_agent_mode_the_call_sends_without_any_dialog() {
    let dir = tempfile::tempdir().unwrap();
    let shown = Arc::new(Mutex::new(Vec::new()));
    let approver = Approver {
        answer: Some(false),
        shown: shown.clone(),
    };
    let client = connect_with(dir.path(), approver, "agent").await;
    let (result, is_error) = prepare_and_send(&client).await;
    // Approved by the call, then refused by the unusable session before any network call.
    assert!(!is_error, "{result}");
    assert_eq!(result["error"]["code"], "TELEGRAM_UNAVAILABLE");
    assert!(shown.lock().unwrap().is_empty());
    client.cancel().await.unwrap();
}
