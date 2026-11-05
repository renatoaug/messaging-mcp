#![allow(clippy::unwrap_used)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use grammers_tl_types::{self as tl, Serializable};
use messaging_mcp::adapters::telegram::{
    TelegramAdapter, TelegramTransport, TransportError, TransportFactory, clean_name, input_peer,
    receipt_from, send_request, telegram_random_id,
};
use messaging_mcp::adapters::whatsapp::WhatsAppAdapter;
use messaging_mcp::config::{Recipient, TelegramPeer, WhatsAppConfig};
use messaging_mcp::error::{Error, Outcome};
use messaging_mcp::plan::{ChannelAdapter, Contact, Receipt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const TOKEN: &str = "test-only-token-never-real";

fn wa_recipient() -> Recipient {
    Recipient::Whatsapp {
        id: "wa-test".to_string(),
        label: "WhatsApp test only".to_string(),
        target: "5511999999999@s.whatsapp.net".to_string(),
    }
}

fn tg_self() -> Recipient {
    Recipient::Telegram {
        id: "tg-self".to_string(),
        label: "Saved Messages".to_string(),
        target: TelegramPeer::Myself,
    }
}

/// A one-shot loopback HTTP server that records requests and replies with `response`.
/// `None` never replies, to exercise the timeout.
async fn bridge(response: Option<String>) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&requests);
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut buffer = vec![0u8; 65_536];
            let read = socket.read(&mut buffer).await.unwrap();
            seen.lock()
                .unwrap()
                .push(String::from_utf8_lossy(&buffer[..read]).to_string());
            match &response {
                Some(response) => {
                    socket.write_all(response.as_bytes()).await.unwrap();
                    socket.shutdown().await.unwrap();
                }
                None => tokio::time::sleep(Duration::from_secs(30)).await,
            }
        }
    });
    (format!("http://127.0.0.1:{port}/api"), requests)
}

fn http(status: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\
         connection: close\r\n\r\n{body}",
        body.len()
    )
}

fn adapter(base_url: String, token: Result<String, Error>) -> WhatsAppAdapter {
    let config = WhatsAppConfig {
        account_label: None,
        base_url,
        token_file: None,
        timeout_ms: 300,
    };
    WhatsAppAdapter::with_token(config, Arc::new(move || token.clone())).unwrap()
}

