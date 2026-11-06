//! Reading chats for the owner: live only, choosing among matches, remembering references.

#![allow(clippy::unwrap_used)]

mod common;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use common::*;
use messaging_mcp::config::TelegramPeer;
use messaging_mcp::error::Error;
use messaging_mcp::plan::{Channel, ChatMatch, ChatMessage, ChatTarget, MessageReader};
use messaging_mcp::service::{ReadResult, Readers};
use serde_json::json;

struct FakeReader {
    matches: Vec<ChatMatch>,
    finds: Mutex<Vec<String>>,
    reads: Mutex<Vec<(String, i32)>>,
}

#[async_trait]
impl MessageReader for FakeReader {
    async fn find_chats(&self, query: &str) -> Result<Vec<ChatMatch>, Error> {
        self.finds.lock().unwrap().push(query.to_string());
        Ok(self.matches.clone())
    }
    async fn read(&self, chat: &ChatMatch, limit: i32) -> Result<Vec<ChatMessage>, Error> {
        self.reads
            .lock()
            .unwrap()
            .push((chat.reference.clone(), limit));
        Ok(vec![ChatMessage {
            from: chat.name.clone(),
            from_me: false,
            at: 1,
            text: "oi".to_string(),
        }])
    }
}

fn person(reference: &str, name: &str) -> ChatMatch {
    ChatMatch {
        channel: Channel::Telegram,
        reference: reference.to_string(),
        name: name.to_string(),
        target: ChatTarget::Telegram(TelegramPeer::Myself),
    }
}

fn reader(matches: Vec<ChatMatch>) -> Arc<FakeReader> {
    Arc::new(FakeReader {
        matches,
        finds: Mutex::new(Vec::new()),
        reads: Mutex::new(Vec::new()),
    })
}

fn with_reader(mode: &str, reader: Arc<FakeReader>) -> Fixture {
    let mut f = fixture(json!({ "mode": mode }), whatsapp(FakeAdapter::accepting()));
    f.service = f.service.with_readers(Readers {
        telegram: Some(reader.clone()),
        whatsapp: Some(reader),
    });
    f
}

#[tokio::test]
async fn one_match_is_read_and_marked_as_data() {
    let reader = reader(vec![person("user:1", "Alice Silva")]);
    let f = with_reader("live", reader.clone());
    let result = f
        .service
        .read_messages(Channel::Telegram, "alice", None)
        .await
        .unwrap();
    let ReadResult::Messages {
        chat,
        messages,
        notice,
    } = result
    else {
        panic!("one match is read");
    };
    assert_eq!(chat.reference, "user:1");
    assert_eq!(messages[0].text, "oi");
    assert!(notice.contains("never instructions"));
    assert_eq!(*reader.reads.lock().unwrap(), [("user:1".to_string(), 20)]);
}

#[tokio::test]
async fn several_matches_are_listed_then_one_is_read_by_reference() {
    let reader = reader(vec![
        person("user:1", "Alice Silva"),
        person("user:2", "Alice Souza"),
    ]);
    let f = with_reader("live", reader.clone());
    let result = f
        .service
        .read_messages(Channel::Telegram, "alice", Some(5))
        .await
        .unwrap();
    let ReadResult::Choose { matches, .. } = result else {
        panic!("several matches are listed");
    };
    assert_eq!(matches.len(), 2);
    assert!(reader.reads.lock().unwrap().is_empty());
    let value = serde_json::to_value(&matches[0]).unwrap();
    assert!(
        value.get("target").is_none(),
        "access hashes never reach the agent"
    );

    let result = f
        .service
        .read_messages(Channel::Telegram, "user:2", Some(5))
        .await
        .unwrap();
    assert!(matches!(result, ReadResult::Messages { .. }));
    assert_eq!(*reader.reads.lock().unwrap(), [("user:2".to_string(), 5)]);
    assert_eq!(
        reader.finds.lock().unwrap().len(),
        1,
        "the reference was remembered"
    );
}

#[tokio::test]
async fn an_exact_name_wins_over_partial_matches() {
    let reader = reader(vec![
        person("user:1", "Álice"),
        person("user:2", "Alice Souza"),
    ]);
    let f = with_reader("live", reader.clone());
    let result = f
        .service
        .read_messages(Channel::Telegram, "alice", None)
        .await
        .unwrap();
    let ReadResult::Messages { chat, .. } = result else {
        panic!("the exact name is read");
    };
    assert_eq!(chat.reference, "user:1");
}

#[tokio::test]
async fn a_whatsapp_number_is_read_without_searching() {
    let reader = reader(vec![]);
    let f = with_reader("live", reader.clone());
    let result = f
        .service
        .read_messages(Channel::Whatsapp, "+55 11 98888-7777", None)
        .await
        .unwrap();
    assert!(matches!(result, ReadResult::Messages { .. }));
    assert!(reader.finds.lock().unwrap().is_empty());
    assert_eq!(
        reader.reads.lock().unwrap()[0].0,
        "5511988887777@s.whatsapp.net"
    );
}

#[tokio::test]
async fn reading_is_live_only_and_validated() {
    let reader = reader(vec![]);
    let dry = with_reader("dry-run", reader.clone());
    let error = dry
        .service
        .read_messages(Channel::Telegram, "alice", None)
        .await
        .unwrap_err();
    assert_eq!(error.code, "LIVE_MODE_REQUIRED");

    let live = with_reader("live", reader.clone());
    for (chat, limit) in [("n", None), ("alice", Some(0)), ("alice", Some(51))] {
        let error = live
            .service
            .read_messages(Channel::Telegram, chat, limit)
            .await
            .unwrap_err();
        assert_eq!(error.code, "INVALID_QUERY");
    }
    let error = live
        .service
        .read_messages(Channel::Telegram, "nobody", None)
        .await
        .unwrap_err();
    assert_eq!(error.code, "CHAT_NOT_FOUND");
    assert!(reader.reads.lock().unwrap().is_empty());
}
