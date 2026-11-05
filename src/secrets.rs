use std::fs::OpenOptions;
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

use rustix::fs::OFlags;
use rustix::process::geteuid;

use crate::error::Error;

const MAX_SECRET_BYTES: u64 = 16_384;

/// True when the mode gives nothing to group or others and the owner is this process's user.
pub fn is_owner_only(metadata: &std::fs::Metadata) -> bool {
    metadata.mode() & 0o077 == 0 && metadata.uid() == geteuid().as_raw()
}

fn read_private_file(path: &Path) -> Option<String> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(OFlags::NOFOLLOW.bits() as i32)
        .open(path)
        .ok()?;
    let metadata = file.metadata().ok()?;
    let is_safe = metadata.is_file()
        && metadata.len() > 0
        && metadata.len() <= MAX_SECRET_BYTES
        && is_owner_only(&metadata);
    if !is_safe {
        return None;
    }
    let mut secret = String::new();
    file.read_to_string(&mut secret).ok()?;
    let secret = secret.trim();
    (!secret.is_empty()).then(|| secret.to_string())
}

/// Reads a secret from a private regular file, without ever exposing its contents in an error.
pub fn read_secret_file(path: &Path) -> Result<String, Error> {
    read_private_file(path).ok_or_else(|| {
        Error::new(
            "SECRET_UNAVAILABLE",
            "Secret file is missing, empty, linked, or not owner-only (0600). Do not paste \
             secrets into agent chat.",
        )
    })
}

/// Looks up an environment variable. Injected, so tests never read inherited credentials.
pub type Env<'a> = &'a (dyn Fn(&str) -> Option<String> + Send + Sync);

/// The real process environment.
pub fn process_env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// Reads a secret from the environment, then from its private file.
pub fn secret_from(env: Env<'_>, env_name: &str, file: Option<&Path>) -> Result<String, Error> {
    if let Some(value) = env(env_name) {
        let value = value.trim();
        if !value.is_empty() {
            return Ok(value.to_string());
        }
    }
    match file {
        Some(path) => read_secret_file(path),
        None => Err(Error::new(
            "SECRET_UNAVAILABLE",
            format!("Configure {env_name} or its private secret file before enabling live mode."),
        )),
    }
}
