use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    messaging_mcp::cli::main().await
}
