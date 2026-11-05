use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use grammers_session::SessionData;
use grammers_session::storages::MemorySession;
use grammers_session::types::DcOption;
use grammers_tl_types as tl;
use serde::{Deserialize, Serialize};
use tokio::time::timeout;

use crate::adapters::telegram_client;
use crate::config::{Recipient, TelegramConfig, TelegramPeer};
use crate::error::{DeliveryError, Error, Outcome};
use crate::identity::{TELEGRAM_API_HASH_ENV, TELEGRAM_API_ID_ENV};
use crate::plan::{Channel, ChannelAdapter, Contact, ContactDirectory, Receipt};
use crate::secrets::{Env, secret_from};
use crate::service::sha256_hex;

const CLOSE_TIMEOUT: Duration = Duration::from_secs(3);

/// A failure inside the transport. It carries no text, so a provider error can never leak.
#[derive(Debug)]
pub enum TransportError {
    Delivery(DeliveryError),
    Failed,
}

#[async_trait]
pub trait TelegramTransport: Send + Sync {
    async fn authorized(&self) -> Result<bool, TransportError>;
    async fn send(
        &self,
        target: &TelegramPeer,
        text: &str,
        random_id: i64,
    ) -> Result<Receipt, TransportError>;
    async fn search(&self, query: &str, limit: i32) -> Result<Vec<Contact>, TransportError>;
    async fn close(&self) -> Result<(), TransportError>;
}

pub type TransportFuture =
    Pin<Box<dyn Future<Output = Result<Box<dyn TelegramTransport>, TransportError>> + Send>>;
pub type TransportFactory = Arc<dyn Fn() -> TransportFuture + Send + Sync>;

/// A stable, nonzero, positive `random_id`, so Telegram can recognize a resent request.
pub fn telegram_random_id(plan_id: &str) -> i64 {
    let hash = sha256_hex(plan_id.as_bytes());
    let value = u64::from_str_radix(&hash[..16], 16).unwrap_or(1) & 0x7fff_ffff_ffff_ffff;
    i64::try_from(value)
        .ok()
        .filter(|value| *value != 0)
        .unwrap_or(1)
}

fn unavailable() -> DeliveryError {
    DeliveryError::new(
        "TELEGRAM_UNAVAILABLE",
        "Telegram credentials, session, or connection are unavailable. No message send was \
         started.",
        Outcome::NotSent,
    )
}

fn uncertain() -> DeliveryError {
    DeliveryError::new(
        "TELEGRAM_OUTCOME_UNKNOWN",
        "Telegram did not confirm the result. Verify the chat; no automatic retry was made.",
        Outcome::Unknown,
    )
}

fn not_authorized() -> DeliveryError {
    DeliveryError::new(
        "TELEGRAM_NOT_AUTHORIZED",
        "Telegram session is not authorized. Authenticate manually; this server cannot log in.",
        Outcome::NotSent,
    )
}

/// Sends through an existing personal session. Never a bot, never a login, one attempt.
pub struct TelegramAdapter {
    timeout: Duration,
    create_transport: TransportFactory,
}

impl TelegramAdapter {
    pub fn new(config: TelegramConfig) -> Self {
        let timeout = Duration::from_millis(config.timeout_ms);
        let factory: TransportFactory = Arc::new(move || {
            let config = config.clone();
            Box::pin(async move { telegram_client::connect(&config) })
        });
        Self::with_transport(timeout, factory)
    }

    pub fn with_transport(timeout: Duration, create_transport: TransportFactory) -> Self {
        Self {
            timeout,
            create_transport,
        }
    }

    /// Opens a transport and verifies the account. Any failure here means nothing was sent.
    async fn open(&self) -> Result<Box<dyn TelegramTransport>, DeliveryError> {
        let transport = (self.create_transport)()
            .await
            .map_err(|error| match error {
                TransportError::Delivery(error) => error,
                TransportError::Failed => unavailable(),
            })?;
        let authorized = match timeout(self.timeout, transport.authorized()).await {
            Ok(Ok(true)) => Ok(()),
            Ok(Ok(false)) => Err(not_authorized()),
            Ok(Err(TransportError::Delivery(error))) => Err(error),
            Ok(Err(TransportError::Failed)) | Err(_) => Err(unavailable()),
        };
        if let Err(error) = authorized {
            close(transport.as_ref()).await;
            return Err(error);
        }
        Ok(transport)
    }
}

/// Closing must never replace a result, so its failures are ignored.
async fn close(transport: &dyn TelegramTransport) {
    let _ = timeout(CLOSE_TIMEOUT, transport.close()).await;
}

