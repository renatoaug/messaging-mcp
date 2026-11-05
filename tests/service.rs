#![allow(clippy::unwrap_used)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use common::*;
use messaging_mcp::config::{Recipient, TelegramPeer};
use messaging_mcp::error::{DeliveryError, Outcome};
use messaging_mcp::identity::SenderIdentity;
use messaging_mcp::plan::{Channel, ChannelAdapter, Contact, Mode, Receipt, Status};
use messaging_mcp::service::{Adapters, MessagingService, PrepareInput};
use messaging_mcp::store::PlanStore;
use serde_json::json;
use tokio::sync::{Mutex as AsyncMutex, oneshot};

#[tokio::test]
async fn dry_run_makes_no_adapter_calls_on_either_channel() {
    let adapter = FakeAdapter::new(|| panic!("must not run"));
    let adapters = Adapters {
        whatsapp: Some(adapter.clone()),
        telegram: Some(adapter.clone()),
    };
    let f = fixture(json!({}), adapters);
    for (index, recipient_id) in ["wa-test", "tg-self"].into_iter().enumerate() {
        let input = PrepareInput {
            recipient_id: recipient_id.to_string(),
            ..draft(&format!("intent_demo_{index}"))
        };
        let plan = f.service.prepare(&input).unwrap();
        assert_eq!(plan.mode, Mode::DryRun);
        assert_eq!(
            f.service.send(&plan.id).await.unwrap().status,
            Status::Simulated
        );
        assert_eq!(
            f.service.send(&plan.id).await.unwrap().status,
            Status::Simulated
        );
    }
    assert_eq!(adapter.count(), 0);
}

#[tokio::test]
async fn live_sends_require_local_approval_and_return_acceptance_not_delivery() {
    let adapter = FakeAdapter::accepting();
    let f = fixture(json!({ "mode": "live" }), whatsapp(adapter.clone()));
    let plan = f.service.prepare(&draft(KEY)).unwrap();
    assert_eq!(
        f.service.send(&plan.id).await.unwrap_err().code,
        "APPROVAL_REQUIRED"
    );
    assert_eq!(adapter.count(), 0);
    f.service.approve(&plan.id).unwrap();
    let sent = f.service.send(&plan.id).await.unwrap();
    assert_eq!(sent.status, Status::Sent);
    let calls = adapter.calls.lock().unwrap().clone();
    assert_eq!(
        (calls[0].1.as_str(), calls[0].2.as_str()),
        (plan.text.as_str(), plan.id.as_str())
    );
    assert!(
        f.service
            .send(&plan.id)
            .await
            .unwrap()
            .receipt
            .unwrap()
            .accepted
    );
    assert_eq!(adapter.count(), 1);
}

#[tokio::test]
async fn idempotency_survives_restart_and_rejects_reuse_with_different_content() {
    let f = fixture(json!({}), Adapters::default());
    let plan = f.service.prepare(&draft(KEY)).unwrap();
    f.service.send(&plan.id).await.unwrap();
    let other = other_client(&f.config, Adapters::default());
    assert_eq!(other.prepare(&draft(KEY)).unwrap().id, plan.id);
    assert_eq!(
        other.send(&plan.id).await.unwrap().status,
        Status::Simulated
    );
    let different = PrepareInput {
        text: "different".to_string(),
        ..draft(KEY)
    };
    assert_eq!(
        other.prepare(&different).unwrap_err().code,
        "IDEMPOTENCY_CONFLICT"
    );
}

/// Blocks inside `send` until the test releases it.
struct GatedAdapter {
    started: AsyncMutex<Option<oneshot::Sender<()>>>,
    release: AsyncMutex<Option<oneshot::Receiver<()>>>,
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl ChannelAdapter for GatedAdapter {
    async fn send(&self, _: &Recipient, _: &str, _: &str) -> Result<Receipt, DeliveryError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Some(started) = self.started.lock().await.take() {
            let _ = started.send(());
        }
        if let Some(release) = self.release.lock().await.take() {
            let _ = release.await;
        }
        Ok(Receipt {
            accepted: true,
            provider_message_id: None,
        })
    }
}

