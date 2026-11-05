use std::fs;
use std::io::{IsTerminal, Write, stderr, stdin, stdout};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use chrono_free::iso_utc;
use clap::{Args, Parser, Subcommand};
use rmcp::ServiceExt;
use serde::Serialize;
use serde_json::json;

use crate::adapters::telegram_login;
use crate::app::create_app;
use crate::config::{Recipient, load_config};
use crate::error::Error;
use crate::mcp::{MessagingServer, status_report};
use crate::plan::Status;
use crate::service::{MessagingService, PrepareInput};

/// Dependency-free timestamp formatting for the approval prompt.
mod chrono_free {
    /// Formats milliseconds since the epoch as an ISO 8601 UTC timestamp.
    pub fn iso_utc(milliseconds: i64) -> String {
        let seconds = milliseconds.div_euclid(1000);
        let (days, second_of_day) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
        // Howard Hinnant's civil_from_days.
        let shifted = days + 719_468;
        let era = shifted.div_euclid(146_097);
        let day_of_era = shifted.rem_euclid(146_097);
        let year_of_era =
            (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let shifted_month = (5 * day_of_year + 2) / 153;
        let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
        let month = if shifted_month < 10 {
            shifted_month + 3
        } else {
            shifted_month - 9
        };
        let year = year_of_era + era * 400 + i64::from(month <= 2);
        format!(
            "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
            second_of_day / 3600,
            second_of_day % 3600 / 60,
            second_of_day % 60
        )
    }
}

/// Local MCP server and CLI for personal WhatsApp and Telegram messaging.
///
/// Only telegram-login signs in, and only a person can run it. No command scans a QR code, starts
/// the WhatsApp bridge or changes client settings.
#[derive(Parser)]
#[command(name = "messaging-mcp", version)]
struct Cli {
    /// Config file. Defaults to MESSAGING_MCP_CONFIG, then ./messaging-mcp.config.json.
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// MCP server on stdin/stdout.
    Mcp,
    /// Local configuration check, no network.
    Doctor,
    /// List allowed recipients.
    Recipients,
    /// Prepare a message. Nothing is sent.
    Prepare(PrepareArgs),
    /// Read persisted state.
    Status { plan_id: String },
    /// Human-only interactive review of a plan.
    Approve { plan_id: String },
    /// Send an approved plan, or simulate it in dry-run.
    Send { plan_id: String },
    /// Human-only: add a recipient after reviewing it.
    #[command(subcommand)]
    Allow(AllowCommand),
    /// Remove a recipient added by allow.
    Revoke { recipient_id: String },
    /// Human-only: create the Telegram session file.
    TelegramLogin,
}

#[derive(Args)]
struct PrepareArgs {
    #[arg(long)]
    recipient: String,
    #[arg(long)]
    key: String,
    /// The message. Prefer --text-file for private text: this stays in the shell history.
    #[arg(
        long,
        conflicts_with = "text_file",
        required_unless_present = "text_file"
    )]
    text: Option<String>,
    #[arg(long)]
    text_file: Option<PathBuf>,
}

#[derive(Subcommand)]
enum AllowCommand {
    /// Allow an exact WhatsApp JID, such as 5511999999999@s.whatsapp.net.
    Whatsapp {
        jid: String,
        #[arg(long)]
        id: String,
        #[arg(long)]
        label: String,
    },
    /// Search your Telegram contacts and chats, then allow the one you pick. Needs live mode.
    Telegram {
        query: String,
        #[arg(long)]
        id: String,
        /// Defaults to the name on Telegram.
        #[arg(long)]
        label: Option<String>,
    },
}

fn print_json(value: &impl Serialize) -> Result<(), Error> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|_| Error::new("INTERNAL_ERROR", "Could not encode the result."))?;
    println!("{text}");
    Ok(())
}

fn require_terminal(purpose: &str) -> Result<(), Error> {
    if stdin().is_terminal() && stdout().is_terminal() {
        return Ok(());
    }
    Err(Error::new(
        "TTY_REQUIRED",
        format!("{purpose} requires a human in a local interactive terminal. It cannot be piped."),
    ))
}

fn ask(prompt: &str) -> Result<String, Error> {
    let failed = |_| Error::new("TTY_REQUIRED", "Could not read from the terminal.");
    print!("{prompt}");
    stdout().flush().map_err(failed)?;
    let mut answer = String::new();
    stdin().read_line(&mut answer).map_err(failed)?;
    Ok(answer.trim_end_matches(['\r', '\n']).to_string())
}

async fn serve_mcp(service: MessagingService) -> Result<(), Error> {
    let failed = |_| Error::new("MCP_FAILED", "The MCP connection failed.");
    let server = MessagingServer::new(Arc::new(service));
    let running = server
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|_| Error::new("MCP_FAILED", "The MCP connection could not start."))?;
    running.waiting().await.map_err(failed)?;
    Ok(())
}

