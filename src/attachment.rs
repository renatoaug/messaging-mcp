//! Files sent with a message. A file is checked and copied into the state directory when the
//! message is prepared, so what the owner reviews (name, size, SHA-256) is exactly what leaves,
//! even if the original changes or disappears before the send.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{DeliveryError, Error, Outcome};
use crate::service::sha256_hex;
use crate::setup::config_file::{ensure_private_dir, write_private_file};

/// The largest file sent. The WhatsApp bridge checks the same limit.
pub const MAX_FILE_BYTES: u64 = 50 * 1024 * 1024;

/// Telegram sends larger photos as documents only, so a bigger image goes as a document.
const MAX_PHOTO_BYTES: u64 = 10 * 1024 * 1024;

/// How the file is shown to the recipient.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A photo, shown inline: JPEG or PNG, by content, up to 10 MB.
    Image,
    /// A file the recipient downloads, never recompressed.
    Document,
}

/// A file attached to a plan, as the owner reviews it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    /// The path the agent gave, after resolving links.
    pub path: String,
    /// The name the recipient sees.
    pub name: String,
    pub size: u64,
    pub mime: String,
    pub kind: Kind,
    pub sha256: String,
}

/// A file ready for an adapter.
pub struct OutgoingFile<'a> {
    pub name: &'a str,
    pub mime: &'a str,
    pub kind: Kind,
    pub data: &'a [u8],
}

/// What an adapter sends: text alone, or a file with the text as its caption.
pub struct Outgoing<'a> {
    pub text: &'a str,
    pub file: Option<OutgoingFile<'a>>,
}

fn refused(message: &str) -> Error {
    Error::new("FILE_NOT_ALLOWED", message)
}

fn unreadable() -> Error {
    Error::new(
        "FILE_UNREADABLE",
        "The file could not be read. Give the absolute path of a regular file.",
    )
}

/// Refuses any path inside a hidden folder or naming a hidden file: that covers ~/.ssh,
/// ~/.config (where messaging-mcp keeps its sessions and tokens), .env files and .git, which an
/// agent could otherwise be talked into sending.
fn is_hidden(path: &Path) -> bool {
    path.components().any(|component| match component {
        Component::Normal(name) => name.to_string_lossy().starts_with('.'),
        _ => false,
    })
}

fn mime_for(name: &str) -> &'static str {
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "pdf" => "application/pdf",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "ogg" => "audio/ogg",
        "txt" | "md" => "text/plain",
        "csv" => "text/csv",
        "json" => "application/json",
        "zip" => "application/zip",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        _ => "application/octet-stream",
    }
}

/// A photo only when the bytes really are JPEG or PNG: a provider rejects a mislabeled photo
/// only after the send started, which would leave the outcome unknown.
fn kind_for(data: &[u8], mime: &str) -> Kind {
    let jpeg = mime == "image/jpeg" && data.starts_with(&[0xff, 0xd8, 0xff]);
    let png = mime == "image/png" && data.starts_with(b"\x89PNG\r\n\x1a\n");
    if (jpeg || png) && data.len() as u64 <= MAX_PHOTO_BYTES {
        Kind::Image
    } else {
        Kind::Document
    }
}

fn plain_name(path: &Path) -> Result<String, Error> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| refused("The file name must be valid text."))?;
    if name.len() > 255 || name.chars().any(char::is_control) {
        return Err(refused(
            "The file name must be at most 255 bytes, with no control characters.",
        ));
    }
    Ok(name.to_string())
}

/// Checks a path the agent gave and reads the file. `protected` are messaging-mcp's own folders
/// (the state directory, the config folder, the secret files' folders), refused even when not
/// hidden.
pub fn read_allowed(path: &str, protected: &[PathBuf]) -> Result<(Attachment, Vec<u8>), Error> {
    let given = Path::new(path);
    if !given.is_absolute() {
        return Err(refused("Give the file's absolute path."));
    }
    // Resolve links first, so a link cannot point into a refused folder.
    let resolved = fs::canonicalize(given).map_err(|_| unreadable())?;
    if is_hidden(&resolved) {
        return Err(refused(
            "Files in hidden folders, and hidden files, are never sent. Copy the file to a \
             normal folder if the user wants it sent.",
        ));
    }
    let inside_protected = protected
        .iter()
        .any(|folder| fs::canonicalize(folder).is_ok_and(|folder| resolved.starts_with(folder)));
    if inside_protected {
        return Err(refused("messaging-mcp's own files are never sent."));
    }
    let name = plain_name(&resolved)?;
    let file = File::open(&resolved).map_err(|_| unreadable())?;
    let metadata = file.metadata().map_err(|_| unreadable())?;
    if !metadata.is_file() {
        return Err(unreadable());
    }
    let too_big = || refused("Files up to 50 MB can be sent.");
    if metadata.len() > MAX_FILE_BYTES {
        return Err(too_big());
    }
    let mut data = Vec::new();
    // Read one byte past the limit, in case the file grew after the size check.
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|_| unreadable())?;
    if data.len() as u64 > MAX_FILE_BYTES {
        return Err(too_big());
    }
    if data.is_empty() {
        return Err(refused("The file is empty."));
    }
    let mime = mime_for(&name);
    let attachment = Attachment {
        path: resolved.to_string_lossy().into_owned(),
        name,
        size: data.len() as u64,
        mime: mime.to_string(),
        kind: kind_for(&data, mime),
        sha256: sha256_hex(&data),
    };
    Ok((attachment, data))
}

/// The folder of copied files, inside the state directory.
pub fn files_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("files")
}

/// Where a plan's copy lives.
pub fn copy_path(state_dir: &Path, plan_id: &str) -> PathBuf {
    files_dir(state_dir).join(plan_id)
}

/// Saves the reviewed copy of a plan's file, owner-only.
pub fn save_copy(state_dir: &Path, plan_id: &str, data: &[u8]) -> Result<(), Error> {
    ensure_private_dir(&files_dir(state_dir))?;
    write_private_file(&copy_path(state_dir, plan_id), data)
}

/// Reads a plan's copy back, refusing it unless it is still exactly what was reviewed.
pub fn load_copy(state_dir: &Path, plan_id: &str, sha256: &str) -> Result<Vec<u8>, DeliveryError> {
    let data = crate::secrets::read_private_bytes(&copy_path(state_dir, plan_id))
        .ok()
        .filter(|data| sha256_hex(data) == sha256)
        .ok_or_else(|| {
            DeliveryError::new(
                "FILE_CHANGED",
                "The reviewed copy of the file is missing or changed. Nothing was sent; prepare \
                 the message again.",
                Outcome::NotSent,
            )
        })?;
    Ok(data)
}

/// Removes a plan's copy once it can never be sent again. Best effort: a leftover copy is only
/// disk space, never a send.
pub fn remove_copy(state_dir: &Path, plan_id: &str) {
    let _ = fs::remove_file(copy_path(state_dir, plan_id));
}
