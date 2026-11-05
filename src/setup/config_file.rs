//! Writing the config and private files. The setup tools change the owner's config, so every
//! write is atomic (a private temporary file, then a rename) and validated before it lands.

use std::fs::{self, DirBuilder, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;

use rustix::fs::OFlags;
use serde_json::{Map, Value};

use crate::config::{Config, parse_config};
use crate::error::Error;
use crate::secrets::is_owner_only;

const DEFAULT_CONFIG: &str =
    "{\n  \"mode\": \"dry-run\",\n  \"stateDir\": \"state\",\n  \"recipients\": []\n}\n";

fn write_error() -> Error {
    Error::new(
        "SETUP_WRITE_FAILED",
        "Could not write a private file in the config folder. Check that it is owned by you \
         with mode 0700.",
    )
}

/// Creates `dir` with mode 0700 if missing, and refuses one that others can read or a symlink.
pub fn ensure_private_dir(dir: &Path) -> Result<(), Error> {
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|_| write_error())?;
    let metadata = dir.symlink_metadata().map_err(|_| write_error())?;
    if metadata.is_dir() && is_owner_only(&metadata) {
        Ok(())
    } else {
        Err(Error::new(
            "STATE_PERMISSIONS",
            "The config folder must be an owner-only directory (0700), not a symlink.",
        ))
    }
}

/// Replaces `path` with `contents` atomically, as an owner-only (0600) regular file.
pub fn write_private_file(path: &Path, contents: &[u8]) -> Result<(), Error> {
    let dir = path.parent().ok_or_else(write_error)?;
    let name = path.file_name().ok_or_else(write_error)?.to_string_lossy();
    let temporary = dir.join(format!(".{name}.tmp"));
    // A leftover from an interrupted write is ours; it never holds the only copy.
    let _ = fs::remove_file(&temporary);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(OFlags::NOFOLLOW.bits() as i32)
        .open(&temporary)
        .map_err(|_| write_error())?;
    file.write_all(contents).map_err(|_| write_error())?;
    file.sync_all().map_err(|_| write_error())?;
    fs::rename(&temporary, path).map_err(|_| write_error())
}

/// Creates a dry-run config at `path` when none exists. Returns whether it created one.
pub fn ensure_config(path: &Path) -> Result<bool, Error> {
    if path.symlink_metadata().is_ok() {
        return Ok(false);
    }
    ensure_private_dir(path.parent().ok_or_else(write_error)?)?;
    write_private_file(path, DEFAULT_CONFIG.as_bytes())?;
    Ok(true)
}

/// Applies `edit` to the config's JSON, checks the result is a valid config, then saves it.
pub fn update_config(
    path: &Path,
    edit: impl FnOnce(&mut Map<String, Value>),
) -> Result<Config, Error> {
    let read_error = || Error::new("CONFIG_READ_FAILED", "Cannot read the configuration.");
    let text = fs::read_to_string(path).map_err(|_| read_error())?;
    let mut value: Value = serde_json::from_str(&text).map_err(|_| read_error())?;
    let object = value.as_object_mut().ok_or_else(read_error)?;
    edit(object);
    let mut updated = serde_json::to_string_pretty(&value).map_err(|_| write_error())?;
    updated.push('\n');
    let absolute = std::path::absolute(path).map_err(|_| read_error())?;
    let config = parse_config(&updated, absolute.parent().ok_or_else(read_error)?)?;
    write_private_file(path, updated.as_bytes())?;
    Ok(config)
}

/// The object at `key`, created empty when missing or not an object.
pub fn section<'a>(object: &'a mut Map<String, Value>, key: &str) -> &'a mut Map<String, Value> {
    let entry = object
        .entry(key.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !entry.is_object() {
        *entry = Value::Object(Map::new());
    }
    match entry {
        Value::Object(map) => map,
        _ => unreachable!("replaced with an object above"),
    }
}
