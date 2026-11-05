#![allow(clippy::unwrap_used)]

//! The real binary, over stdio MCP and as a CLI. No account details, credentials or real
//! destinations, and an environment with nothing inherited.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::transport::{ConfigureCommandExt, TokioChildProcess};
use serde_json::{Value, json};
use tempfile::TempDir;

const BINARY: &str = env!("CARGO_BIN_EXE_messaging-mcp");

struct Fixture {
    dir: TempDir,
    config: PathBuf,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.json");
    let input = json!({
        "mode": "dry-run",
        "stateDir": dir.path().join("state"),
        "recipients": [
            { "id": "wa-fixture", "label": "Offline WhatsApp fixture", "channel": "whatsapp",
              "target": "15555550123@s.whatsapp.net" },
            { "id": "tg-fixture", "label": "Offline Telegram fixture", "channel": "telegram",
              "target": { "kind": "self" } },
        ],
    });
    std::fs::write(&config, input.to_string()).unwrap();
    Fixture { dir, config }
}

/// Keeps only what the binary needs, plus the coverage profile path when measuring coverage.
fn kept_env() -> Vec<(String, String)> {
    ["PATH", "LLVM_PROFILE_FILE"]
        .iter()
        .filter_map(|name| {
            std::env::var(name)
                .ok()
                .map(|value| (name.to_string(), value))
        })
        .collect()
}

fn isolated(command: &mut Command, home: &Path) {
    command.env_clear().env("HOME", home).envs(kept_env());
}

impl Fixture {
    fn run(&self, args: &[&str]) -> Output {
        let mut command = Command::new(BINARY);
        isolated(&mut command, self.dir.path());
        command
            .args(args)
            .arg("--config")
            .arg(&self.config)
            .current_dir(self.dir.path());
        command.stdin(std::process::Stdio::null()).output().unwrap()
    }

    fn json(&self, args: &[&str]) -> Value {
        let output = self.run(args);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{args:?}: {stderr}");
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

fn tool_value(result: CallToolResult) -> Value {
    assert_ne!(result.is_error, Some(true), "{result:?}");
    assert_eq!(result.content.len(), 1);
    let text = result.content[0].as_text().unwrap();
    serde_json::from_str(&text.text).unwrap()
}

#[tokio::test]
async fn an_mcp_client_lists_only_safe_tools_and_persists_both_dry_run_channels() {
    let f = fixture();
    let (home, config) = (f.dir.path().to_path_buf(), f.config.clone());
    let transport = TokioChildProcess::new(tokio::process::Command::new(BINARY).configure(|cmd| {
        cmd.env_clear()
            .env("HOME", &home)
            .envs(kept_env())
            .arg("mcp")
            .arg("--config")
            .arg(&config)
            .current_dir(&home);
    }))
    .unwrap();
    let client = ().serve(transport).await.unwrap();
    assert_eq!(
        client
            .peer_info()
            .unwrap()
            .server_info
            .as_ref()
            .unwrap()
            .name,
        "messaging-mcp"
    );
    let mut names: Vec<String> = client
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "get_message_status",
            "get_status",
            "list_recipients",
            "prepare_message",
            "search_contacts",
            "send_message"
        ]
    );
    assert!(
        names
            .iter()
            .all(|name| !name.contains("approve") && !name.contains("allow"))
    );

