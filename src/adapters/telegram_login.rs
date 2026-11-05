//! Manual session setup. Deliberately outside MCP: a person runs it after deciding to create
//! persistent account access.

use std::fs::{DirBuilder, OpenOptions};
use std::io::{IsTerminal, Write, stdin, stdout};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;
use std::sync::Arc;

use grammers_client::{Client, SignInError};
use grammers_session::Session;
use grammers_session::storages::MemorySession;
use rustix::fs::OFlags;

use crate::adapters::telegram::{StoredSession, read_credentials};
use crate::adapters::telegram_client::single_attempt_pool;
use crate::config::Config;
use crate::error::Error;
use crate::secrets::{is_owner_only, process_env};

const INTRODUCTION: &str = "This creates and saves persistent access to your PERSONAL Telegram \
account.\nThe session can read and send as you; keep it private and revoke it in Telegram \
Settings > Devices when no longer needed.\nThe login is subject to the Telegram API terms: \
https://core.telegram.org/api/terms\nNo messages will be sent. No new Telegram account will be \
created.";

/// The known datacenters. Their authorization keys are what the session file preserves.
const DATACENTERS: std::ops::RangeInclusive<i32> = 1..=5;

fn setup_error(message: &str) -> Error {
    Error::new("TELEGRAM_LOGIN_FAILED", message)
}

fn failed() -> Error {
    setup_error(
        "Setup failed. No secrets are displayed. Review your Telegram Devices before attempting \
         a new session.",
    )
}

fn ask(prompt: &str) -> Result<String, Error> {
    print!("{prompt}");
    stdout().flush().map_err(|_| failed())?;
    let mut answer = String::new();
    stdin().read_line(&mut answer).map_err(|_| failed())?;
    Ok(answer.trim_end_matches(['\r', '\n']).to_string())
}

fn ask_hidden(prompt: &str) -> Result<String, Error> {
    rpassword::prompt_password(prompt).map_err(|_| failed())
}

fn prepare_directory(session_file: &Path) -> Result<(), Error> {
    let message = "Session directory must be an owner-only, non-symlink directory (0700).";
    let directory = session_file.parent().ok_or_else(|| setup_error(message))?;
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)
        .map_err(|_| setup_error(message))?;
    let metadata = directory
        .symlink_metadata()
        .map_err(|_| setup_error(message))?;
    if !metadata.is_dir() || !is_owner_only(&metadata) {
        return Err(setup_error(message));
    }
    Ok(())
}

fn save_session(session: &MemorySession, path: &Path) -> Result<(), Error> {
    let dc_options = DATACENTERS
        .filter_map(|id| session.dc_option(id).ok().flatten())
        .filter(|option| option.auth_key.is_some())
        .collect();
    let home_dc = session.home_dc_id().map_err(|_| failed())?;
    let stored = StoredSession {
        home_dc,
        dc_options,
    };
    let json = serde_json::to_string(&stored).map_err(|_| failed())?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(OFlags::NOFOLLOW.bits() as i32)
        .open(path)
        .map_err(|_| failed())?;
    file.write_all(json.as_bytes()).map_err(|_| failed())?;
    file.write_all(b"\n").map_err(|_| failed())?;
    file.sync_all().map_err(|_| failed())
}

async fn sign_in(client: &Client, api_hash: &str) -> Result<grammers_client::peer::User, Error> {
    let phone = ask_hidden("Your phone number with country code (hidden): ")?;
    let token = client
        .request_login_code(phone.trim(), api_hash)
        .await
        .map_err(|_| failed())?;
    let code = ask_hidden("Matching Telegram login code (hidden): ")?;
    match client.sign_in(&token, code.trim()).await {
        Ok(user) => Ok(user),
        Err(SignInError::SignUpRequired) => {
            Err(setup_error("Creating a new account is not supported."))
        }
        Err(SignInError::PasswordRequired(password_token)) => {
            // The password keeps its exact characters: spaces can be part of it.
            let password = ask_hidden("Your Telegram 2FA password (hidden): ")?;
            client
                .check_password(password_token, password)
                .await
                .map_err(|_| failed())
        }
        Err(_) => Err(failed()),
    }
}

/// Runs the interactive login and writes the session file. It never overwrites one.
pub async fn run(config: &Config) -> Result<(), Error> {
    if !stdin().is_terminal() || !stdout().is_terminal() {
        return Err(Error::new(
            "TTY_REQUIRED",
            "Run this setup yourself in an interactive local terminal.",
        ));
    }
    let settings = config
        .telegram
        .as_ref()
        .ok_or_else(|| setup_error("Configure the telegram section first."))?;
    let session_file = settings
        .session_file
        .as_deref()
        .ok_or_else(|| setup_error("Set telegram.sessionFile to a private path outside Git."))?;
    if session_file.symlink_metadata().is_ok() {
        return Err(setup_error(
            "Session file already exists; this will never overwrite it.",
        ));
    }
    let credentials = read_credentials(settings, &process_env)?;
    println!("{INTRODUCTION}");
    if ask("Type CREATE SESSION to proceed: ")? != "CREATE SESSION" {
        return Err(setup_error("Cancelled before connecting."));
    }
    prepare_directory(session_file)?;

    let session = Arc::new(MemorySession::default());
    let pool = single_attempt_pool(Arc::clone(&session), credentials.api_id);
    let handle = pool.handle.thin.clone();
    let client = Client::new(pool.handle);
    let runner = tokio::spawn(pool.runner.run());
    let result = sign_in(&client, &credentials.api_hash).await;
    let outcome = result.and_then(|user| {
        if user.is_bot() {
            return Err(setup_error("Bot sessions are not supported."));
        }
        let id = user.id().bare_id().ok_or_else(failed)?.to_string();
        if settings
            .expected_user_id
            .as_deref()
            .is_some_and(|expected| expected != id)
        {
            return Err(setup_error(
                "The logged-in account differs from expectedUserId. No session saved; review and \
                 revoke this new session in Telegram Settings > Devices.",
            ));
        }
        save_session(&session, session_file)?;
        println!(
            "Session saved privately. Set telegram.expectedUserId to \"{id}\".\nKeep mode=dry-run \
             until you intentionally approve a real test."
        );
        Ok(())
    });
    handle.quit();
    runner.abort();
    outcome
}
