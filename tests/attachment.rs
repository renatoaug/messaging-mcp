//! Files sent with a message: which paths are refused, the reviewed copy, and the send.

#![allow(clippy::unwrap_used)]

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use common::*;
use messaging_mcp::attachment::{Kind, MAX_FILE_BYTES, copy_path, files_dir as attachment_dir};
use messaging_mcp::plan::Status;
use messaging_mcp::service::{PrepareInput, Swept};
use serde_json::json;
use tempfile::TempDir;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n rest of a picture";

/// A folder outside the fixture (whose folder is the config folder, which is protected) and with
/// a name that is not hidden, as `tempdir()`'s `.tmp` names are.
fn files() -> TempDir {
    tempfile::Builder::new().prefix("files").tempdir().unwrap()
}

fn write(dir: &Path, name: &str, data: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, data).unwrap();
    path
}

fn with_file(key: &str, path: &Path, caption: &str) -> PrepareInput {
    PrepareInput {
        text: caption.to_string(),
        file_path: Some(path.to_string_lossy().into_owned()),
        ..draft(key)
    }
}

fn live() -> serde_json::Value {
    json!({ "mode": "live" })
}

#[tokio::test]
async fn a_file_is_reviewed_copied_and_sent_as_reviewed() {
    let adapter = FakeAdapter::accepting();
    let f = fixture(live(), whatsapp(adapter.clone()));
    let dir = files();
    let path = write(dir.path(), "report.pdf", b"%PDF-1.7 first version");
    let plan = f.service.prepare(&with_file(KEY, &path, "")).unwrap();
    let attachment = plan.attachment.clone().unwrap();
    assert_eq!(attachment.name, "report.pdf");
    assert_eq!(attachment.mime, "application/pdf");
    assert_eq!(attachment.kind, Kind::Document);
    assert_eq!(attachment.size, 22);
    assert_eq!(
        attachment.sha256,
        messaging_mcp::service::sha256_hex(b"%PDF-1.7 first version")
    );
    let copy = copy_path(&f.config.state_dir, &plan.id);
    let mode = std::fs::metadata(&copy).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);

    // The original changes after the review: the reviewed copy is what leaves.
    std::fs::write(&path, b"%PDF-1.7 changed").unwrap();
    f.service.approve(&plan.id).unwrap();
    let sent = f.service.send(&plan.id).await.unwrap();
    assert_eq!(sent.status, Status::Sent);
    let files = adapter.files.lock().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(
        files[0],
        ("report.pdf".to_string(), b"%PDF-1.7 first version".to_vec())
    );
    assert!(
        !copy.exists(),
        "the copy is removed once the plan was attempted"
    );
    assert_eq!(
        f.service.status(&plan.id).unwrap().attachment,
        Some(attachment)
    );
}

#[tokio::test]
async fn a_changed_copy_is_never_sent() {
    let adapter = FakeAdapter::accepting();
    let f = fixture(live(), whatsapp(adapter.clone()));
    let dir = files();
    let path = write(dir.path(), "photo.png", PNG);
    let plan = f.service.prepare(&with_file(KEY, &path, "look")).unwrap();
    assert_eq!(plan.attachment.as_ref().unwrap().kind, Kind::Image);
    std::fs::write(copy_path(&f.config.state_dir, &plan.id), b"something else").unwrap();
    f.service.approve(&plan.id).unwrap();
    let sent = f.service.send(&plan.id).await.unwrap();
    assert_eq!(sent.status, Status::Failed);
    assert_eq!(sent.error.unwrap().code, "FILE_CHANGED");
    assert_eq!(adapter.count(), 0);
}

#[test]
fn private_and_unusual_paths_are_refused() {
    let f = fixture(json!({}), whatsapp(FakeAdapter::accepting()));
    let dir = files();
    let hidden = dir.path().join(".secrets");
    std::fs::create_dir(&hidden).unwrap();
    let in_hidden = write(&hidden, "key.txt", b"secret");
    let dotfile = write(dir.path(), ".env", b"TOKEN=1");
    let link = dir.path().join("innocent.txt");
    std::os::unix::fs::symlink(&in_hidden, &link).unwrap();
    let in_config = write(f.dir.path(), "notes.txt", b"config folder");
    let empty = write(dir.path(), "empty.txt", b"");
    let big = dir.path().join("big.bin");
    std::fs::File::create(&big)
        .unwrap()
        .set_len(MAX_FILE_BYTES + 1)
        .unwrap();
    let cases = [
        (in_hidden.clone(), "FILE_NOT_ALLOWED"),
        (dotfile, "FILE_NOT_ALLOWED"),
        (link, "FILE_NOT_ALLOWED"),
        (in_config, "FILE_NOT_ALLOWED"),
        (empty, "FILE_NOT_ALLOWED"),
        (big, "FILE_NOT_ALLOWED"),
        (dir.path().to_path_buf(), "FILE_UNREADABLE"),
        (dir.path().join("missing.pdf"), "FILE_UNREADABLE"),
        (PathBuf::from("relative.pdf"), "FILE_NOT_ALLOWED"),
    ];
    for (index, (path, code)) in cases.iter().enumerate() {
        let error = f
            .service
            .prepare(&with_file(&format!("refused_{index:04}"), path, ""))
            .unwrap_err();
        assert_eq!(error.code, *code, "{}", path.display());
    }
    assert!(!f.config.state_dir.join("files").exists());
}

