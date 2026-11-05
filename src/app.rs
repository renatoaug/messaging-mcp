use std::sync::Arc;

use crate::adapters::telegram::TelegramAdapter;
use crate::adapters::whatsapp::WhatsAppAdapter;
use crate::config::Config;
use crate::error::Error;
use crate::service::{Adapters, MessagingService};
use crate::store::PlanStore;

/// Wires the real adapters. Creating them contacts nobody: connections open only on a live send.
pub fn create_app(config: Config) -> Result<MessagingService, Error> {
    let store = Arc::new(PlanStore::open(&config.state_dir)?);
    let mut adapters = Adapters::default();
    if let Some(settings) = &config.whatsapp {
        adapters.whatsapp = Some(Arc::new(WhatsAppAdapter::new(settings.clone())?));
    }
    let telegram = config
        .telegram
        .clone()
        .map(|settings| Arc::new(TelegramAdapter::new(settings)));
    if let Some(telegram) = &telegram {
        adapters.telegram = Some(telegram.clone());
    }
    let service = MessagingService::new(config, store, adapters);
    Ok(match telegram {
        Some(telegram) => service.with_directory(telegram),
        None => service,
    })
}
