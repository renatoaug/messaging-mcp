//! Files other people sent, saved only when the owner asks. The name comes from a stranger, so it
//! is reduced to a plain file name that cannot leave the folder, hide itself or replace a file.

use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use rustix::fs::OFlags;

use crate::error::Error;
use crate::plan::ReceivedFile;
use crate::secrets::Env;
use crate::setup::config_file::ensure_private_dir;

/// Where saved files go: `~/Downloads/messaging-mcp/`, or `None` without a home folder.
pub fn default_dir(env: Env<'_>) -> Option<PathBuf> {
    env("HOME")
        .filter(|home| !home.is_empty())
        .map(|home| Path::new(&home).join("Downloads").join("messaging-mcp"))
}

/// Direction overrides and zero-width characters, which can disguise an extension
/// ("invoice\u{202e}fdp.exe" shows as "invoiceexe.pdf").
fn is_invisible(c: char) -> bool {
    matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}')
}

/// A plain, visible file name of at most 120 characters, keeping the extension.
pub fn safe_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_control() || is_invisible(c) || c == ':' {
                '_'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_start_matches(['.', ' ']);
    let (stem, extension) = match cleaned.rsplit_once('.') {
        Some((stem, extension))
            if !stem.is_empty()
                && (1..=10).contains(&extension.chars().count())
                && extension.chars().all(char::is_alphanumeric) =>
        {
            (stem, format!(".{extension}"))
        }
        _ => (cleaned, String::new()),
    };
    let stem: String = stem.chars().take(120 - extension.chars().count()).collect();
    let stem = stem.trim_end();
    if stem.is_empty() {
        format!("attachment{extension}")
    } else {
        format!("{stem}{extension}")
    }
}

fn save_failed() -> Error {
    Error::new(
        "SAVE_FAILED",
        "The file could not be written to the downloads folder.",
    )
}

/// Writes the file into `dir` (created 0700) as a new 0600 file, never replacing one: a name
/// already taken gets ` (2)`, ` (3)`... Returns the path.
pub fn save(dir: &Path, file: &ReceivedFile) -> Result<PathBuf, Error> {
    ensure_private_dir(dir)?;
    let name = safe_name(&file.name);
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, format!(".{extension}")),
        _ => (name.as_str(), String::new()),
    };
    for n in 1..=100 {
        let candidate = if n == 1 {
            dir.join(&name)
        } else {
            dir.join(format!("{stem} ({n}){extension}"))
        };
        let opened = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(OFlags::NOFOLLOW.bits() as i32)
            .open(&candidate);
        match opened {
            Ok(mut out) => {
                let written = out.write_all(&file.data).and_then(|()| out.sync_all());
                if written.is_err() {
                    let _ = std::fs::remove_file(&candidate);
                    return Err(save_failed());
                }
                return Ok(candidate);
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(_) => return Err(save_failed()),
        }
    }
    Err(save_failed())
}
