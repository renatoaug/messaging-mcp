use std::fmt;

use serde::Serialize;

/// An error that is safe to show to a client: a stable code and a message we wrote ourselves.
///
/// Never build one from a provider or SDK error. Those can carry URLs, tokens and private data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Error {
    pub code: &'static str,
    pub message: String,
}

impl Error {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Error {}

/// What is known about a send that did not succeed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Rejected before the provider could have accepted the message.
    NotSent,
    /// The provider may have accepted it. Only a person looking at the chat can tell.
    Unknown,
}

/// A failed send, with what is known about whether the message left.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeliveryError {
    pub code: &'static str,
    pub message: String,
    pub outcome: Outcome,
}

impl DeliveryError {
    pub fn new(code: &'static str, message: impl Into<String>, outcome: Outcome) -> Self {
        Self {
            code,
            message: message.into(),
            outcome,
        }
    }
}