#[async_trait]
impl ChannelAdapter for TelegramAdapter {
    async fn send(
        &self,
        recipient: &Recipient,
        text: &str,
        plan_id: &str,
    ) -> Result<Receipt, DeliveryError> {
        let Recipient::Telegram { target, .. } = recipient else {
            return Err(DeliveryError::new(
                "CHANNEL_MISMATCH",
                "Wrong adapter for recipient.",
                Outcome::NotSent,
            ));
        };
        let transport = self.open().await?;
        let random_id = telegram_random_id(plan_id);
        let result = match timeout(self.timeout, transport.send(target, text, random_id)).await {
            Ok(Ok(receipt)) => Ok(receipt),
            Ok(Err(TransportError::Delivery(error))) => Err(error),
            Ok(Err(TransportError::Failed)) | Err(_) => Err(uncertain()),
        };
        close(transport.as_ref()).await;
        result
    }
}

#[async_trait]
impl ContactDirectory for TelegramAdapter {
    async fn search(&self, query: &str, limit: i32) -> Result<Vec<Contact>, Error> {
        let failed = || {
            Error::new(
                "TELEGRAM_SEARCH_FAILED",
                "Telegram search failed. Check the session; no message was sent.",
            )
        };
        let transport = self
            .open()
            .await
            .map_err(|error| Error::new(error.code, error.message))?;
        let result = timeout(self.timeout, transport.search(query, limit)).await;
        close(transport.as_ref()).await;
        result.map_err(|_| failed())?.map_err(|_| failed())
    }
}

/// What is kept in the session file: the authorization keys and the home datacenter.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoredSession {
    pub home_dc: i32,
    pub dc_options: Vec<DcOption>,
}

impl StoredSession {
    pub fn into_memory_session(self) -> MemorySession {
        let mut data = SessionData {
            home_dc: self.home_dc,
            ..SessionData::default()
        };
        for option in self.dc_options {
            data.dc_options.insert(option.id, option);
        }
        MemorySession::from(data)
    }
}

pub struct Credentials {
    pub api_id: i32,
    pub api_hash: String,
}

/// Reads the application credentials, checking their format without showing them.
pub fn read_credentials(config: &TelegramConfig, env: Env<'_>) -> Result<Credentials, Error> {
    let api_id = env(TELEGRAM_API_ID_ENV)
        .and_then(|value| value.trim().parse::<i32>().ok())
        .or(config.api_id)
        .filter(|id| *id > 0);
    let api_hash = secret_from(env, TELEGRAM_API_HASH_ENV, config.api_hash_file.as_deref())?;
    let hash_is_valid = api_hash.len() == 32 && api_hash.bytes().all(|b| b.is_ascii_hexdigit());
    match api_id {
        Some(api_id) if hash_is_valid => Ok(Credentials { api_id, api_hash }),
        _ => Err(Error::new(
            "TELEGRAM_CREDENTIALS_INVALID",
            "Configure a valid Telegram apiId and API hash privately first.",
        )),
    }
}

pub fn input_peer(target: &TelegramPeer) -> Option<tl::enums::InputPeer> {
    Some(match target {
        TelegramPeer::Myself => tl::enums::InputPeer::PeerSelf,
        TelegramPeer::User { id, access_hash } => tl::types::InputPeerUser {
            user_id: id.parse().ok()?,
            access_hash: access_hash.parse().ok()?,
        }
        .into(),
        TelegramPeer::Chat { id } => tl::types::InputPeerChat {
            chat_id: id.parse().ok()?,
        }
        .into(),
        TelegramPeer::Channel { id, access_hash } => tl::types::InputPeerChannel {
            channel_id: id.parse().ok()?,
            access_hash: access_hash.parse().ok()?,
        }
        .into(),
    })
}

/// The exact request sent: literal text, no link preview, and never any paid delivery.
pub fn send_request(
    peer: tl::enums::InputPeer,
    text: &str,
    random_id: i64,
) -> tl::functions::messages::SendMessage {
    tl::functions::messages::SendMessage {
        no_webpage: true,
        silent: false,
        background: false,
        clear_draft: false,
        noforwards: false,
        update_stickersets_order: false,
        invert_media: false,
        allow_paid_floodskip: false,
        peer,
        reply_to: None,
        message: text.to_string(),
        random_id,
        reply_markup: None,
        entities: None,
        schedule_date: None,
        schedule_repeat_period: None,
        send_as: None,
        quick_reply_shortcut: None,
        effect: None,
        allow_paid_stars: None,
        suggested_post: None,
        rich_message: None,
    }
}

