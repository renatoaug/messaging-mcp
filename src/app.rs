use std::sync::Arc;

use crate::adapters::telegram::TelegramAdapter;
use crate::adapters::whatsapp::WhatsAppAdapter;
use crate::config::Config;
use crate::error::Error;
use crate::secrets::process_env;
use crate::service::{Adapters, MessagingService, Readers};
use crate::store::PlanStore;
use crate::transcribe::Transcriber;

/// Wires the real adapters. Creating them contacts nobody: connections open only on a live send.
pub fn create_app(config: Config) -> Result<MessagingService, Error> {
    let store = Arc::new(PlanStore::open(&config.state_dir)?);
    let mut adapters = Adapters::default();
    let mut readers = Readers::default();
    if let Some(settings) = &config.whatsapp {
        let whatsapp = Arc::new(WhatsAppAdapter::new(settings.clone())?);
        adapters.whatsapp = Some(whatsapp.clone());
        readers.whatsapp = Some(whatsapp);
    }
    let telegram = config.telegram.clone().map(|settings| {
        let transcriber = Transcriber::find(&config, &process_env);
        Arc::new(TelegramAdapter::new(settings).with_transcriber(transcriber))
    });
    if let Some(telegram) = &telegram {
        adapters.telegram = Some(telegram.clone());
        readers.telegram = Some(telegram.clone());
    }
    let service = MessagingService::new(config, store, adapters).with_readers(readers);
    Ok(match telegram {
        Some(telegram) => service.with_directory(telegram),
        None => service,
    })
}
