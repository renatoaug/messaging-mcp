#![allow(clippy::unwrap_used)]

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

use messaging_mcp::config::{default_config_dir, load_config_from, resolve_config_path};
use messaging_mcp::plan::Mode;
use messaging_mcp::setup::Setup;
use messaging_mcp::setup::config_file::{ensure_config, update_config, write_private_file};
use messaging_mcp::setup::qr::{Tone, encode, page, status_page, svg};
use messaging_mcp::setup::telegram_qr::login_url;
use messaging_mcp::setup::whatsapp_bridge::{BridgePaths, phone_from_jid, port_from_base_url};
use serde_json::json;

type EnvFn = dyn Fn(&str) -> Option<String> + Send + Sync;

fn mode_of(path: &Path) -> u32 {
    fs::metadata(path).unwrap().mode() & 0o777
}

fn fresh_config() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config").join("messaging-mcp.config.json");
    assert!(ensure_config(&path).unwrap());
    (dir, path)
}

#[test]
fn the_default_config_is_private_dry_run_and_never_overwritten() {
    let (_dir, path) = fresh_config();
    assert_eq!(mode_of(&path), 0o600);
    assert_eq!(mode_of(path.parent().unwrap()), 0o700);
    let config = load_config_from(&path).unwrap();
    assert_eq!(config.mode, Mode::DryRun);
    assert_eq!(config.state_dir, path.parent().unwrap().join("state"));
    fs::write(&path, r#"{"mode":"live"}"#).unwrap();
    assert!(!ensure_config(&path).unwrap());
    assert_eq!(fs::read_to_string(&path).unwrap(), r#"{"mode":"live"}"#);
}

#[test]
fn the_config_path_comes_from_the_flag_the_variable_then_the_home_folder() {
    let env: &EnvFn = &|name| match name {
        "HOME" => Some("/home/ana".to_string()),
        _ => None,
    };
    let xdg: &EnvFn = &|name| match name {
        "XDG_CONFIG_HOME" => Some("/xdg".to_string()),
        "MESSAGING_MCP_CONFIG" => Some("/etc/mm.json".to_string()),
        _ => None,
    };
    assert_eq!(
        default_config_dir(env).unwrap(),
        Path::new("/home/ana/.config/messaging-mcp")
    );
    assert_eq!(
        resolve_config_path(None, env).unwrap(),
        Path::new("/home/ana/.config/messaging-mcp/messaging-mcp.config.json")
    );
    assert_eq!(
        default_config_dir(xdg).unwrap(),
        Path::new("/xdg/messaging-mcp")
    );
    assert_eq!(
        resolve_config_path(None, xdg).unwrap(),
        Path::new("/etc/mm.json")
    );
    assert_eq!(
        resolve_config_path(Some(Path::new("/x.json")), xdg).unwrap(),
        Path::new("/x.json")
    );
    assert!(resolve_config_path(None, &|_| None).is_err());
}

#[test]
fn an_invalid_edit_is_refused_and_leaves_the_file_as_it_was() {
    let (_dir, path) = fresh_config();
    let before = fs::read_to_string(&path).unwrap();
    let error = update_config(&path, |config| {
        config.insert("mode".into(), json!("everything"));
    })
    .unwrap_err();
    assert_eq!(error.code, "CONFIG_INVALID");
    assert_eq!(fs::read_to_string(&path).unwrap(), before);
    let config = update_config(&path, |config| {
        config.insert("maxSendsPerMinute".into(), json!(3));
    })
    .unwrap();
    assert_eq!(config.max_sends_per_minute, 3);
    assert_eq!(mode_of(&path), 0o600);
}

#[test]
fn private_files_replace_the_old_one_and_stay_owner_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secret");
    fs::write(&path, "old").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    write_private_file(&path, b"new").unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "new");
    assert_eq!(mode_of(&path), 0o600);
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}

fn setup_for(path: &Path, home: &Path) -> Setup {
    let home = home.to_string_lossy().to_string();
    let env: &EnvFn = &move |name| (name == "HOME").then(|| home.clone());
    Setup::new(path, env).unwrap()
}

#[tokio::test]
async fn telegram_credentials_go_to_a_private_file_never_the_config() {
    let (dir, path) = fresh_config();
    let setup = setup_for(&path, dir.path());
    let hash = "0123456789abcdef0123456789abcdef";
    for (id, bad) in [
        (0, hash),
        (1, "short"),
        (1, "zz23456789abcdef0123456789abcdef"),
    ] {
        let error = setup.set_telegram_credentials(id, bad).unwrap_err();
        assert_eq!(error.code, "TELEGRAM_CREDENTIALS_INVALID");
    }
    let config = setup.set_telegram_credentials(12345, hash).unwrap();
    let telegram = config.telegram.unwrap();
    assert_eq!(telegram.api_id, Some(12345));
    let hash_file = telegram.api_hash_file.unwrap();
    assert_eq!(fs::read_to_string(&hash_file).unwrap(), hash);
    assert_eq!(mode_of(&hash_file), 0o600);
    assert!(!fs::read_to_string(&path).unwrap().contains(hash));

    let status = setup.status(&load_config_from(&path).unwrap(), 0).await;
    assert_eq!(status["telegram"]["credentials"], true);
    assert_eq!(status["telegram"]["loggedIn"], false);
    assert!(
        status["next"][0]
            .as_str()
            .unwrap()
            .contains("telegram_login")
    );
}