fn prepare(service: &MessagingService, args: PrepareArgs) -> Result<(), Error> {
    let text = match args.text_file {
        Some(path) => fs::read_to_string(path)
            .map_err(|_| Error::new("TEXT_FILE_UNREADABLE", "Cannot read the text file."))?,
        None => args.text.unwrap_or_default(),
    };
    let input = PrepareInput {
        recipient_id: args.recipient,
        text,
        idempotency_key: args.key,
    };
    print_json(&service.prepare(&input)?)
}

fn approve(service: &MessagingService, id: &str) -> Result<(), Error> {
    require_terminal("Approval")?;
    let plan = service.status(id)?;
    // JSON encoding keeps message text from controlling the terminal with escape sequences.
    print_json(&json!({
        "review": "Verify account, exact destination, and complete text. Sent is not a delivery \
                   receipt.",
        "mode": plan.mode,
        "sender": plan.sender,
        "channel": plan.channel,
        "recipient": plan.recipient_label,
        "destination": plan.target,
        "message": plan.text,
        "expiresAt": iso_utc(plan.expires_at),
    }))?;
    let answer = ask(&format!(
        "Type APPROVE {id} to authorize this exact message: "
    ))?;
    if answer != format!("APPROVE {id}") {
        return Err(Error::new(
            "APPROVAL_CANCELLED",
            "Approval cancelled. No message was sent.",
        ));
    }
    print_json(&service.approve(id)?)
}

fn confirm_allow(service: &MessagingService, recipient: &Recipient) -> Result<(), Error> {
    recipient.validate()?;
    print_json(&json!({
        "review": "Agents will be able to prepare messages to this destination. Each live message \
                   still needs your approval.",
        "id": recipient.id(),
        "label": recipient.label(),
        "channel": recipient.channel(),
        "destination": recipient.display_target(),
    }))?;
    let answer = ask(&format!(
        "Type ALLOW {} to add this recipient: ",
        recipient.id()
    ))?;
    if answer != format!("ALLOW {}", recipient.id()) {
        return Err(Error::new(
            "ALLOW_CANCELLED",
            "Cancelled. No recipient was added.",
        ));
    }
    service.allow_recipient(recipient)?;
    print_json(&json!({ "allowed": recipient.id() }))
}

async fn allow(service: &MessagingService, command: AllowCommand) -> Result<(), Error> {
    require_terminal("Allowing a recipient")?;
    match command {
        AllowCommand::Whatsapp { jid, id, label } => confirm_allow(
            service,
            &Recipient::Whatsapp {
                id,
                label,
                target: jid,
            },
        ),
        AllowCommand::Telegram { query, id, label } => {
            let contacts = service.search_contacts(&query).await?;
            if contacts.is_empty() {
                return Err(Error::new("NO_CONTACTS", "No contact or chat matched."));
            }
            print_json(&contacts)?;
            let answer = ask(&format!(
                "Type the number of the result, 1 to {}: ",
                contacts.len()
            ))?;
            let contact = answer
                .trim()
                .parse::<usize>()
                .ok()
                .and_then(|number| number.checked_sub(1))
                .and_then(|index| contacts.get(index))
                .ok_or_else(|| Error::new("ALLOW_CANCELLED", "Cancelled. Nothing was added."))?;
            let label = label.unwrap_or_else(|| contact.name.clone());
            confirm_allow(
                service,
                &Recipient::Telegram {
                    id,
                    label,
                    target: contact.peer.clone(),
                },
            )
        }
    }
}

async fn run(cli: Cli) -> Result<ExitCode, Error> {
    let config = load_config(cli.config.as_deref())?;
    if matches!(cli.command, Command::TelegramLogin) {
        telegram_login::run(&config).await?;
        return Ok(ExitCode::SUCCESS);
    }
    let service = create_app(config)?;
    match cli.command {
        Command::Mcp => serve_mcp(service).await?,
        Command::Doctor => print_json(&status_report(&service))?,
        Command::Recipients => print_json(&service.list_recipients()?)?,
        Command::Prepare(args) => prepare(&service, args)?,
        Command::Status { plan_id } => print_json(&service.status(&plan_id)?)?,
        Command::Approve { plan_id } => approve(&service, &plan_id)?,
        Command::Send { plan_id } => {
            let plan = service.send(&plan_id).await?;
            print_json(&plan)?;
            let uncertain = matches!(
                plan.status,
                Status::Failed | Status::OutcomeUnknown | Status::Sending
            );
            if uncertain {
                return Ok(ExitCode::from(2));
            }
        }
        Command::Allow(command) => allow(&service, command).await?,
        Command::Revoke { recipient_id } => {
            service.revoke_recipient(&recipient_id)?;
            print_json(&json!({ "revoked": recipient_id }))?;
        }
        Command::TelegramLogin => {}
    }
    Ok(ExitCode::SUCCESS)
}

pub async fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli).await {
        Ok(code) => code,
        Err(error) => {
            let line = json!({ "code": error.code, "message": error.message });
            // Nothing useful remains to do if stderr itself is closed.
            let _ = writeln!(stderr(), "{line}");
            ExitCode::FAILURE
        }
    }
}