#[tokio::test]
async fn whatsapp_matches_the_pinned_bridge_contract_once() {
    let (url, requests) =
        bridge(Some(http("200 OK", r#"{"success":true,"message":"sent"}"#))).await;
    let receipt = adapter(url, Ok(TOKEN.to_string()))
        .send(&wa_recipient(), "hi", "plan")
        .await;
    assert_eq!(
        receipt.unwrap(),
        Receipt {
            accepted: true,
            provider_message_id: None
        }
    );
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let request = requests[0].to_lowercase();
    assert!(request.starts_with("post /api/send http/1.1"));
    assert!(request.contains(&format!("authorization: bearer {TOKEN}")));
    assert!(
        requests[0].ends_with(r#"{"message":"hi","recipient":"5511999999999@s.whatsapp.net"}"#)
    );
}

#[tokio::test]
async fn whatsapp_timeouts_errors_and_odd_bodies_are_uncertain() {
    let large = "x".repeat(17_000);
    for response in [
        None,
        Some(http("500 Internal Server Error", "oops")),
        Some(http("302 Found", "")),
        Some(http("200 OK", "bad json")),
        Some(http("200 OK", r#"{"success":false}"#)),
        Some(http("200 OK", &large)),
    ] {
        let (url, requests) = bridge(response).await;
        let error = adapter(url, Ok(TOKEN.to_string()))
            .send(&wa_recipient(), "hi", "plan")
            .await
            .unwrap_err();
        assert_eq!(error.outcome, Outcome::Unknown);
        assert!(!error.message.contains(TOKEN));
        assert_eq!(requests.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn whatsapp_auth_rejection_is_not_sent_and_a_missing_token_sends_nothing() {
    let (url, _) = bridge(Some(http("401 Unauthorized", "Unauthorized"))).await;
    let error = adapter(url, Ok(TOKEN.to_string()))
        .send(&wa_recipient(), "hi", "plan")
        .await
        .unwrap_err();
    assert_eq!(
        (error.code, error.outcome),
        ("WHATSAPP_REJECTED", Outcome::NotSent)
    );

    let (url, requests) = bridge(Some(http("200 OK", r#"{"success":true}"#))).await;
    let missing = Err(Error::new("SECRET_UNAVAILABLE", "secret"));
    let error = adapter(url.clone(), missing)
        .send(&wa_recipient(), "hi", "plan")
        .await
        .unwrap_err();
    assert_eq!(error.outcome, Outcome::NotSent);
    let short = adapter(url, Ok("short".to_string()))
        .send(&wa_recipient(), "hi", "plan")
        .await;
    assert_eq!(short.unwrap_err().outcome, Outcome::NotSent);
    assert!(requests.lock().unwrap().is_empty());
}

#[derive(Default)]
struct Events(Mutex<Vec<&'static str>>);

struct FakeTransport {
    events: Arc<Events>,
    authorized: Result<bool, ()>,
    hang_on_send: bool,
    sends: Arc<AtomicUsize>,
    fail_close: bool,
}

#[async_trait]
impl TelegramTransport for FakeTransport {
    async fn authorized(&self) -> Result<bool, TransportError> {
        self.events.0.lock().unwrap().push("authorized");
        self.authorized.map_err(|()| TransportError::Failed)
    }
    async fn send(
        &self,
        target: &TelegramPeer,
        text: &str,
        random_id: i64,
    ) -> Result<Receipt, TransportError> {
        self.events.0.lock().unwrap().push("send");
        self.sends.fetch_add(1, Ordering::SeqCst);
        assert_eq!(target, &TelegramPeer::Myself);
        assert_eq!(text, "**literal text**");
        assert_eq!(random_id, telegram_random_id("plan"));
        if self.hang_on_send {
            std::future::pending::<()>().await;
        }
        Ok(Receipt {
            accepted: true,
            provider_message_id: Some("23".to_string()),
        })
    }
    async fn search(&self, _: &str, _: i32) -> Result<Vec<Contact>, TransportError> {
        Ok(Vec::new())
    }
    async fn close(&self) -> Result<(), TransportError> {
        self.events.0.lock().unwrap().push("close");
        if self.fail_close {
            Err(TransportError::Failed)
        } else {
            Ok(())
        }
    }
}

struct Setup {
    authorized: Result<bool, ()>,
    hang_on_send: bool,
    fail_close: bool,
}

fn telegram(setup: Setup, timeout_ms: u64) -> (TelegramAdapter, Arc<Events>, Arc<AtomicUsize>) {
    let events = Arc::new(Events::default());
    let sends = Arc::new(AtomicUsize::new(0));
    let (shared_events, shared_sends) = (Arc::clone(&events), Arc::clone(&sends));
    let factory: TransportFactory = Arc::new(move || {
        let transport = FakeTransport {
            events: Arc::clone(&shared_events),
            authorized: setup.authorized,
            hang_on_send: setup.hang_on_send,
            sends: Arc::clone(&shared_sends),
            fail_close: setup.fail_close,
        };
        Box::pin(async move { Ok(Box::new(transport) as Box<dyn TelegramTransport>) })
    });
    (
        TelegramAdapter::with_transport(Duration::from_millis(timeout_ms), factory),
        events,
        sends,
    )
}

const READY: Setup = Setup {
    authorized: Ok(true),
    hang_on_send: false,
    fail_close: false,
};

#[tokio::test]
async fn telegram_uses_the_existing_session_explicit_peer_and_stable_random_id() {
    let (adapter, events, _) = telegram(READY, 1000);
    let receipt = adapter
        .send(&tg_self(), "**literal text**", "plan")
        .await
        .unwrap();
    assert_eq!(receipt.provider_message_id.as_deref(), Some("23"));
    assert_eq!(*events.0.lock().unwrap(), ["authorized", "send", "close"]);
    assert!(telegram_random_id("plan") > 0);
    assert_eq!(telegram_random_id("plan"), telegram_random_id("plan"));
    assert_ne!(telegram_random_id("plan"), telegram_random_id("other"));
}

#[tokio::test]
async fn telegram_refuses_an_unauthorized_session_without_sending_and_closes() {
    let (adapter, events, sends) = telegram(
        Setup {
            authorized: Ok(false),
            ..READY
        },
        1000,
    );
    let error = adapter.send(&tg_self(), "hi", "plan").await.unwrap_err();
    assert_eq!(
        (error.code, error.outcome),
        ("TELEGRAM_NOT_AUTHORIZED", Outcome::NotSent)
    );
    assert_eq!(sends.load(Ordering::SeqCst), 0);
    assert!(events.0.lock().unwrap().contains(&"close"));

    let (adapter, _, sends) = telegram(
        Setup {
            authorized: Err(()),
            ..READY
        },
        1000,
    );
    let error = adapter.send(&tg_self(), "hi", "plan").await.unwrap_err();
    assert_eq!(
        (error.code, error.outcome),
        ("TELEGRAM_UNAVAILABLE", Outcome::NotSent)
    );
    assert_eq!(sends.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn telegram_times_out_a_send_once_reports_uncertain_and_closes() {
    let (adapter, events, sends) = telegram(
        Setup {
            hang_on_send: true,
            ..READY
        },
        20,
    );
    let error = adapter
        .send(&tg_self(), "**literal text**", "plan")
        .await
        .unwrap_err();
    assert_eq!(error.outcome, Outcome::Unknown);
    assert_eq!(sends.load(Ordering::SeqCst), 1);
    assert_eq!(events.0.lock().unwrap().last(), Some(&"close"));
}

#[tokio::test]
async fn a_telegram_close_failure_cannot_replace_an_accepted_message() {
    let (adapter, _, _) = telegram(
        Setup {
            fail_close: true,
            ..READY
        },
        1000,
    );
    assert!(
        adapter
            .send(&tg_self(), "**literal text**", "plan")
            .await
            .unwrap()
            .accepted
    );
}

#[test]
fn the_telegram_request_is_literal_text_with_no_preview_and_no_payment() {
    let peer = input_peer(&TelegramPeer::User {
        id: "42".into(),
        access_hash: "-7".into(),
    });
    let request = send_request(peer.clone().unwrap(), "**literal**", 99);
    assert!(request.no_webpage);
    assert_eq!(request.message, "**literal**");
    assert_eq!(request.entities, None);
    assert_eq!(request.allow_paid_stars, None);
    assert!(!request.allow_paid_floodskip);
    assert_eq!(request.random_id, 99);
    assert!(request.to_bytes().len() > 20);
    let expected: tl::enums::InputPeer = tl::types::InputPeerUser {
        user_id: 42,
        access_hash: -7,
    }
    .into();
    assert_eq!(peer, Some(expected));
    let invalid = TelegramPeer::Chat {
        id: "not-a-number".into(),
    };
    assert_eq!(input_peer(&invalid), None);
}

#[test]
fn a_receipt_takes_only_the_message_id_matching_our_random_id() {
    let update =
        |id, random_id| tl::enums::Update::MessageId(tl::types::UpdateMessageId { id, random_id });
    let updates = tl::enums::Updates::Updates(tl::types::Updates {
        updates: vec![update(5, 1), update(6, 2)],
        users: vec![],
        chats: vec![],
        date: 0,
        seq: 0,
    });
    assert_eq!(
        receipt_from(&updates, 2).provider_message_id.as_deref(),
        Some("6")
    );
    assert_eq!(receipt_from(&updates, 3).provider_message_id, None);
    assert!(receipt_from(&tl::enums::Updates::TooLong, 3).accepted);
}

#[test]
fn contact_names_lose_control_characters_and_stay_short() {
    assert_eq!(clean_name("Ma\u{1b}[2Jria\n"), "Ma[2Jria");
    assert_eq!(clean_name(&"a".repeat(300)).len(), 100);
}
