//! Voice messages as text: finding the tools, the exact commands, cleaning whisper's output, and
//! a failed run leaving no audio behind.

#![allow(clippy::unwrap_used)]

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use common::*;
use grammers_tl_types as tl;
use messaging_mcp::adapters::telegram::{file_of, voice_of};
use messaging_mcp::transcribe::{Transcriber, voice_text};
use serde_json::json;

fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    path
}

fn transcriber(dir: &Path, whisper: &str) -> Transcriber {
    Transcriber {
        ffmpeg: script(dir, "ffmpeg", r#"eval last=\${$#}; cp "$6" "$last""#),
        whisper: script(dir, "whisper-cli", whisper),
        model: dir.join("model.bin"),
        language: "pt".to_string(),
        work_dir: dir.join("work"),
    }
}

#[tokio::test]
async fn whisper_output_becomes_one_paragraph_and_the_audio_is_removed() {
    let dir = tempfile::Builder::new().prefix("tools").tempdir().unwrap();
    let t = transcriber(
        dir.path(),
        r"printf '[BLANK_AUDIO]\n Oi, tudo bem?\n\n (música) \n Até amanhã. \n'",
    );
    let text = t.transcribe(b"opus", "v1").await.unwrap();
    assert_eq!(text, "Oi, tudo bem? Até amanhã.");
    assert_eq!(
        std::fs::read_dir(dir.path().join("work")).unwrap().count(),
        0
    );
    let mode = std::fs::metadata(dir.path().join("work"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o700);
}

#[tokio::test]
async fn a_failing_tool_or_an_empty_file_is_an_error_with_nothing_left() {
    let dir = tempfile::Builder::new().prefix("tools").tempdir().unwrap();
    let t = transcriber(dir.path(), "echo secret words; exit 1");
    let error = t.transcribe(b"opus", "v2").await.unwrap_err();
    assert_eq!(error.code, "TRANSCRIPTION_FAILED");
    assert!(
        !error.message.contains("secret"),
        "tool output never reaches an error"
    );
    assert_eq!(
        std::fs::read_dir(dir.path().join("work")).unwrap().count(),
        0
    );
    assert!(t.transcribe(b"", "v3").await.is_err());
}

#[test]
fn the_commands_are_exact() {
    let t = Transcriber {
        ffmpeg: "/bin/ffmpeg".into(),
        whisper: "/bin/whisper-cli".into(),
        model: "/m/ggml-small.bin".into(),
        language: "auto".to_string(),
        work_dir: "/w".into(),
    };
    let ffmpeg = Transcriber::ffmpeg_args(Path::new("/w/a.audio"), Path::new("/w/a.wav"));
    assert_eq!(
        ffmpeg.join(std::ffi::OsStr::new(" ")),
        "-nostdin -loglevel error -y -i /w/a.audio -ar 16000 -ac 1 -c:a pcm_s16le /w/a.wav"
    );
    let whisper = t.whisper_args(Path::new("/w/a.wav"));
    assert_eq!(
        whisper.join(std::ffi::OsStr::new(" ")),
        "-m /m/ggml-small.bin -l auto -nt -np -f /w/a.wav"
    );
}

#[test]
fn the_transcriber_needs_the_tools_the_model_and_the_setting() {
    let dir = tempfile::Builder::new().prefix("tools").tempdir().unwrap();
    script(dir.path(), "ffmpeg", "true");
    script(dir.path(), "whisper-cli", "true");
    let path = dir.path().to_string_lossy().into_owned();
    let env = move |name: &str| (name == "PATH").then(|| path.clone());
    let model = dir.path().join("ggml-small.bin");
    let config = |enabled: bool| {
        config_with(
            dir.path(),
            json!({ "transcription": { "enabled": enabled, "language": "pt", "model": model } }),
        )
    };
    assert!(
        Transcriber::find(&config(true), &env).is_none(),
        "no model yet"
    );
    std::fs::write(&model, b"model").unwrap();
    let found = Transcriber::find(&config(true), &env).unwrap();
    assert_eq!(found.language, "pt");
    assert_eq!(found.whisper, dir.path().join("whisper-cli"));
    assert!(
        Transcriber::find(&config(false), &env).is_none(),
        "turned off"
    );
}

#[test]
fn a_voice_message_reads_with_its_length() {
    assert_eq!(voice_text(Some(75), Some("oi")), "[audio 1:15] oi");
    assert_eq!(voice_text(Some(5), Some("")), "[audio 0:05] (nothing said)");
    assert_eq!(voice_text(None, None), "[audio] (not transcribed)");
}

fn document(mime: &str, attributes: Vec<tl::enums::DocumentAttribute>) -> tl::enums::MessageMedia {
    tl::types::MessageMediaDocument {
        nopremium: false,
        spoiler: false,
        video: false,
        round: false,
        voice: false,
        document: Some(
            tl::types::Document {
                id: 9,
                access_hash: 3,
                file_reference: vec![1],
                date: 0,
                mime_type: mime.to_string(),
                size: 2048,
                thumbs: None,
                video_thumbs: None,
                dc_id: 4,
                attributes,
            }
            .into(),
        ),
        alt_documents: None,
        video_cover: None,
        video_timestamp: None,
        ttl_seconds: None,
    }
    .into()
}

#[test]
fn telegram_voice_messages_and_audio_files_are_found_and_other_files_are_not() {
    let audio = tl::types::DocumentAttributeAudio {
        voice: true,
        duration: 42,
        title: None,
        performer: None,
        waveform: None,
    };
    let voice = voice_of(&document("audio/ogg", vec![audio.into()])).unwrap();
    assert_eq!(
        (voice.id, voice.size, voice.dc_id, voice.seconds),
        (9, 2048, 4, Some(42))
    );
    assert!(voice_of(&document("audio/mpeg", vec![])).is_some());
    assert!(voice_of(&document("application/pdf", vec![])).is_none());
}

#[test]
fn telegram_photos_and_files_can_be_saved_but_voice_and_stickers_cannot() {
    let named = tl::types::DocumentAttributeFilename {
        file_name: "deck.pdf".to_string(),
    };
    let file = file_of(&document("application/pdf", vec![named.into()])).unwrap();
    assert_eq!(
        (
            file.name.as_str(),
            file.mime.as_str(),
            file.size,
            file.dc_id
        ),
        ("deck.pdf", "application/pdf", 2048, 4)
    );
    assert_eq!(
        file_of(&document("application/pdf", vec![])).unwrap().name,
        "file-9.pdf"
    );
    let voice = tl::types::DocumentAttributeAudio {
        voice: true,
        duration: 3,
        title: None,
        performer: None,
        waveform: None,
    };
    assert!(file_of(&document("audio/ogg", vec![voice.into()])).is_none());
    let sticker = tl::types::DocumentAttributeSticker {
        mask: false,
        alt: String::new(),
        stickerset: tl::types::InputStickerSetEmpty {}.into(),
        mask_coords: None,
    };
    assert!(file_of(&document("image/webp", vec![sticker.into()])).is_none());

    let size = |kind: &str, w: i32, size: i32| -> tl::enums::PhotoSize {
        tl::types::PhotoSize {
            r#type: kind.to_string(),
            w,
            h: w,
            size,
        }
        .into()
    };
    let photo: tl::enums::MessageMedia = tl::types::MessageMediaPhoto {
        spoiler: false,
        live_photo: false,
        photo: Some(
            tl::types::Photo {
                has_stickers: false,
                id: 11,
                access_hash: 2,
                file_reference: vec![1],
                date: 0,
                sizes: vec![
                    size("s", 90, 100),
                    size("y", 1280, 90_000),
                    size("m", 320, 900),
                ],
                video_sizes: None,
                dc_id: 2,
            }
            .into(),
        ),
        ttl_seconds: None,
        video: None,
    }
    .into();
    let file = file_of(&photo).unwrap();
    assert_eq!(
        (file.name.as_str(), file.size, file.dc_id),
        ("photo-11.jpg", 90_000, 2)
    );
    let tl::enums::InputFileLocation::InputPhotoFileLocation(location) = file.location else {
        panic!("a photo location");
    };
    assert_eq!(location.thumb_size, "y", "the largest size");
}
