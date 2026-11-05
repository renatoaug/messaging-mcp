#![allow(clippy::unwrap_used)]

mod common;

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};

use common::recipients;
use messaging_mcp::config::{parse_config, validate_bridge_url};
use messaging_mcp::plan::Mode;
use messaging_mcp::secrets::read_secret_file;
use messaging_mcp::store::PlanStore;
use serde_json::json;

#[test]
fn config_defaults_closed_and_validates_exact_recipients() {
    let dir = tempfile::tempdir().unwrap();
    let config = parse_config("{}", dir.path()).unwrap();
    assert_eq!(config.mode, Mode::DryRun);
    assert!(config.recipients.is_empty());
    let whatsapp = recipients()[0].clone();
    let telegram = recipients()[1].clone();
    let mut phone = whatsapp.clone();
    phone["target"] = json!("+55 11 99999-9999");
    let mut username = telegram.clone();
    username["target"] = json!({ "kind": "username", "username": "someone" });
    let mut extra = whatsapp.clone();
    extra["note"] = json!("unknown field");
    for input in [
        json!({ "mode": "true" }),
        json!({ "unknown": true }),
        json!({ "recipients": [whatsapp.clone(), whatsapp] }),
        json!({ "recipients": [phone] }),
        json!({ "recipients": [username] }),
        json!({ "recipients": [extra] }),
        json!({ "planTtlSeconds": 5 }),
    ] {
        let error = parse_config(&input.to_string(), dir.path()).unwrap_err();
        assert_eq!(error.code, "CONFIG_INVALID", "{input}");
    }
}

#[test]
fn config_errors_never_quote_the_input() {
    let dir = tempfile::tempdir().unwrap();
    let error = parse_config(r#"{"mode": "secret-value"}"#, dir.path()).unwrap_err();
    assert!(!error.message.contains("secret-value"));
}

#[test]
fn bridge_url_rejects_remote_hosts_dns_credentials_and_injection() {
    for url in [
        "https://example.com/api",
        "http://localhost:8080/api",
        "http://127.0.0.1:8080/evil",
        "http://user:pass@127.0.0.1:8080/api",
        "http://127.0.0.1:8080/api?redirect=evil",
        "http://127.0.0.1/api",
        "file:///tmp/api",
    ] {
        assert_eq!(
            validate_bridge_url(url).unwrap_err().code,
            "CONFIG_INVALID",
            "{url}"
        );
    }
    assert_eq!(
        validate_bridge_url("http://[::1]:8080/api/").unwrap(),
        "http://[::1]:8080/api"
    );
}

#[test]
fn secret_files_must_be_private_regular_files_and_errors_hide_contents() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("token");
    fs::write(&path, "test-secret\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(read_secret_file(&path).unwrap(), "test-secret");
    let linked = dir.path().join("linked");
    symlink(&path, &linked).unwrap();
    assert_eq!(
        read_secret_file(&linked).unwrap_err().code,
        "SECRET_UNAVAILABLE"
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    let error = read_secret_file(&path).unwrap_err();
    assert_eq!(error.code, "SECRET_UNAVAILABLE");
    assert!(!error.message.contains("test-secret"));
}

#[test]
fn the_state_store_rejects_insecure_directories() {
    let dir = tempfile::tempdir().unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        PlanStore::open(dir.path()).err().unwrap().code,
        "STATE_PERMISSIONS"
    );
}
