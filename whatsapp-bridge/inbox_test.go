package main

import (
	"context"
	"os"
	"path/filepath"
	"testing"
	"time"

	"go.mau.fi/whatsmeow/proto/waE2E"
	"google.golang.org/protobuf/proto"
)

func openTestInbox(t *testing.T, now time.Time) *Inbox {
	t.Helper()
	path := filepath.Join(t.TempDir(), "messages.db")
	inbox, err := OpenInbox(context.Background(), path)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { inbox.Close() })
	inbox.now = func() time.Time { return now }
	info, err := os.Stat(path)
	if err != nil || info.Mode().Perm() != 0o600 {
		t.Fatalf("messages.db must be 0600: %v %v", info.Mode(), err)
	}
	return inbox
}

func TestTheInboxKeepsThreeDaysOncePerMessage(t *testing.T) {
	ctx := context.Background()
	now := time.UnixMilli(1_000_000_000_000)
	inbox := openTestInbox(t, now)
	chat := "5511988887777@s.whatsapp.net"
	add := func(id string, age time.Duration, text string) {
		m := Stored{Chat: chat, ID: id, From: "Alice", At: now.Add(-age).UnixMilli(), Text: text}
		if err := inbox.Add(ctx, m); err != nil {
			t.Fatal(err)
		}
	}
	add("old", 73*time.Hour, "too old")
	add("a", 2*time.Hour, "first")
	add("a", 2*time.Hour, "first again")
	add("b", time.Hour, "second")
	add("c", time.Minute, "")
	messages, err := inbox.Recent(ctx, chat, 10)
	if err != nil || len(messages) != 2 || messages[0].Text != "first" || messages[1].Text != "second" {
		t.Fatalf("got %v %v", messages, err)
	}
	if last, _ := inbox.Recent(ctx, chat, 1); len(last) != 1 || last[0].Text != "second" {
		t.Fatalf("the limit keeps the newest: %v", last)
	}
	inbox.now = func() time.Time { return now.Add(71 * time.Hour) }
	if err := inbox.Prune(ctx); err != nil {
		t.Fatal(err)
	}
	if kept, _ := inbox.Recent(ctx, chat, 10); len(kept) != 1 || kept[0].Text != "second" {
		t.Fatalf("after pruning: %v", kept)
	}
	if names, _ := inbox.Senders(ctx); names[chat] != "Alice" {
		t.Fatalf("senders: %v", names)
	}
}

func TestMessageTextLabelsMediaAndSkipsTheRest(t *testing.T) {
	for _, tc := range []struct {
		message *waE2E.Message
		want    string
	}{
		{&waE2E.Message{Conversation: proto.String("oi")}, "oi"},
		{&waE2E.Message{ExtendedTextMessage: &waE2E.ExtendedTextMessage{Text: proto.String("link")}}, "link"},
		{&waE2E.Message{ImageMessage: &waE2E.ImageMessage{Caption: proto.String("look")}}, "[photo] look"},
		{&waE2E.Message{DocumentMessage: &waE2E.DocumentMessage{FileName: proto.String("a.pdf")}}, "[file: a.pdf]"},
		{&waE2E.Message{AudioMessage: &waE2E.AudioMessage{Seconds: proto.Uint32(75)}}, "[audio 1:15] (not transcribed)"},
		{&waE2E.Message{ReactionMessage: &waE2E.ReactionMessage{Text: proto.String("x")}}, ""},
		{nil, ""},
	} {
		if got := messageText(tc.message); got != tc.want {
			t.Fatalf("got %q, want %q", got, tc.want)
		}
	}
	if fold("Álice") != "alice" {
		t.Fatalf("fold: %q", fold("Álice"))
	}
}