#[tokio::test]
async fn status_lists_the_next_steps_and_the_mode_can_change() {
    let (dir, path) = fresh_config();
    let setup = setup_for(&path, dir.path());
    let status = setup.status(&load_config_from(&path).unwrap(), 0).await;
    assert_eq!(status["mode"], "dry-run");
    assert_eq!(status["whatsapp"]["bridgeRunning"], false);
    assert_eq!(status["liveSendsRequireTerminalApproval"], true);
    let next: Vec<&str> = status["next"]
        .as_array()
        .unwrap()
        .iter()
        .map(|step| step.as_str().unwrap())
        .collect();
    assert!(next[0].contains("my.telegram.org"));
    assert!(next[1].contains("whatsapp_setup"));
    assert_eq!(setup.set_mode(Mode::Live).unwrap().mode, Mode::Live);
    let error = setup
        .telegram_login(&load_config_from(&path).unwrap())
        .await
        .unwrap_err();
    assert_eq!(error.code, "TELEGRAM_UNCONFIGURED");
    let not_running = setup
        .whatsapp_status(&load_config_from(&path).unwrap())
        .await
        .unwrap();
    assert_eq!(not_running["status"], "not_running");
    assert_eq!(setup.whatsapp_stop()["stopped"], false);
}

#[test]
fn a_pairing_code_becomes_a_page_that_reloads() {
    let matrix = encode("2@abc,def,ghi").unwrap();
    let picture = svg(&matrix);
    assert!(picture.starts_with("<svg") && picture.contains("<rect x="));
    let html = page("Title", &["Open the app.", "Scan it."], &picture);
    assert!(
        html.contains("http-equiv=\"refresh\"") && html.contains("<li><span>Scan it.</span></li>")
    );
}

#[test]
fn bridge_details_are_read_without_guessing() {
    assert_eq!(
        phone_from_jid("5511999999999:12@s.whatsapp.net").as_deref(),
        Some("+5511999999999")
    );
    assert_eq!(
        phone_from_jid("5511999999999@s.whatsapp.net").as_deref(),
        Some("+5511999999999")
    );
    assert_eq!(phone_from_jid("abc@s.whatsapp.net"), None);
    assert_eq!(port_from_base_url("http://127.0.0.1:8123/api"), Some(8123));
    let paths = BridgePaths::new(Path::new("/c"));
    assert_eq!(
        paths.token(),
        Path::new("/c/whatsapp-bridge/store/.bridge-token")
    );
    assert_eq!(login_url(&[0xfb, 0xff]), "tg://login?token=-_8");
}

#[tokio::test]
async fn a_finished_login_is_reported_without_contacting_telegram() {
    let (dir, path) = fresh_config();
    let setup = setup_for(&path, dir.path());
    setup
        .set_telegram_credentials(12345, "0123456789abcdef0123456789abcdef")
        .unwrap();
    let session = path.parent().unwrap().join("telegram-session.json");
    write_private_file(&session, b"{}").unwrap();
    let config = update_config(&path, |config| {
        config["telegram"]["expectedUserId"] = json!("777");
    })
    .unwrap();
    let status = setup.telegram_login(&config).await.unwrap();
    assert_eq!(
        (status["status"].as_str(), status["userId"].as_str()),
        (Some("logged_in"), Some("777"))
    );
    let overview = setup.status(&config, 0).await;
    assert_eq!(overview["telegram"]["loggedIn"], true);
    assert!(
        overview["next"][0]
            .as_str()
            .unwrap()
            .contains("whatsapp_setup")
    );
    assert!(overview["next"][1].as_str().unwrap().contains("set_mode"));
}

#[test]
fn the_outcome_page_stops_reloading_and_says_what_happened() {
    let html = status_page(
        Tone::Success,
        "Telegram is connected",
        "You can close this tab.",
    );
    assert!(html.contains("Telegram is connected") && html.contains("You can close this tab."));
    assert!(html.contains("hero success") && html.contains("rel=\"icon\""));
    assert!(!html.contains("http-equiv=\"refresh\""));
    assert!(status_page(Tone::Failure, "Stopped", "Try again.").contains("hero failure"));
}

#[tokio::test]
async fn the_login_reads_the_config_file_not_the_copy_the_server_started_with() {
    let (dir, path) = fresh_config();
    let setup = setup_for(&path, dir.path());
    let stale = setup
        .set_telegram_credentials(12345, "0123456789abcdef0123456789abcdef")
        .unwrap();
    write_private_file(&path.parent().unwrap().join("telegram-session.json"), b"{}").unwrap();
    update_config(&path, |config| {
        config["telegram"]["expectedUserId"] = json!("777");
    })
    .unwrap();
    // `stale` has no user ID; the file does. The answer must come from the file.
    let status = setup.telegram_login(&stale).await.unwrap();
    assert_eq!(status["userId"], "777");
}