/// Takes the message ID only from the update matching our `random_id`, never from another one.
pub fn receipt_from(updates: &tl::enums::Updates, random_id: i64) -> Receipt {
    use tl::enums::Update;
    let find = |updates: &[Update]| {
        updates.iter().find_map(|update| match update {
            Update::MessageId(update) if update.random_id == random_id => {
                Some(update.id.to_string())
            }
            _ => None,
        })
    };
    let provider_message_id = match updates {
        tl::enums::Updates::UpdateShortSentMessage(sent) => Some(sent.id.to_string()),
        tl::enums::Updates::Updates(all) => find(&all.updates),
        tl::enums::Updates::Combined(all) => find(&all.updates),
        _ => None,
    };
    Receipt {
        accepted: true,
        provider_message_id,
    }
}

/// Keeps a display name printable and short. Names come from strangers: they are data.
pub fn clean_name(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_control())
        .take(100)
        .collect::<String>()
        .trim()
        .to_string()
}

/// The parts of a Telegram user that decide whether it can be offered as a contact.
pub struct UserFacts<'a> {
    pub id: i64,
    pub access_hash: Option<i64>,
    pub first_name: Option<&'a str>,
    pub last_name: Option<&'a str>,
    pub username: Option<&'a str>,
    pub is_self: bool,
    pub deleted: bool,
    pub bot: bool,
    pub min: bool,
}

/// A person the owner can message: not themselves, not deleted, not a bot, with a full hash.
pub fn user_contact(user: &UserFacts<'_>) -> Option<Contact> {
    if user.is_self || user.deleted || user.bot || user.min {
        return None;
    }
    let access_hash = user.access_hash?;
    let name = [user.first_name, user.last_name]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let id = user.id.to_string();
    Some(contact(
        "user",
        id.clone(),
        &name.join(" "),
        user.username.map(str::to_string),
        TelegramPeer::User {
            id,
            access_hash: access_hash.to_string(),
        },
    ))
}

fn facts(user: &tl::types::User) -> UserFacts<'_> {
    UserFacts {
        id: user.id,
        access_hash: user.access_hash,
        first_name: user.first_name.as_deref(),
        last_name: user.last_name.as_deref(),
        username: user.username.as_deref(),
        is_self: user.is_self,
        deleted: user.deleted,
        bot: user.bot,
        min: user.min,
    }
}

fn contact(
    kind: &'static str,
    id: String,
    name: &str,
    username: Option<String>,
    peer: TelegramPeer,
) -> Contact {
    let cleaned = clean_name(name);
    Contact {
        channel: Channel::Telegram,
        kind,
        reference: format!("{kind}:{id}"),
        name: if cleaned.is_empty() {
            format!("Telegram {kind} {id}")
        } else {
            cleaned
        },
        username,
        recipient_id: None,
        peer,
    }
}

pub fn chat_contact(chat: &tl::enums::Chat) -> Option<Contact> {
    match chat {
        tl::enums::Chat::Chat(chat) if !chat.left && !chat.deactivated => {
            let id = chat.id.to_string();
            Some(contact(
                "chat",
                id.clone(),
                &chat.title,
                None,
                TelegramPeer::Chat { id },
            ))
        }
        tl::enums::Chat::Channel(channel) if !channel.left && !channel.min => {
            let id = channel.id.to_string();
            let access_hash = channel.access_hash?.to_string();
            Some(contact(
                "channel",
                id.clone(),
                &channel.title,
                channel.username.clone(),
                TelegramPeer::Channel { id, access_hash },
            ))
        }
        _ => None,
    }
}

/// Reads only `my_results`: the owner's own contacts and chats, never the global directory.
pub fn contacts_from_found(found: &tl::types::contacts::Found) -> Vec<Contact> {
    let find_user = |wanted: i64| {
        found.users.iter().find_map(|user| match user {
            tl::enums::User::User(user) if user.id == wanted => user_contact(&facts(user)),
            _ => None,
        })
    };
    let find_chat = |wanted: i64| {
        found
            .chats
            .iter()
            .filter(|chat| chat.id() == wanted)
            .find_map(chat_contact)
    };
    found
        .my_results
        .iter()
        .filter_map(|peer| match peer {
            tl::enums::Peer::User(peer) => find_user(peer.user_id),
            tl::enums::Peer::Chat(peer) => find_chat(peer.chat_id),
            tl::enums::Peer::Channel(peer) => find_chat(peer.channel_id),
        })
        .collect()
}
