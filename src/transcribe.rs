//! Voice messages as text. The agent cannot listen to audio, so a voice message is transcribed on
//! this machine with whisper.cpp (`whisper-cli`), after `ffmpeg` turns it into 16 kHz WAV. The
//! audio never leaves the machine, and its temporary files are removed right after.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::config::Config;
use crate::error::Error;
use crate::secrets::Env;
use crate::setup::config_file::{ensure_private_dir, write_private_file};

/// Longer voice messages are not transcribed: about 20 minutes of Opus.
pub const MAX_AUDIO_BYTES: usize = 20 * 1024 * 1024;

/// How long one transcription may take, conversion included.
const TIMEOUT: Duration = Duration::from_secs(180);

/// The `transcription` section of the config. All optional.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TranscriptionConfig {
    /// False turns transcription off; voice messages then read as `[audio]`.
    #[serde(default = "enabled")]
    pub enabled: bool,
    /// `auto`, or a language code such as `pt` or `en`. A fixed language is more accurate on
    /// short messages.
    #[serde(default = "auto")]
    pub language: String,
    /// The whisper.cpp model. Default: `whisper/ggml-small.bin` next to the config.
    pub model: Option<PathBuf>,
}

fn enabled() -> bool {
    true
}

fn auto() -> String {
    "auto".to_string()
}

impl Default for TranscriptionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            language: auto(),
            model: None,
        }
    }
}

/// Where Homebrew and most installers put the tools, for an MCP client whose PATH is short.
const TOOL_DIRS: [&str; 3] = ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"];

fn find_tool(name: &str, env: Env<'_>) -> Option<PathBuf> {
    let path = env("PATH").unwrap_or_default();
    path.split(':')
        .filter(|dir| !dir.is_empty())
        .chain(TOOL_DIRS)
        .map(|dir| Path::new(dir).join(name))
        .find(|candidate| candidate.is_file())
}

fn failed() -> Error {
    Error::new(
        "TRANSCRIPTION_FAILED",
        "The voice message could not be transcribed.",
    )
}

/// Runs ffmpeg and whisper-cli on one voice message.
#[derive(Clone, Debug)]
pub struct Transcriber {
    pub ffmpeg: PathBuf,
    pub whisper: PathBuf,
    pub model: PathBuf,
    pub language: String,
    /// A private folder for the temporary audio and WAV files.
    pub work_dir: PathBuf,
}

impl Transcriber {
    /// The transcriber the config and this machine allow, or `None` when it is off or a tool or
    /// the model is missing (voice messages then read as `[audio]`).
    pub fn find(config: &Config, env: Env<'_>) -> Option<Self> {
        let settings = &config.transcription;
        if !settings.enabled {
            return None;
        }
        let model = settings
            .model
            .clone()
            .unwrap_or_else(|| config.base_dir.join("whisper").join("ggml-small.bin"));
        if !model.is_file() {
            return None;
        }
        Some(Self {
            ffmpeg: find_tool("ffmpeg", env)?,
            whisper: find_tool("whisper-cli", env)?,
            model,
            language: settings.language.clone(),
            work_dir: config.state_dir.join("transcribe"),
        })
    }

    /// Any input format ffmpeg reads, to the 16 kHz mono WAV whisper.cpp expects.
    pub fn ffmpeg_args(input: &Path, output: &Path) -> Vec<OsString> {
        let mut args: Vec<OsString> = ["-nostdin", "-loglevel", "error", "-y", "-i"]
            .into_iter()
            .map(OsString::from)
            .collect();
        args.push(input.into());
        args.extend(
            ["-ar", "16000", "-ac", "1", "-c:a", "pcm_s16le"]
                .into_iter()
                .map(OsString::from),
        );
        args.push(output.into());
        args
    }

    /// Plain text only: no timestamps, no progress, no colors.
    pub fn whisper_args(&self, wav: &Path) -> Vec<OsString> {
        let mut args: Vec<OsString> = vec!["-m".into(), self.model.clone().into_os_string()];
        args.extend(["-l".into(), OsString::from(&self.language)]);
        args.extend(["-nt", "-np", "-f"].into_iter().map(OsString::from));
        args.push(wav.into());
        args
    }

    /// whisper.cpp's text as one paragraph, without its markers for silence or music.
    pub fn clean_output(stdout: &str) -> String {
        stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .filter(|line| !(line.starts_with('[') && line.ends_with(']')))
            .filter(|line| !(line.starts_with('(') && line.ends_with(')')))
            .collect::<Vec<_>>()
            .join(" ")
    }

    async fn run(program: &Path, args: &[OsString]) -> Result<Vec<u8>, Error> {
        let child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            // Nothing from the tools is shown or logged: it could quote the audio.
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| failed())?;
        let output = tokio::time::timeout(TIMEOUT, child.wait_with_output())
            .await
            .map_err(|_| failed())?
            .map_err(|_| failed())?;
        if !output.status.success() {
            return Err(failed());
        }
        Ok(output.stdout)
    }

    /// The text of one voice message. Empty when nothing was said.
    pub async fn transcribe(&self, audio: &[u8], name: &str) -> Result<String, Error> {
        if audio.is_empty() || audio.len() > MAX_AUDIO_BYTES {
            return Err(failed());
        }
        ensure_private_dir(&self.work_dir)?;
        let input = self.work_dir.join(format!("{name}.audio"));
        let wav = self.work_dir.join(format!("{name}.wav"));
        let result = async {
            write_private_file(&input, audio)?;
            Self::run(&self.ffmpeg, &Self::ffmpeg_args(&input, &wav)).await?;
            let stdout = Self::run(&self.whisper, &self.whisper_args(&wav)).await?;
            Ok(Self::clean_output(&String::from_utf8_lossy(&stdout)))
        }
        .await;
        let _ = std::fs::remove_file(&input);
        let _ = std::fs::remove_file(&wav);
        result
    }
}

/// How a voice message reads: its length, then what was said.
pub fn voice_text(seconds: Option<u32>, transcript: Option<&str>) -> String {
    let label = match seconds {
        Some(seconds) => format!("[audio {}:{:02}]", seconds / 60, seconds % 60),
        None => "[audio]".to_string(),
    };
    match transcript {
        Some(text) if !text.is_empty() => format!("{label} {text}"),
        Some(_) => format!("{label} (nothing said)"),
        None => format!("{label} (not transcribed)"),
    }
}