#[test]
fn a_caption_is_optional_and_shorter_than_a_message() {
    let f = fixture(json!({}), whatsapp(FakeAdapter::accepting()));
    let dir = files();
    let path = write(dir.path(), "a.txt", b"hello");
    assert!(
        f.service
            .prepare(&with_file("caption_0001", &path, ""))
            .is_ok()
    );
    let long = "x".repeat(1025);
    let error = f
        .service
        .prepare(&with_file("caption_0002", &path, &long))
        .unwrap_err();
    assert_eq!(error.code, "INVALID_MESSAGE");
}

#[test]
fn a_mislabeled_picture_goes_as_a_document() {
    let f = fixture(json!({}), whatsapp(FakeAdapter::accepting()));
    let dir = files();
    let path = write(dir.path(), "fake.png", b"not a picture");
    let plan = f.service.prepare(&with_file(KEY, &path, "")).unwrap();
    let attachment = plan.attachment.unwrap();
    assert_eq!(attachment.kind, Kind::Document);
    assert_eq!(attachment.mime, "image/png");
}

#[test]
fn a_key_reused_for_another_file_keeps_the_first_copy() {
    let f = fixture(json!({}), whatsapp(FakeAdapter::accepting()));
    let dir = files();
    let first = write(dir.path(), "first.txt", b"first");
    let second = write(dir.path(), "second.txt", b"second");
    let plan = f.service.prepare(&with_file(KEY, &first, "")).unwrap();
    let error = f.service.prepare(&with_file(KEY, &second, "")).unwrap_err();
    assert_eq!(error.code, "IDEMPOTENCY_CONFLICT");
    let copy = std::fs::read(copy_path(&f.config.state_dir, &plan.id)).unwrap();
    assert_eq!(copy, b"first");
}

#[tokio::test]
async fn a_dry_run_with_a_file_simulates_and_cleans_up() {
    let adapter = FakeAdapter::accepting();
    let f = fixture(json!({}), whatsapp(adapter.clone()));
    let dir = files();
    let path = write(dir.path(), "a.txt", b"hello");
    let plan = f.service.prepare(&with_file(KEY, &path, "")).unwrap();
    let sent = f.service.send(&plan.id).await.unwrap();
    assert_eq!(sent.status, Status::Simulated);
    assert_eq!(adapter.count(), 0);
    assert!(!copy_path(&f.config.state_dir, &plan.id).exists());
}

#[test]
fn the_sweep_removes_old_plans_and_copies_that_can_never_be_sent() {
    let clock = TestClock::new(1_000_000_000_000);
    let mut f = fixture(
        json!({ "planRetentionDays": 2 }),
        whatsapp(FakeAdapter::accepting()),
    );
    f.service = f.service.with_clock(clock.clock());
    let dir = files();
    let path = write(dir.path(), "a.txt", b"hello");
    let old = f
        .service
        .prepare(&with_file("sweep_old_01", &path, ""))
        .unwrap();
    let state = f.config.state_dir.clone();
    std::fs::write(
        attachment_dir(&state).join("msg_00000000000000000000000000000000"),
        b"x",
    )
    .unwrap();
    std::fs::write(attachment_dir(&state).join(".leftover.tmp"), b"x").unwrap();

    // Still sendable: its copy stays; the orphan and the leftover go.
    assert_eq!(
        f.service.sweep().unwrap(),
        Swept {
            plans: 0,
            copies: 2
        }
    );
    assert!(copy_path(&state, &old.id).exists());

    // Expired after 10 minutes: its copy goes, the plan stays until the retention.
    clock.set(1_000_000_000_000 + 11 * 60_000);
    let fresh = f
        .service
        .prepare(&with_file("sweep_new_01", &path, ""))
        .unwrap();
    assert!(!copy_path(&state, &old.id).exists());
    assert!(copy_path(&state, &fresh.id).exists());
    assert!(f.service.status(&old.id).is_ok());

    // After the retention, the old plan and its text are deleted.
    clock.set(1_000_000_000_000 + 2 * 86_400_000 + 1);
    assert_eq!(f.service.sweep().unwrap().plans, 1);
    assert!(f.service.status(&fresh.id).is_ok());
    assert_eq!(
        f.service.status(&old.id).unwrap_err().code,
        "PLAN_NOT_FOUND"
    );
}