    let call = |name: &'static str, arguments: Value| {
        let client = &client;
        async move {
            let params = CallToolRequestParams::new(name)
                .with_arguments(arguments.as_object().cloned().unwrap_or_default());
            tool_value(client.call_tool(params).await.unwrap())
        }
    };
    let status = call("get_status", json!({})).await;
    assert_eq!(status["mode"], "dry-run");
    assert_eq!(status["accountConnectionTested"], false);
    assert_eq!(
        status["channelsConfigured"],
        json!({ "whatsapp": false, "telegram": false })
    );
    assert_eq!(
        call("list_recipients", json!({}))
            .await
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let search = client
        .call_tool(
            CallToolRequestParams::new("search_contacts")
                .with_arguments(json!({ "query": "Maria" }).as_object().cloned().unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(search.is_error, Some(true));

    let mut completed = Vec::new();
    for recipient_id in ["wa-fixture", "tg-fixture"] {
        let args = json!({
            "recipientId": recipient_id,
            "text": "Offline only: café\n**literal**",
            "idempotencyKey": format!("mcp_test_{recipient_id}"),
        });
        let plan = call("prepare_message", args.clone()).await;
        assert_eq!(
            (plan["status"].clone(), plan["mode"].clone()),
            (json!("pending"), json!("dry-run"))
        );
        assert_eq!(plan["text"], args["text"]);
        assert_eq!(call("prepare_message", args).await["id"], plan["id"]);
        let id = json!({ "planId": plan["id"] });
        let sent = call("send_message", id.clone()).await;
        assert_eq!(sent["status"], "simulated");
        assert_eq!(sent.get("receipt"), None);
        assert_eq!(call("send_message", id.clone()).await, sent);
        assert_eq!(call("get_message_status", id).await, sent);
        completed.push(sent);
    }
    client.cancel().await.unwrap();

    // New processes prove the durable state survives the MCP shutdown.
    for sent in completed {
        let id = sent["id"].as_str().unwrap();
        assert_eq!(f.json(&["status", id]), sent);
        assert_eq!(f.json(&["send", id]), sent);
    }
}

#[test]
fn the_cli_works_offline_across_restarts() {
    let f = fixture();
    let status = f.json(&["doctor"]);
    assert_eq!(status["check"], "local_configuration_only");
    assert_eq!(status["accountConnectionTested"], false);
    assert_eq!(status["allowedRecipients"], 2);
    assert_eq!(f.json(&["recipients"]).as_array().unwrap().len(), 2);
    let text_file = f.dir.path().join("message.txt");
    let text = "Offline CLI fixture\nUnicode: olá 👋";
    std::fs::write(&text_file, text).unwrap();
    let file = text_file.to_str().unwrap();
    let prepare = [
        "prepare",
        "--recipient",
        "tg-fixture",
        "--text-file",
        file,
        "--key",
        "cli_file_test",
    ];
    let plan = f.json(&prepare);
    assert_eq!(
        (plan["text"].as_str(), plan["status"].as_str()),
        (Some(text), Some("pending"))
    );
    let id = plan["id"].as_str().unwrap();
    assert_eq!(f.json(&["status", id]), plan);
    let sent = f.json(&["send", id]);
    assert_eq!(sent["status"], "simulated");
    assert_eq!(f.json(&["status", id]), sent);
    assert_eq!(f.json(&["send", id]), sent);
    assert_eq!(f.json(&prepare)["id"], plan["id"]);
}

fn structured_error(output: &Output) -> Value {
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    let lines: Vec<Value> = stderr
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 1);
    lines[0].clone()
}

#[test]
fn approval_and_allow_refuse_a_pipe_and_change_nothing() {
    let f = fixture();
    let prepare = [
        "prepare",
        "--recipient",
        "wa-fixture",
        "--text",
        "Never through a pipe",
        "--key",
        "cli_approval_test",
    ];
    let plan = f.json(&prepare);
    let id = plan["id"].as_str().unwrap();
    assert_eq!(
        structured_error(&f.run(&["approve", id]))["code"],
        "TTY_REQUIRED"
    );
    assert_eq!(f.json(&["status", id]), plan);
    let allow = [
        "allow",
        "whatsapp",
        "5511977776666@s.whatsapp.net",
        "--id",
        "ana",
        "--label",
        "Ana",
    ];
    assert_eq!(structured_error(&f.run(&allow))["code"], "TTY_REQUIRED");
    assert_eq!(
        structured_error(&f.run(&["telegram-login"]))["code"],
        "TTY_REQUIRED"
    );
    assert_eq!(f.json(&["recipients"]).as_array().unwrap().len(), 2);
}
