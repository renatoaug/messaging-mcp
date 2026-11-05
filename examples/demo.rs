//! Prepares and simulates one message per channel in a throwaway state directory.
//! Run with `cargo run --example demo`. No account is contacted.

use std::path::Path;
use std::process::ExitCode;

use messaging_mcp::app::create_app;
use messaging_mcp::config::load_config;
use messaging_mcp::error::Error;
use messaging_mcp::service::PrepareInput;
use serde_json::json;

async fn run() -> Result<(), Error> {
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/messaging-mcp.config.json");
    let mut config = load_config(Some(&example))?;
    let state = tempfile::tempdir()
        .map_err(|_| Error::new("DEMO_FAILED", "Could not create a temporary directory."))?;
    config.state_dir = state.path().join("state");
    let service = create_app(config)?;
    for recipient_id in ["wa-demo", "tg-self"] {
        let plan = service.prepare(&PrepareInput {
            recipient_id: recipient_id.to_string(),
            text: "One project, two channels. No real message was sent.".to_string(),
            idempotency_key: format!("demo_{recipient_id}_001"),
        })?;
        let result = service.send(&plan.id).await?;
        let line = json!({
            "channel": result.channel,
            "status": result.status,
            "mode": result.mode,
            "target": result.target,
        });
        println!("{line}");
    }
    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
