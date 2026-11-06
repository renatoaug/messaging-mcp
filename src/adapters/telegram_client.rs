//! The network side of Telegram: the real MTProto client. Nothing here runs without an
//! account, so the offline suite leaves it out of the coverage floor; the first live test to
//! Saved Messages exercises it.

use std::sync::Arc;

use async_trait::async_trait;
use grammers_client::Client;
use grammers_mtsender::{ConnectionParams, NoRetries, SenderPool, SenderPoolHandle};
use grammers_session::storages::MemorySession;
use grammers_tl_types as tl;
use tokio::task::JoinHandle;

use crate::adapters::telegram::{
    StoredSession, TelegramTransport, TransportError, UploadPart, contacts_from_address_book,
    contacts_from_found, input_peer, read_credentials, receipt_from, send_media_request,
    send_request, upload_parts, uploaded_file,
};
use crate::config::{TelegramConfig, TelegramPeer};
use crate::error::{DeliveryError, Error, Outcome};
use crate::identity::TELEGRAM_SESSION_ENV;
use crate::plan::{Contact, Receipt};
use crate::secrets::{process_env, secret_from};

/// A sender pool that never retries on its own: an uncertain send must stay uncertain.
pub fn single_attempt_pool(session: Arc<MemorySession>, api_id: i32) -> SenderPool {
    SenderPool::with_configuration(
        session,
        api_id,
        ConnectionParams {
            device_model: "Messaging MCP".to_string(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            retry_policy: Box::new(NoRetries),
            ..Default::default()
        },
    )
}

/// Who a saved session belongs to, asked of Telegram. Refuses bots and unauthorized sessions.
pub async fn session_user_id(
    config: &TelegramConfig,
    session_path: &std::path::Path,
) -> Result<String, Error> {
    let failed = || {
        Error::new(
            "TELEGRAM_SESSION_INVALID",
            "The saved session cannot be used.",
        )
    };
    let credentials = read_credentials(config, &process_env)?;
    let session = crate::secrets::read_secret_file(session_path)?;
    let session: StoredSession = serde_json::from_str(&session).map_err(|_| failed())?;
    let pool = single_attempt_pool(Arc::new(session.into_memory_session()), credentials.api_id);
    let handle = pool.handle.thin.clone();
    let client = Client::new(pool.handle);
    let runner = tokio::spawn(pool.runner.run());
    let user = async {
        if !client.is_authorized().await.map_err(|_| failed())? {
            return Err(failed());
        }
        let me = client.get_me().await.map_err(|_| failed())?;
        if me.is_bot() {
            return Err(failed());
        }
        me.id()
            .bare_id()
            .map(|id| id.to_string())
            .ok_or_else(failed)
    }
    .await;
    handle.quit();
    runner.abort();
    user
}

struct GrammersTransport {
    client: Client,
    handle: SenderPoolHandle,
    runner: JoinHandle<()>,
    expected_user_id: i64,
}

/// Builds the real transport. The network is first used by the first request, not here.
pub fn connect(config: &TelegramConfig) -> Result<Box<dyn TelegramTransport>, TransportError> {
    Ok(Box::new(GrammersTransport::connect(config)?))
}

impl GrammersTransport {
    fn connect(config: &TelegramConfig) -> Result<Self, TransportError> {
        let credentials =
            read_credentials(config, &process_env).map_err(|_| TransportError::Failed)?;
        let expected_user_id = config
            .expected_user_id
            .as_deref()
            .and_then(|id| id.parse::<i64>().ok())
            .ok_or(TransportError::Failed)?;
        let session = secret_from(
            &process_env,
            TELEGRAM_SESSION_ENV,
            config.session_file.as_deref(),
        )
        .map_err(|_| TransportError::Failed)?;
        let session: StoredSession =
            serde_json::from_str(&session).map_err(|_| TransportError::Failed)?;
        let pool = single_attempt_pool(Arc::new(session.into_memory_session()), credentials.api_id);
        let handle = pool.handle.thin.clone();
        let client = Client::new(pool.handle);
        let runner = tokio::spawn(pool.runner.run());
        Ok(Self {
            client,
            handle,
            runner,
            expected_user_id,
        })
    }
}

fn account_mismatch() -> TransportError {
    TransportError::Delivery(DeliveryError::new(
        "TELEGRAM_ACCOUNT_MISMATCH",
        "Telegram session is not the configured personal account. No message was sent.",
        Outcome::NotSent,
    ))
}

#[async_trait]
impl TelegramTransport for GrammersTransport {
    async fn authorized(&self) -> Result<bool, TransportError> {
        if !self
            .client
            .is_authorized()
            .await
            .map_err(|_| TransportError::Failed)?
        {
            return Ok(false);
        }
        let me = self
            .client
            .get_me()
            .await
            .map_err(|_| TransportError::Failed)?;
        if me.is_bot() || me.id().bare_id() != Some(self.expected_user_id) {
            return Err(account_mismatch());
        }
        Ok(true)
    }

    async fn upload(
        &self,
        name: &str,
        data: &[u8],
        file_id: i64,
    ) -> Result<tl::enums::InputFile, TransportError> {
        for part in upload_parts(file_id, data) {
            let saved = match part {
                UploadPart::Small(request) => self.client.invoke(&request).await,
                UploadPart::Big(request) => self.client.invoke(&request).await,
            };
            if !saved.map_err(|_| TransportError::Failed)? {
                return Err(TransportError::Failed);
            }
        }
        Ok(uploaded_file(file_id, name, data.len()))
    }

    async fn send(
        &self,
        target: &TelegramPeer,
        text: &str,
        media: Option<tl::enums::InputMedia>,
        random_id: i64,
    ) -> Result<Receipt, TransportError> {
        let peer = input_peer(target).ok_or_else(|| {
            TransportError::Delivery(DeliveryError::new(
                "TELEGRAM_TARGET_INVALID",
                "Telegram target is invalid. No message was sent.",
                Outcome::NotSent,
            ))
        })?;
        let updates = match media {
            Some(media) => {
                let request = send_media_request(peer, text, media, random_id);
                self.client.invoke(&request).await
            }
            None => {
                self.client
                    .invoke(&send_request(peer, text, random_id))
                    .await
            }
        }
        .map_err(|_| TransportError::Failed)?;
        Ok(receipt_from(&updates, random_id))
    }

    /// The owner's address book, filtered here, then their chats and channels from Telegram's
    /// search. Telegram's search did not return address book people on 2026-11-05, only the
    /// global directory, which is never used.
    async fn search(&self, query: &str, limit: i32) -> Result<Vec<Contact>, TransportError> {
        let max = usize::try_from(limit).unwrap_or(20);
        let book = self
            .client
            .invoke(&tl::functions::contacts::GetContacts { hash: 0 })
            .await
            .map_err(|_| TransportError::Failed)?;
        let mut contacts = match book {
            tl::enums::contacts::Contacts::Contacts(book) => {
                contacts_from_address_book(&book.users, query, max)
            }
            tl::enums::contacts::Contacts::NotModified => Vec::new(),
        };
        let request = tl::functions::contacts::Search {
            broadcasts: true,
            bots: false,
            q: query.to_string(),
            limit,
        };
        let tl::enums::contacts::Found::Found(found) = self
            .client
            .invoke(&request)
            .await
            .map_err(|_| TransportError::Failed)?;
        for contact in contacts_from_found(&found) {
            let known = contacts.iter().any(|c| c.reference == contact.reference);
            if !known && contacts.len() < max {
                contacts.push(contact);
            }
        }
        Ok(contacts)
    }

    async fn close(&self) -> Result<(), TransportError> {
        self.handle.quit();
        self.runner.abort();
        Ok(())
    }
}
