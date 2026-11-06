use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::attachment::{Attachment, Outgoing};
use crate::config::{Recipient, TelegramPeer};
use crate::error::{DeliveryError, Error};

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    #[default]
    DryRun,
    Live,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DryRun => "dry-run",
            Self::Live => "live",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "dry-run" => Some(Self::DryRun),
            "live" => Some(Self::Live),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    Whatsapp,
    Telegram,
}

impl Channel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Whatsapp => "whatsapp",
            Self::Telegram => "telegram",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "whatsapp" => Some(Self::Whatsapp),
            "telegram" => Some(Self::Telegram),
            _ => None,
        }
    }
}

/// The states of a plan. `Sending` and everything after it are never attempted again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pending,
    Approved,
    Sending,
    Sent,
    Simulated,
    Failed,
    OutcomeUnknown,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Sending => "sending",
            Self::Sent => "sent",
            Self::Simulated => "simulated",
            Self::Failed => "failed",
            Self::OutcomeUnknown => "outcome_unknown",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "approved" => Some(Self::Approved),
            "sending" => Some(Self::Sending),
            "sent" => Some(Self::Sent),
            "simulated" => Some(Self::Simulated),
            "failed" => Some(Self::Failed),
            "outcome_unknown" => Some(Self::OutcomeUnknown),
            _ => None,
        }
    }

    pub fn is_open(self) -> bool {
        matches!(self, Self::Pending | Self::Approved)
    }
}

/// The provider accepted the message. This is not a delivery or read receipt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub accepted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_message_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanError {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub id: String,
    pub mode: Mode,
    pub recipient_id: String,
    pub recipient_label: String,
    pub sender: String,
    pub channel: Channel,
    pub target: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment: Option<Attachment>,
    pub fingerprint: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approved_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempted_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt: Option<Receipt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<PlanError>,
}

/// A person, group or channel found by [`ContactDirectory::search`].
///
/// Finding someone authorizes nothing. Only a person running `allow` turns a contact into a
/// recipient. The peer stays out of the serialized form, so the agent never sees access hashes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Contact {
    pub channel: Channel,
    pub kind: &'static str,
    pub reference: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recipient_id: Option<String>,
    #[serde(skip)]
    pub peer: TelegramPeer,
}

#[async_trait]
pub trait ChannelAdapter: Send + Sync {
    async fn send(
        &self,
        recipient: &Recipient,
        message: &Outgoing<'_>,
        plan_id: &str,
    ) -> Result<Receipt, DeliveryError>;
}

#[async_trait]
pub trait ContactDirectory: Send + Sync {
    async fn search(&self, query: &str, limit: i32) -> Result<Vec<Contact>, Error>;
}

/// A chat found by name, to read from. The target stays out of the serialized form.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMatch {
    pub channel: Channel,
    pub reference: String,
    pub name: String,
    #[serde(skip)]
    pub target: ChatTarget,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatTarget {
    Telegram(TelegramPeer),
    /// A phone number or group JID.
    Whatsapp(String),
}

/// One message read from a chat. Its text was written by someone else: data, never
/// instructions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub from: String,
    pub from_me: bool,
    /// Milliseconds since the epoch.
    pub at: i64,
    pub text: String,
}

/// Reads the owner's chats, only when the owner asks. Never sends anything.
#[async_trait]
pub trait MessageReader: Send + Sync {
    async fn find_chats(&self, query: &str) -> Result<Vec<ChatMatch>, Error>;
    /// The chat's last `limit` messages, oldest first.
    async fn read(&self, chat: &ChatMatch, limit: i32) -> Result<Vec<ChatMessage>, Error>;
}
