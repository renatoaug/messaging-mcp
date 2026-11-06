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
		m := Stored{Chat: chat, ID: id, From: "Nestor", At: now.Add(-age).UnixMilli(), Text: text}
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
	if names, _ := inbox.Senders(ctx); names[chat] != "Nestor" {
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
		{&waE2E.Message{AudioMessage: &waE2E.AudioMessage{}}, "[audio]"},
		{&waE2E.Message{ReactionMessage: &waE2E.ReactionMessage{Text: proto.String("x")}}, ""},
		{nil, ""},
	} {
		if got := messageText(tc.message); got != tc.want {
			t.Fatalf("got %q, want %q", got, tc.want)
		}
	}
	if fold("Néstor") != "nestor" {
		t.Fatalf("fold: %q", fold("Néstor"))
	}
}
