#![allow(clippy::unwrap_used)]

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use common::config_with;
use grammers_session::Session;
use grammers_tl_types as tl;
use messaging_mcp::adapters::telegram::{
    StoredSession, UserFacts, chat_contact, contacts_from_found, read_credentials, user_contact,
};
use messaging_mcp::config::TelegramPeer;
use messaging_mcp::identity::identify_sender;
use messaging_mcp::plan::Channel;
use serde_json::json;

const NO_ENV: &(dyn Fn(&str) -> Option<String> + Send + Sync) = &|_| None;
const HASH: &str = "0123456789abcdef0123456789abcdef";

fn secret(dir: &Path, name: &str, value: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, value).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    path
}

fn live(dir: &Path) -> messaging_mcp::config::Config {
    config_with(
        dir,
        json!({
            "mode": "live",
            "whatsapp": { "accountLabel": "Me", "tokenFile": secret(dir, "token", "a-bridge-token-0123") },
            "telegram": {
                "apiId": 12345,
                "expectedUserId": "777",
                "apiHashFile": secret(dir, "hash", HASH),
                "sessionFile": secret(dir, "session", "{}"),
            },
        }),
    )
}

#[test]
fn dry_run_identity_reads_no_secret() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_with(
        dir.path(),
        json!({ "whatsapp": { "tokenFile": "/missing" } }),
    );
    let sender = identify_sender(&config, Channel::Whatsapp, NO_ENV).unwrap();
    assert_eq!(sender.fingerprint, "dry-run");
}

#[test]
fn live_identity_binds_the_credentials_without_exposing_them() {
    let dir = tempfile::tempdir().unwrap();
    let config = live(dir.path());
    let whatsapp = identify_sender(&config, Channel::Whatsapp, NO_ENV).unwrap();
    let telegram = identify_sender(&config, Channel::Telegram, NO_ENV).unwrap();
    assert_eq!(whatsapp.label, "WhatsApp: Me (operator-verified)");
    assert_eq!(telegram.label, "Telegram user:777");
    assert!(!whatsapp.fingerprint.contains("a-bridge-token"));
    assert_eq!(whatsapp.fingerprint.len(), 64);

    secret(dir.path(), "token", "another-bridge-token-0123");
    let rotated = identify_sender(&config, Channel::Whatsapp, NO_ENV).unwrap();
    assert_ne!(rotated.fingerprint, whatsapp.fingerprint);
    let from_env: &(dyn Fn(&str) -> Option<String> + Send + Sync) =
        &|name| (name == "WHATSAPP_BRIDGE_TOKEN").then(|| "env-bridge-token-0123".to_string());
    let overridden = identify_sender(&config, Channel::Whatsapp, from_env).unwrap();
    assert_ne!(overridden.fingerprint, rotated.fingerprint);
}

#[test]
fn live_identity_requires_the_verified_account_settings() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_with(
        dir.path(),
        json!({ "mode": "live", "whatsapp": {}, "telegram": {} }),
    );
    for channel in [Channel::Whatsapp, Channel::Telegram] {
        let error = identify_sender(&config, channel, NO_ENV).unwrap_err();
        assert_eq!(error.code, "SENDER_UNCONFIGURED");
    }
    let mut missing = live(dir.path());
    missing.telegram.as_mut().unwrap().session_file = None;
    let error = identify_sender(&missing, Channel::Telegram, NO_ENV).unwrap_err();
    assert_eq!(error.code, "SECRET_UNAVAILABLE");
}

#[test]
fn telegram_credentials_are_checked_without_being_shown() {
    let dir = tempfile::tempdir().unwrap();
    let config = live(dir.path());
    let settings = config.telegram.as_ref().unwrap();
    assert_eq!(read_credentials(settings, NO_ENV).unwrap().api_id, 12345);
    let bad_hash: &(dyn Fn(&str) -> Option<String> + Send + Sync) =
        &|name| (name == "TELEGRAM_API_HASH").then(|| "not-hex".to_string());
    let error = read_credentials(settings, bad_hash).err().unwrap();
    assert_eq!(error.code, "TELEGRAM_CREDENTIALS_INVALID");
    assert!(!error.message.contains("not-hex"));
}

#[test]
fn the_session_file_keeps_authorization_keys_and_the_home_datacenter() {
    let mut option = grammers_session::SessionData::default().dc_options[&4].clone();
    option.auth_key = Some([9u8; 256]);
    let json = serde_json::to_string(&StoredSession {
        home_dc: 4,
        dc_options: vec![option],
    })
    .unwrap();
    let stored: StoredSession = serde_json::from_str(&json).unwrap();
    let session = stored.into_memory_session();
    assert_eq!(session.home_dc_id().unwrap(), 4);
    assert_eq!(
        session.dc_option(4).unwrap().unwrap().auth_key,
        Some([9u8; 256])
    );
    assert!(session.dc_option(2).unwrap().unwrap().auth_key.is_none());
    assert!(serde_json::from_str::<StoredSession>(r#"{"homeDc":2,"dcOptions":[],"x":1}"#).is_err());
}

fn person() -> UserFacts<'static> {
    UserFacts {
        id: 42,
        access_hash: Some(-7),
        first_name: Some("Ma\u{1b}ria"),
        last_name: Some("Silva"),
        username: Some("maria"),
        is_self: false,
        deleted: false,
        bot: false,
        min: false,
    }
}

#[test]
fn only_reachable_people_become_contacts() {
    let contact = user_contact(&person()).unwrap();
    assert_eq!(
        (contact.name.as_str(), contact.reference.as_str()),
        ("Maria Silva", "user:42")
    );
    assert_eq!(
        contact.peer,
        TelegramPeer::User {
            id: "42".into(),
            access_hash: "-7".into()
        }
    );
    for excluded in [
        UserFacts {
            is_self: true,
            ..person()
        },
        UserFacts {
            deleted: true,
            ..person()
        },
        UserFacts {
            bot: true,
            ..person()
        },
        UserFacts {
            min: true,
            ..person()
        },
        UserFacts {
            access_hash: None,
            ..person()
        },
    ] {
        assert!(user_contact(&excluded).is_none());
    }
    let unnamed = user_contact(&UserFacts {
        first_name: None,
        last_name: None,
        ..person()
    });
    assert_eq!(unnamed.unwrap().name, "Telegram user 42");
}

fn group(id: i64, left: bool) -> tl::enums::Chat {
    tl::types::Chat {
        creator: false,
        left,
        deactivated: false,
        call_active: false,
        call_not_empty: false,
        noforwards: false,
        id,
        title: "Family".to_string(),
        photo: tl::enums::ChatPhoto::Empty,
        participants_count: 3,
        date: 0,
        version: 1,
        migrated_to: None,
        admin_rights: None,
        default_banned_rights: None,
    }
    .into()
}

#[test]
fn search_results_come_only_from_the_owners_own_chats() {
    assert!(chat_contact(&group(9, true)).is_none());
    let found = tl::types::contacts::Found {
        my_results: vec![tl::types::PeerChat { chat_id: 9 }.into()],
        results: vec![tl::types::PeerChat { chat_id: 10 }.into()],
        chats: vec![group(9, false), group(10, false)],
        users: vec![],
    };
    let contacts = contacts_from_found(&found);
    assert_eq!(contacts.len(), 1);
    assert_eq!(contacts[0].peer, TelegramPeer::Chat { id: "9".into() });
}