func TestMediaKeysAreKeptAndShownAsAnAttachment(t *testing.T) {
	ctx := context.Background()
	now := time.UnixMilli(1_000_000_000_000)
	inbox := openTestInbox(t, now)
	chat := "5511988887777@s.whatsapp.net"
	keys, _ := proto.Marshal(mediaOf(&waE2E.Message{DocumentMessage: &waE2E.DocumentMessage{FileName: proto.String("deck.pdf")}}))
	for _, m := range []Stored{
		{Chat: chat, ID: "TEXT1", From: "Alice", At: now.UnixMilli() - 2, Text: "oi"},
		{Chat: chat, ID: "FILE1", From: "Alice", At: now.UnixMilli() - 1, Text: "[file: deck.pdf]", Media: keys},
	} {
		if err := inbox.Add(ctx, m); err != nil {
			t.Fatal(err)
		}
	}
	messages, _ := inbox.Recent(ctx, chat, 10)
	if len(messages) != 2 || messages[0].Attachment != "" || messages[1].Attachment != "FILE1" {
		t.Fatalf("got %+v", messages)
	}
	if got, err := inbox.Media(ctx, chat, "FILE1"); err != nil || string(got) != string(keys) {
		t.Fatalf("media: %v", err)
	}
	if got, err := inbox.Media(ctx, chat, "TEXT1"); err != nil || got != nil {
		t.Fatalf("a text message has no media: %v %v", got, err)
	}
	if got, err := inbox.Media(ctx, chat, "MISSING"); err != nil || got != nil {
		t.Fatalf("a missing message has no media: %v %v", got, err)
	}
	inbox.now = func() time.Time { return now.Add(73 * time.Hour) }
	if got, _ := inbox.Media(ctx, chat, "FILE1"); got != nil {
		t.Fatal("media older than three days is not handed over")
	}
}

func TestAnOldInboxGetsTheMediaColumn(t *testing.T) {
	ctx := context.Background()
	path := filepath.Join(t.TempDir(), "messages.db")
	old, err := OpenInbox(ctx, path)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := old.db.ExecContext(ctx, "ALTER TABLE message DROP COLUMN media"); err != nil {
		t.Fatal(err)
	}
	old.Close()
	again, err := OpenInbox(ctx, path)
	if err != nil {
		t.Fatal(err)
	}
	defer again.Close()
	if err := again.Add(ctx, Stored{Chat: "c", ID: "X", From: "A", At: time.Now().UnixMilli(), Text: "t", Media: []byte{1}}); err != nil {
		t.Fatal(err)
	}
}

func TestOnlyPhotosVideosAndDocumentsAreKeptToDownload(t *testing.T) {
	doc := &waE2E.DocumentMessage{FileName: proto.String("a.pdf"), Mimetype: proto.String("application/pdf"), FileLength: proto.Uint64(9)}
	for _, tc := range []struct {
		message *waE2E.Message
		name    string
	}{
		{&waE2E.Message{ImageMessage: &waE2E.ImageMessage{Mimetype: proto.String("image/jpeg")}}, "photo-ID.jpg"},
		{&waE2E.Message{VideoMessage: &waE2E.VideoMessage{Mimetype: proto.String("video/mp4")}}, "video-ID.mp4"},
		{&waE2E.Message{DocumentMessage: doc}, "a.pdf"},
		{&waE2E.Message{DocumentMessage: &waE2E.DocumentMessage{Mimetype: proto.String("application/pdf")}}, "file-ID.pdf"},
		{&waE2E.Message{DocumentWithCaptionMessage: &waE2E.FutureProofMessage{Message: &waE2E.Message{DocumentMessage: doc}}}, "a.pdf"},
		{&waE2E.Message{EphemeralMessage: &waE2E.FutureProofMessage{Message: &waE2E.Message{ImageMessage: &waE2E.ImageMessage{}}}}, "photo-ID.jpg"},
		{&waE2E.Message{AudioMessage: &waE2E.AudioMessage{}}, ""},
		{&waE2E.Message{StickerMessage: &waE2E.StickerMessage{}}, ""},
		{&waE2E.Message{Conversation: proto.String("oi")}, ""},
	} {
		media := mediaOf(tc.message)
		if tc.name == "" {
			if media != nil {
				t.Fatalf("%v should not be kept", tc.message)
			}
			continue
		}
		file, downloadable := describeMedia(media, "ID")
		if downloadable == nil || file.name != tc.name {
			t.Fatalf("got %q, want %q", file.name, tc.name)
		}
	}
	if text := messageText(&waE2E.Message{DocumentWithCaptionMessage: &waE2E.FutureProofMessage{Message: &waE2E.Message{DocumentMessage: doc}}}); text != "[file: a.pdf]" {
		t.Fatalf("a document with a caption reads as a file: %q", text)
	}
}
