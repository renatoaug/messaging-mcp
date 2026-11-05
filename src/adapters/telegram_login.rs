//! Telegram login steps that need a person at a terminal: the phone-code login
//! (`telegram-login`, an alternative to the agent's QR login) and the 2FA password that finishes
//! a QR login (`telegram-password`). The password is typed hidden and never passes through an
//! agent.

use std::io::{IsTerminal, Write, stdin, stdout};
use std::path::Path;
use std::sync::Arc;

use grammers_client::client::PasswordToken;
use grammers_client::{Client, SignInError};
use grammers_session::storages::MemorySession;
use grammers_tl_types as tl;
use serde_json::json;

use crate::adapters::telegram::{StoredSession, read_credentials, write_session};
use crate::adapters::telegram_client::single_attempt_pool;
use crate::config::{Config, TelegramConfig};
use crate::error::Error;
use crate::secrets::{process_env, read_secret_file};
use crate::setup::config_file::{ensure_private_dir, section, update_config};

const INTRODUCTION: &str = "This creates and saves persistent access to your PERSONAL Telegram \
account.\nThe session can read and send as you; keep it private and revoke it in Telegram \
Settings > Devices when no longer needed.\nThe login is subject to the Telegram API terms: \
https://core.telegram.org/api/terms\nNo messages will be sent. No new Telegram account will be \
created.";

fn setup_error(message: &str) -> Error {
    Error::new("TELEGRAM_LOGIN_FAILED", message)
}

fn failed() -> Error {
    setup_error(
        "Setup failed. No secrets are displayed. Review your Telegram Devices before attempting \
         a new session.",
    )
}

fn require_terminal() -> Result<(), Error> {
    if stdin().is_terminal() && stdout().is_terminal() {
        Ok(())
    } else {
        Err(Error::new(
            "TTY_REQUIRED",
            "Run this yourself in an interactive local terminal.",
        ))
    }
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

fn settings(config: &Config) -> Result<&TelegramConfig, Error> {
    config
        .telegram
        .as_ref()
        .ok_or_else(|| setup_error("Configure the telegram section first."))
}

fn session_file(settings: &TelegramConfig) -> Result<&Path, Error> {
    settings
        .session_file
        .as_deref()
        .ok_or_else(|| setup_error("Set telegram.sessionFile to a private path outside Git."))
}

/// Refuses bots and a different user than the one already expected, then records the user.
fn accept_user(
    user: &grammers_client::peer::User,
    settings: &TelegramConfig,
    session: &MemorySession,
    config_path: &Path,
) -> Result<String, Error> {
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
    let session_path = session_file(settings)?;
    write_session(session, session_path)?;
    update_config(config_path, |config| {
        let telegram = section(config, "telegram");
        telegram.insert("expectedUserId".into(), json!(id));
    })?;
    Ok(id)
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
            check_password(client, password_token).await
        }
        Err(_) => Err(failed()),
    }
}

async fn check_password(
    client: &Client,
    token: PasswordToken,
) -> Result<grammers_client::peer::User, Error> {
    // The password keeps its exact characters: spaces can be part of it.
    let password = ask_hidden("Your Telegram 2FA password (hidden): ")?;
    client
        .check_password(token, password)
        .await
        .map_err(|_| setup_error("The password was not accepted. Nothing was saved."))
}

/// The phone-code login. Writes the session file; never overwrites one.
pub async fn run(config: &Config, config_path: &Path) -> Result<(), Error> {
    require_terminal()?;
    let settings = settings(config)?;
    let session_path = session_file(settings)?;
    if session_path.symlink_metadata().is_ok() {
        return Err(setup_error(
            "Session file already exists; this will never overwrite it.",
        ));
    }
    let credentials = read_credentials(settings, &process_env)?;
    println!("{INTRODUCTION}");
    if ask("Type CREATE SESSION to proceed: ")? != "CREATE SESSION" {
        return Err(setup_error("Cancelled before connecting."));
    }
    ensure_private_dir(session_path.parent().ok_or_else(failed)?)?;
    let session = Arc::new(MemorySession::default());
    let pool = single_attempt_pool(Arc::clone(&session), credentials.api_id);
    let handle = pool.handle.thin.clone();
    let client = Client::new(pool.handle);
    let runner = tokio::spawn(pool.runner.run());
    let outcome = match sign_in(&client, &credentials.api_hash).await {
        Ok(user) => accept_user(&user, settings, &session, config_path),
        Err(error) => Err(error),
    };
    handle.quit();
    runner.abort();
    let id = outcome?;
    println!("Logged in as Telegram user {id}. The session is saved privately.");
    Ok(())
}

/// Finishes a QR login that stopped at the 2FA password.
pub async fn finish_with_password(config: &Config, config_path: &Path) -> Result<(), Error> {
    require_terminal()?;
    let settings = settings(config)?;
    let pending_path = config_path
        .parent()
        .ok_or_else(failed)?
        .join("telegram-pending-session.json");
    let pending = read_secret_file(&pending_path).map_err(|_| {
        setup_error("No login is waiting for a password. Ask the agent to run telegram_login.")
    })?;
    let stored: StoredSession = serde_json::from_str(&pending).map_err(|_| failed())?;
    let credentials = read_credentials(settings, &process_env)?;
    let session = Arc::new(stored.into_memory_session());
    let pool = single_attempt_pool(Arc::clone(&session), credentials.api_id);
    let handle = pool.handle.thin.clone();
    let client = Client::new(pool.handle);
    let runner = tokio::spawn(pool.runner.run());
    let outcome = async {
        let tl::enums::account::Password::Password(password) = client
            .invoke(&tl::functions::account::GetPassword {})
            .await
            .map_err(|_| failed())?;
        let user = check_password(&client, PasswordToken::new(password)).await?;
        accept_user(&user, settings, &session, config_path)
    }
    .await;
    handle.quit();
    runner.abort();
    let id = outcome?;
    let _ = std::fs::remove_file(&pending_path);
    println!("Logged in as Telegram user {id}. The session is saved privately.");
    Ok(())
}
