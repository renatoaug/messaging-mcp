# ADR-013: Voice messages are transcribed on this machine

- **Status**: Accepted
- **Date**: 2026-11-06
- **Related**: extends [ADR-012](ADR-012-read-chats-when-the-owner-asks.md); [design/03-telegram.md](../design/03-telegram.md#reading-a-chat), [design/04-whatsapp.md](../design/04-whatsapp.md#reading-a-chat), [SECURITY.md](../../SECURITY.md)

## Context

The owner wants `read_messages` to tell what a voice message says, on both channels. The agent cannot listen to audio, so something has to turn it into text. WhatsApp media links expire, and the bridge keeps only text ([ADR-012](ADR-012-read-chats-when-the-owner-asks.md)).

## Decision

**Voice messages are transcribed locally by whisper.cpp (`whisper-cli`, model `ggml-small.bin` next to the config by default), after `ffmpeg` converts them to 16 kHz WAV. One implementation, `messaging-mcp transcribe`, serves both channels. Telegram downloads and transcribes the latest 10 voice messages of a chat when it is read, and remembers each transcript for the life of the process. The WhatsApp bridge transcribes a voice message as it arrives, by running `messaging-mcp transcribe` on a private temporary file, keeps only the text (`[audio 0:42] …`) and deletes the audio.** Transcription is on when the tools and the model are present; `transcription.enabled: false` turns it off.

## Alternatives considered

- **A cloud speech API:** better accuracy, but the owner's voice messages would go to another company, and it needs an API key and money.
- **Telegram's own transcription:** only for Premium accounts, and nothing like it for WhatsApp.
- **whisper.cpp linked into the binaries:** no external tools, but a C++ build in both release pipelines and `CGO` in the bridge.
- **Keep WhatsApp audio until asked:** transcribes only what is read, but keeps the audio on disk.

## Consequences

- Reading a chat tells what voice messages say, and no audio leaves the machine.
- The machine needs `whisper-cli`, `ffmpeg` and a 466 MB model; `setup_status` says how to install them, and the agent asks before it does.
- Every WhatsApp voice message is transcribed when it arrives, read or not: a few seconds of CPU each, one at a time.
- A transcript can be wrong, especially for short messages with `language: auto`.
