//! Local MCP server and CLI that send messages through personal WhatsApp and Telegram accounts.
//!
//! Every entry point goes through [`service::MessagingService`] and the durable
//! [`store::PlanStore`]. Dry-run is the default.

pub mod adapters;
pub mod app;
pub mod cli;
pub mod config;
pub mod error;
pub mod identity;
pub mod mcp;
pub mod plan;
pub mod secrets;
pub mod service;
pub mod store;
