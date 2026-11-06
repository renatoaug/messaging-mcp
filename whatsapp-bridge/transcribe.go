package main

import (
	"context"
	"fmt"
	"log"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/types"
)

// maxAudioBytes matches messaging-mcp's limit: about 20 minutes of Opus.
const maxAudioBytes = 20 << 20

// transcribeTimeout bounds one download and transcription.
const transcribeTimeout = 4 * time.Minute

// voiceJob is a voice message to turn into text.
type voiceJob struct {
	chat    string
	id      string
	message *waE2E.AudioMessage
}

// Transcriber runs `messaging-mcp transcribe` on voice messages as they arrive, one at a time, so
// only their text is kept. The audio is downloaded into a private folder and removed right after.
type Transcriber struct {
	exe    string
	config string
	dir    string
	jobs   chan voiceJob
}

// NewTranscriber returns nil when no messaging-mcp was given: voice messages then stay
// "(not transcribed)".
func NewTranscriber(exe, config, storeDir string) *Transcriber {
	if exe == "" || config == "" {
		return nil
	}
	return &Transcriber{exe: exe, config: config, dir: filepath.Join(storeDir, "transcribe"), jobs: make(chan voiceJob, 64)}
}

// command is what runs for one audio file.
func (t *Transcriber) command(ctx context.Context, file string) *exec.Cmd {
	return exec.CommandContext(ctx, t.exe, "--config", t.config, "transcribe", file)
}

// Queue adds a voice message; when the queue is full it is left untranscribed.
func (t *Transcriber) Queue(job voiceJob) {
	select {
	case t.jobs <- job:
	default:
		log.Print("transcription queue full")
	}
}

// Run transcribes queued voice messages until ctx ends. download fetches the audio from WhatsApp.
func (t *Transcriber) Run(ctx context.Context, download func(context.Context, *waE2E.AudioMessage) ([]byte, error), inbox *Inbox) {
	for {
		select {
		case <-ctx.Done():
			return
		case job := <-t.jobs:
			text, err := t.transcribe(ctx, job, download)
			if err != nil {
				// No detail: it could carry the audio's content.
				log.Print("a voice message was not transcribed")
				continue
			}
			if err := inbox.SetText(ctx, job.chat, job.id, voiceText(job.message.GetSeconds(), &text)); err != nil {
				log.Print("could not keep a transcript")
			}
		}
	}
}

func (t *Transcriber) transcribe(ctx context.Context, job voiceJob, download func(context.Context, *waE2E.AudioMessage) ([]byte, error)) (string, error) {
	if job.message.GetFileLength() > maxAudioBytes {
		return "", fmt.Errorf("too long")
	}
	ctx, cancel := context.WithTimeout(ctx, transcribeTimeout)
	defer cancel()
	audio, err := download(ctx, job.message)
	if err != nil || len(audio) > maxAudioBytes {
		return "", fmt.Errorf("download")
	}
	if err := os.MkdirAll(t.dir, 0o700); err != nil {
		return "", err
	}
	file, err := os.CreateTemp(t.dir, "voice-*.ogg")
	if err != nil {
		return "", err
	}
	defer os.Remove(file.Name())
	if err := file.Chmod(0o600); err != nil {
		file.Close()
		return "", err
	}
	_, err = file.Write(audio)
	if closeErr := file.Close(); err == nil {
		err = closeErr
	}
	if err != nil {
		return "", err
	}
	out, err := t.command(ctx, file.Name()).Output()
	if err != nil {
		return "", err
	}
	return strings.TrimSpace(string(out)), nil
}

// queueVoice hands an arriving voice message to the transcriber.
func (w *whatsapp) queueVoice(chat string, info types.MessageInfo, message *waE2E.Message) {
	audio := message.GetAudioMessage()
	if audio == nil && message.GetEphemeralMessage() != nil {
		audio = message.GetEphemeralMessage().GetMessage().GetAudioMessage()
	}
	if w.transcriber == nil || audio == nil {
		return
	}
	w.transcriber.Queue(voiceJob{chat: chat, id: info.ID, message: audio})
}
