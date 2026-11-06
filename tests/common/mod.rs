#![allow(dead_code, clippy::unwrap_used)]

use std::path::Path;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use messaging_mcp::attachment::Outgoing;
use messaging_mcp::config::{Config, Recipient, parse_config};
use messaging_mcp::error::{DeliveryError, Error};
use messaging_mcp::identity::SenderIdentity;
use messaging_mcp::plan::{ChannelAdapter, Contact, ContactDirectory, Receipt};
use messaging_mcp::service::{Adapters, Clock, MessagingService, PrepareInput};
use messaging_mcp::store::PlanStore;
use serde_json::{Value, json};
use tempfile::TempDir;

pub fn recipients() -> Value {
    json!([
        { "id": "wa-test", "label": "WhatsApp test only", "channel": "whatsapp",
          "target": "5511999999999@s.whatsapp.net" },
        { "id": "tg-self", "label": "Telegram Saved Messages", "channel": "telegram",
          "target": { "kind": "self" } },
    ])
}

pub fn config_with(dir: &Path, overrides: Value) -> Config {
    let mut input = json!({ "stateDir": dir.join("state"), "recipients": recipients() });
    for (key, value) in overrides.as_object().unwrap() {
        input[key] = value.clone();
    }
    parse_config(&input.to_string(), dir).unwrap()
}

pub fn fake_identity() -> messaging_mcp::identity::IdentifySender {
    Arc::new(|_| {
        Ok(SenderIdentity {
            label: "Test account (fake)".to_string(),
            fingerprint: "test-account".to_string(),
        })
    })
}

/// A clock the test moves by hand.
#[derive(Clone)]
pub struct TestClock(pub Arc<AtomicI64>);

impl TestClock {
    pub fn new(start: i64) -> Self {
        Self(Arc::new(AtomicI64::new(start)))
    }
    pub fn set(&self, value: i64) {
        self.0.store(value, Ordering::SeqCst);
    }
    pub fn clock(&self) -> Clock {
        let value = Arc::clone(&self.0);
        Arc::new(move || value.load(Ordering::SeqCst))
    }
}

pub struct Fixture {
    pub dir: TempDir,
    pub config: Config,
    pub store: Arc<PlanStore>,
    pub service: MessagingService,
}

pub fn fixture(overrides: Value, adapters: Adapters) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let config = config_with(dir.path(), overrides);
    let store = Arc::new(PlanStore::open(&config.state_dir).unwrap());
    let service = MessagingService::new(config.clone(), Arc::clone(&store), adapters)
        .with_identity(fake_identity());
    Fixture {
        dir,
        config,
        store,
        service,
    }
}

/// Another client on the same machine: its own connection to the same state directory.
pub fn other_client(config: &Config, adapters: Adapters) -> MessagingService {
    let store = Arc::new(PlanStore::open(&config.state_dir).unwrap());
    MessagingService::new(config.clone(), store, adapters).with_identity(fake_identity())
}

pub fn draft(key: &str) -> PrepareInput {
    PrepareInput {
        recipient_id: "wa-test".to_string(),
        text: "Olá, teste 👋".to_string(),
        file_path: None,
        idempotency_key: key.to_string(),
    }
}

pub const KEY: &str = "intent_00000001";

pub type SendResult = Result<Receipt, DeliveryError>;

/// An adapter that records every call and answers with whatever `respond` returns.
pub struct FakeAdapter {
    pub calls: Mutex<Vec<(Recipient, String, String)>>,
    /// The files sent: name and bytes.
    pub files: Mutex<Vec<(String, Vec<u8>)>>,
    respond: Box<dyn Fn() -> SendResult + Send + Sync>,
}

impl FakeAdapter {
    pub fn new(respond: impl Fn() -> SendResult + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            files: Mutex::new(Vec::new()),
            respond: Box::new(respond),
        })
    }
    pub fn accepting() -> Arc<Self> {
        Self::new(|| {
            Ok(Receipt {
                accepted: true,
                provider_message_id: None,
            })
        })
    }
    pub fn count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

#[async_trait]
impl ChannelAdapter for FakeAdapter {
    async fn send(
        &self,
        recipient: &Recipient,
        message: &Outgoing<'_>,
        plan_id: &str,
    ) -> SendResult {
        self.calls.lock().unwrap().push((
            recipient.clone(),
            message.text.to_string(),
            plan_id.to_string(),
        ));
        if let Some(file) = &message.file {
            self.files
                .lock()
                .unwrap()
                .push((file.name.to_string(), file.data.to_vec()));
        }
        (self.respond)()
    }
}

pub fn whatsapp(adapter: Arc<FakeAdapter>) -> Adapters {
    Adapters {
        whatsapp: Some(adapter),
        telegram: None,
    }
}

/// A directory that returns fixed contacts and counts searches.
pub struct FakeDirectory {
    pub contacts: Vec<Contact>,
    pub searches: Mutex<Vec<String>>,
}

#[async_trait]
impl ContactDirectory for FakeDirectory {
    async fn search(&self, query: &str, _limit: i32) -> Result<Vec<Contact>, Error> {
        self.searches.lock().unwrap().push(query.to_string());
        Ok(self.contacts.clone())
    }
}
