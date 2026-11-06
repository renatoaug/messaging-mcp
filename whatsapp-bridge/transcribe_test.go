package main

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"testing"
	"time"

	"go.mau.fi/whatsmeow/proto/waE2E"
	"google.golang.org/protobuf/proto"
)

// fakeTranscriber stands in for messaging-mcp: it checks the arguments and the audio file's
// permissions, then prints a transcript.
func fakeTranscriber(t *testing.T) string {
	t.Helper()
	script := filepath.Join(t.TempDir(), "messaging-mcp")
	body := `#!/bin/sh
[ "$1" = "--config" ] && [ "$3" = "transcribe" ] || exit 2
[ "$(stat -f %Lp "$4" 2>/dev/null || stat -c %a "$4")" = "600" ] || exit 3
[ "$(cat "$4")" = "opus bytes" ] || exit 4
echo "  oi, tudo bem?  "
`
	if err := os.WriteFile(script, []byte(body), 0o700); err != nil {
		t.Fatal(err)
	}
	return script
}

func TestAVoiceMessageIsKeptAsItsTranscript(t *testing.T) {
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	store := t.TempDir()
	inbox := openTestInbox(t, time.Now())
	chat := "5511988887777@s.whatsapp.net"
	audio := &waE2E.AudioMessage{Seconds: proto.Uint32(7)}
	if err := inbox.Add(ctx, Stored{Chat: chat, ID: "v1", From: "Nestor", At: time.Now().UnixMilli(), Text: messageText(&waE2E.Message{AudioMessage: audio})}); err != nil {
		t.Fatal(err)
	}
	tr := NewTranscriber(fakeTranscriber(t), "/config.json", store)
	download := func(context.Context, *waE2E.AudioMessage) ([]byte, error) { return []byte("opus bytes"), nil }
	go tr.Run(ctx, download, inbox)
	tr.Queue(voiceJob{chat: chat, id: "v1", message: audio})
	deadline := time.Now().Add(5 * time.Second)
	for {
		messages, _ := inbox.Recent(ctx, chat, 1)
		if len(messages) == 1 && messages[0].Text == "[audio 0:07] oi, tudo bem?" {
			break
		}
		if time.Now().After(deadline) {
			t.Fatalf("got %v", messages)
		}
		time.Sleep(20 * time.Millisecond)
	}
	if left, _ := os.ReadDir(filepath.Join(store, "transcribe")); len(left) != 0 {
		t.Fatalf("the audio must be removed, found %v", left)
	}
}

func TestAFailedDownloadOrAnOversizedVoiceIsNotTranscribed(t *testing.T) {
	tr := NewTranscriber(fakeTranscriber(t), "/config.json", t.TempDir())
	fail := func(context.Context, *waE2E.AudioMessage) ([]byte, error) { return nil, errors.New("gone") }
	if _, err := tr.transcribe(context.Background(), voiceJob{message: &waE2E.AudioMessage{}}, fail); err == nil {
		t.Fatal("a failed download must not be transcribed")
	}
	big := &waE2E.AudioMessage{FileLength: proto.Uint64(maxAudioBytes + 1)}
	ok := func(context.Context, *waE2E.AudioMessage) ([]byte, error) { return []byte("x"), nil }
	if _, err := tr.transcribe(context.Background(), voiceJob{message: big}, ok); err == nil {
		t.Fatal("an oversized voice message must not be transcribed")
	}
	if NewTranscriber("", "", t.TempDir()) != nil {
		t.Fatal("no messaging-mcp, no transcriber")
	}
}