#[tokio::test]
async fn concurrent_clients_claim_a_plan_only_once() {
    let (started_tx, started_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let adapter = Arc::new(GatedAdapter {
        started: AsyncMutex::new(Some(started_tx)),
        release: AsyncMutex::new(Some(release_rx)),
        calls: Default::default(),
    });
    let adapters = Adapters {
        whatsapp: Some(adapter.clone()),
        telegram: None,
    };
    let f = fixture(json!({ "mode": "live" }), adapters.clone());
    let other = other_client(&f.config, adapters);
    let plan = f.service.prepare(&draft(KEY)).unwrap();
    f.service.approve(&plan.id).unwrap();
    let service = Arc::new(f.service);
    let in_flight = tokio::spawn({
        let service = Arc::clone(&service);
        let id = plan.id.clone();
        async move { service.send(&id).await }
    });
    started_rx.await.unwrap();
    assert_eq!(other.send(&plan.id).await.unwrap().status, Status::Sending);
    release_tx.send(()).unwrap();
    in_flight.await.unwrap().unwrap();
    assert_eq!(other.send(&plan.id).await.unwrap().status, Status::Sent);
    assert_eq!(adapter.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn uncertain_and_rejected_results_are_terminal_and_never_retried() {
    for (outcome, status) in [
        (Outcome::Unknown, Status::OutcomeUnknown),
        (Outcome::NotSent, Status::Failed),
    ] {
        let adapter =
            FakeAdapter::new(move || Err(DeliveryError::new("TEST", "Safe message", outcome)));
        let f = fixture(json!({ "mode": "live" }), whatsapp(adapter.clone()));
        let plan = f.service.prepare(&draft(KEY)).unwrap();
        f.service.approve(&plan.id).unwrap();
        assert_eq!(f.service.send(&plan.id).await.unwrap().status, status);
        assert_eq!(f.service.send(&plan.id).await.unwrap().status, status);
        assert_eq!(adapter.count(), 1);
        assert_eq!(
            f.service.approve(&plan.id).unwrap_err().code,
            "PLAN_TERMINAL"
        );
    }
}

#[tokio::test]
async fn invalid_recipient_text_and_keys_fail_closed() {
    let f = fixture(json!({}), Adapters::default());
    let unknown = PrepareInput {
        recipient_id: "not-allowed".to_string(),
        ..draft(KEY)
    };
    assert_eq!(
        f.service.prepare(&unknown).unwrap_err().code,
        "RECIPIENT_NOT_ALLOWED"
    );
    let long = "a".repeat(4097);
    for text in ["", "  ", long.as_str(), "\u{1b}[2J", "a\u{0}b"] {
        let input = PrepareInput {
            text: text.to_string(),
            ..draft(KEY)
        };
        assert_eq!(
            f.service.prepare(&input).unwrap_err().code,
            "INVALID_MESSAGE"
        );
    }
    // 4096 UTF-16 units is the limit, so a text of emoji counts double.
    let emoji = PrepareInput {
        text: "👋".repeat(2049),
        ..draft(KEY)
    };
    assert_eq!(
        f.service.prepare(&emoji).unwrap_err().code,
        "INVALID_MESSAGE"
    );
    assert_eq!(
        f.service.prepare(&draft("../../escape")).unwrap_err().code,
        "INVALID_MESSAGE"
    );
    assert_eq!(
        f.service.status("../../escape").unwrap_err().code,
        "PLAN_ID_INVALID"
    );
}

#[tokio::test]
async fn expiry_and_recipient_or_mode_changes_invalidate_a_pending_plan() {
    let clock = TestClock::new(100_000);
    let f = fixture(json!({}), Adapters::default());
    let service =
        MessagingService::new(f.config.clone(), Arc::clone(&f.store), Adapters::default())
            .with_identity(fake_identity())
            .with_clock(clock.clock());
    let plan = service.prepare(&draft(KEY)).unwrap();
    clock.set(100_000 + 600_001);
    assert_eq!(
        service.send(&plan.id).await.unwrap_err().code,
        "PLAN_EXPIRED"
    );
    assert_eq!(service.approve(&plan.id).unwrap_err().code, "PLAN_EXPIRED");
    clock.set(100_000);

    let mut changed = f.config.clone();
    changed.recipients[0] = Recipient::Whatsapp {
        id: "wa-test".to_string(),
        label: "WhatsApp test only".to_string(),
        target: "5511888888888@s.whatsapp.net".to_string(),
    };
    let moved = MessagingService::new(changed.clone(), Arc::clone(&f.store), Adapters::default())
        .with_identity(fake_identity())
        .with_clock(clock.clock());
    assert_eq!(
        moved.send(&plan.id).await.unwrap_err().code,
        "PLAN_CONTEXT_CHANGED"
    );

    changed.mode = Mode::Live;
    let live = MessagingService::new(changed, Arc::clone(&f.store), Adapters::default())
        .with_identity(fake_identity())
        .with_clock(clock.clock());
    assert_eq!(live.send(&plan.id).await.unwrap_err().code, "MODE_CHANGED");
}

#[tokio::test]
async fn the_rate_limit_persists_and_is_shared_across_clients() {
    let f = fixture(json!({ "maxSendsPerMinute": 1 }), Adapters::default());
    let first = f.service.prepare(&draft(KEY)).unwrap();
    f.service.send(&first.id).await.unwrap();
    let other = other_client(&f.config, Adapters::default());
    let second = other.prepare(&draft("intent_second")).unwrap();
    assert_eq!(
        other.send(&second.id).await.unwrap_err().code,
        "RATE_LIMITED"
    );
    assert_eq!(other.status(&second.id).unwrap().status, Status::Pending);
}

#[tokio::test]
async fn a_crash_after_the_claim_never_permits_a_resend_after_reopening() {
    let f = fixture(json!({}), Adapters::default());
    let plan = f.service.prepare(&draft(KEY)).unwrap();
    let claimed = messaging_mcp::plan::Plan {
        status: Status::Sending,
        attempted_at: Some(1),
        ..plan.clone()
    };
    f.store.transaction(|tx| tx.update(&claimed)).unwrap();
    let reopened = other_client(&f.config, Adapters::default());
    assert_eq!(
        reopened.send(&plan.id).await.unwrap().status,
        Status::Sending
    );
}

#[tokio::test]
async fn rotating_the_sender_identity_invalidates_an_approved_plan() {
    let adapter = FakeAdapter::accepting();
    let f = fixture(json!({ "mode": "live" }), whatsapp(adapter.clone()));
    let account = Arc::new(std::sync::Mutex::new("first-account".to_string()));
    let identity = {
        let account = Arc::clone(&account);
        Arc::new(move |_| {
            Ok(SenderIdentity {
                label: "Test account".to_string(),
                fingerprint: account.lock().unwrap().clone(),
            })
        })
    };
    let service = MessagingService::new(
        f.config.clone(),
        Arc::clone(&f.store),
        whatsapp(adapter.clone()),
    )
    .with_identity(identity);
    let plan = service.prepare(&draft(KEY)).unwrap();
    service.approve(&plan.id).unwrap();
    *account.lock().unwrap() = "second-account".to_string();
    assert_eq!(
        service.send(&plan.id).await.unwrap_err().code,
        "PLAN_CONTEXT_CHANGED"
    );
    assert_eq!(adapter.count(), 0);
}

/// While the adapter "sends", another connection takes an exclusive lock on the ledger, as another
/// client could, so the final result cannot be written.
struct LockingAdapter {
    state_dir: std::path::PathBuf,
    lock: std::sync::Mutex<Option<rusqlite::Connection>>,
}

#[async_trait]
impl ChannelAdapter for LockingAdapter {
    async fn send(&self, _: &Recipient, _: &str, _: &str) -> Result<Receipt, DeliveryError> {
        let connection = rusqlite::Connection::open(self.state_dir.join("plans.sqlite")).unwrap();
        connection.execute_batch("BEGIN EXCLUSIVE").unwrap();
        *self.lock.lock().unwrap() = Some(connection);
        Ok(Receipt {
            accepted: true,
            provider_message_id: None,
        })
    }
}

#[tokio::test]
async fn a_failed_final_write_never_says_the_send_did_not_happen() {
    let f = fixture(json!({ "mode": "live" }), Adapters::default());
    let adapter = Arc::new(LockingAdapter {
        state_dir: f.config.state_dir.clone(),
        lock: Default::default(),
    });
    let store = Arc::new(
        PlanStore::open_with_busy_timeout(&f.config.state_dir, Duration::from_millis(50)).unwrap(),
    );
    let adapters = Adapters {
        whatsapp: Some(adapter.clone()),
        telegram: None,
    };
    let service =
        MessagingService::new(f.config.clone(), store, adapters).with_identity(fake_identity());
    let plan = service.prepare(&draft(KEY)).unwrap();
    service.approve(&plan.id).unwrap();
    let error = service.send(&plan.id).await.unwrap_err();
    assert_eq!(error.code, "RESULT_NOT_PERSISTED");
    assert!(error.message.contains("send was attempted"));
    drop(adapter.lock.lock().unwrap().take());
    assert_eq!(f.service.status(&plan.id).unwrap().status, Status::Sending);
    assert_eq!(
        f.service.send(&plan.id).await.unwrap().status,
        Status::Sending
    );
}

fn maria() -> Contact {
    Contact {
        channel: Channel::Telegram,
        kind: "user",
        reference: "user:42".to_string(),
        name: "Maria".to_string(),
        username: None,
        recipient_id: None,
        peer: TelegramPeer::User {
            id: "42".to_string(),
            access_hash: "-7".to_string(),
        },
    }
}

#[tokio::test]
async fn contact_search_runs_only_in_live_mode_and_marks_allowed_contacts() {
    let directory = Arc::new(FakeDirectory {
        contacts: vec![maria()],
        searches: Default::default(),
    });
    let dry = fixture(json!({}), Adapters::default());
    let dry_service =
        other_client(&dry.config, Adapters::default()).with_directory(directory.clone());
    assert_eq!(
        dry_service.search_contacts("Maria").await.unwrap_err().code,
        "LIVE_MODE_REQUIRED"
    );
    assert!(directory.searches.lock().unwrap().is_empty());

    let live = fixture(json!({ "mode": "live" }), Adapters::default());
    let service = other_client(&live.config, Adapters::default()).with_directory(directory.clone());
    assert_eq!(
        service.search_contacts("x").await.unwrap_err().code,
        "INVALID_QUERY"
    );
    let found = service.search_contacts("Maria").await.unwrap();
    assert_eq!(found[0].recipient_id, None);
    let encoded = serde_json::to_string(&found).unwrap();
    assert!(!encoded.contains("accessHash") && !encoded.contains("-7"));

    let recipient = Recipient::Telegram {
        id: "maria".to_string(),
        label: "Maria".to_string(),
        target: maria().peer,
    };
    service.allow_recipient(&recipient).unwrap();
    let found = service.search_contacts("Maria").await.unwrap();
    assert_eq!(found[0].recipient_id.as_deref(), Some("maria"));
}

#[tokio::test]
async fn allowed_recipients_are_shared_and_revoking_one_blocks_its_plans() {
    let f = fixture(json!({}), Adapters::default());
    let recipient = Recipient::Whatsapp {
        id: "ana".to_string(),
        label: "Ana".to_string(),
        target: "5511977776666@s.whatsapp.net".to_string(),
    };
    f.service.allow_recipient(&recipient).unwrap();
    assert_eq!(
        f.service.allow_recipient(&recipient).unwrap_err().code,
        "RECIPIENT_EXISTS"
    );
    let whatsapp = |id: &str, target: &str| Recipient::Whatsapp {
        id: id.to_string(),
        label: "Ana".to_string(),
        target: target.to_string(),
    };
    let in_config = whatsapp("wa-test", "5511977776666@s.whatsapp.net");
    assert_eq!(
        f.service.allow_recipient(&in_config).unwrap_err().code,
        "RECIPIENT_EXISTS"
    );
    let bad = whatsapp("ana-2", "+55 11 9");
    assert_eq!(
        f.service.allow_recipient(&bad).unwrap_err().code,
        "CONFIG_INVALID"
    );

    let other = other_client(&f.config, Adapters::default());
    assert!(
        other
            .list_recipients()
            .unwrap()
            .iter()
            .any(|r| r.id == "ana")
    );
    let input = PrepareInput {
        recipient_id: "ana".to_string(),
        ..draft(KEY)
    };
    let plan = other.prepare(&input).unwrap();
    f.service.revoke_recipient("ana").unwrap();
    assert_eq!(
        other.send(&plan.id).await.unwrap_err().code,
        "RECIPIENT_NOT_ALLOWED"
    );
    assert_eq!(
        f.service.revoke_recipient("wa-test").unwrap_err().code,
        "RECIPIENT_IN_CONFIG"
    );
}
