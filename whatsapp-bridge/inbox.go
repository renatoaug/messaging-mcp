package main

import (
	"context"
	"database/sql"
	"fmt"
	"os"
	"strings"
	"time"

	"go.mau.fi/whatsmeow/proto/waE2E"
	"golang.org/x/text/unicode/norm"
)

// retention is how long messages are kept so the owner can ask what someone sent. WhatsApp gives
// a linked device no way to fetch older messages, so the bridge keeps what arrives, briefly.
const retention = 72 * time.Hour

// Stored is one message the bridge saw, received or sent.
type Stored struct {
	Chat   string `json:"-"`
	ID     string `json:"-"`
	From   string `json:"from"`
	FromMe bool   `json:"fromMe"`
	At     int64  `json:"at"`
	Text   string `json:"text"`
}

// Inbox keeps the last three days of messages in an owner-only SQLite file.
type Inbox struct {
	db  *sql.DB
	now func() time.Time
}

// OpenInbox opens (or creates, 0600) the messages database.
func OpenInbox(ctx context.Context, path string) (*Inbox, error) {
	file, err := os.OpenFile(path, os.O_RDWR|os.O_CREATE, 0o600)
	if err != nil {
		return nil, err
	}
	file.Close()
	if err := os.Chmod(path, 0o600); err != nil {
		return nil, err
	}
	db, err := sql.Open("sqlite", "file:"+path+"?_pragma=busy_timeout(5000)&_pragma=journal_mode(WAL)")
	if err != nil {
		return nil, err
	}
	db.SetMaxOpenConns(1)
	_, err = db.ExecContext(ctx, `CREATE TABLE IF NOT EXISTS message (
		chat TEXT NOT NULL, id TEXT NOT NULL, sender TEXT NOT NULL, from_me INTEGER NOT NULL,
		at INTEGER NOT NULL, text TEXT NOT NULL, PRIMARY KEY (chat, id));
		CREATE INDEX IF NOT EXISTS message_at ON message (at);`)
	if err != nil {
		db.Close()
		return nil, err
	}
	inbox := &Inbox{db: db, now: time.Now}
	return inbox, inbox.Prune(ctx)
}

// Close closes the database.
func (in *Inbox) Close() error { return in.db.Close() }

// Prune deletes what is older than the retention.
func (in *Inbox) Prune(ctx context.Context) error {
	_, err := in.db.ExecContext(ctx, "DELETE FROM message WHERE at < ?", in.now().Add(-retention).UnixMilli())
	return err
}

// Add keeps a message once; the same ID again is ignored.
func (in *Inbox) Add(ctx context.Context, m Stored) error {
	if m.Text == "" || in.now().Sub(time.UnixMilli(m.At)) > retention {
		return nil
	}
	_, err := in.db.ExecContext(ctx,
		"INSERT OR IGNORE INTO message (chat, id, sender, from_me, at, text) VALUES (?, ?, ?, ?, ?, ?)",
		m.Chat, m.ID, m.From, m.FromMe, m.At, m.Text)
	if err != nil {
		return err
	}
	return in.Prune(ctx)
}

// SetText replaces a kept message's text, as when its voice message was transcribed.
func (in *Inbox) SetText(ctx context.Context, chat, id, text string) error {
	_, err := in.db.ExecContext(ctx, "UPDATE message SET text = ? WHERE chat = ? AND id = ?", text, chat, id)
	return err
}

// Recent returns a chat's last messages, oldest first.
func (in *Inbox) Recent(ctx context.Context, chat string, limit int) ([]Stored, error) {
	rows, err := in.db.QueryContext(ctx,
		"SELECT sender, from_me, at, text FROM message WHERE chat = ? AND at >= ? ORDER BY at DESC LIMIT ?",
		chat, in.now().Add(-retention).UnixMilli(), limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	messages := []Stored{}
	for rows.Next() {
		var m Stored
		if err := rows.Scan(&m.From, &m.FromMe, &m.At, &m.Text); err != nil {
			return nil, err
		}
		messages = append(messages, m)
	}
	for i, j := 0, len(messages)-1; i < j; i, j = i+1, j-1 {
		messages[i], messages[j] = messages[j], messages[i]
	}
	return messages, rows.Err()
}

// Senders returns, per chat, the latest name a sender gave, for finding a chat by name.
func (in *Inbox) Senders(ctx context.Context) (map[string]string, error) {
	rows, err := in.db.QueryContext(ctx,
		"SELECT chat, sender FROM message WHERE from_me = 0 ORDER BY at")
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	names := map[string]string{}
	for rows.Next() {
		var chat, sender string
		if err := rows.Scan(&chat, &sender); err != nil {
			return nil, err
		}
		names[chat] = sender
	}
	return names, rows.Err()
}

// messageText is what the owner reads: the text, or a short label for media. Reactions, edits
// and other protocol messages return "" and are not kept.
func messageText(m *waE2E.Message) string {
	if m == nil {
		return ""
	}
	label := func(kind, caption string) string {
		if caption == "" {
			return kind
		}
		return kind + " " + caption
	}
	switch {
	case m.GetConversation() != "":
		return m.GetConversation()
	case m.GetExtendedTextMessage() != nil:
		return m.GetExtendedTextMessage().GetText()
	case m.GetImageMessage() != nil:
		return label("[photo]", m.GetImageMessage().GetCaption())
	case m.GetVideoMessage() != nil:
		return label("[video]", m.GetVideoMessage().GetCaption())
	case m.GetDocumentMessage() != nil:
		d := m.GetDocumentMessage()
		return label(fmt.Sprintf("[file: %s]", d.GetFileName()), d.GetCaption())
	case m.GetAudioMessage() != nil:
		return voiceText(m.GetAudioMessage().GetSeconds(), nil)
	case m.GetStickerMessage() != nil:
		return "[sticker]"
	case m.GetContactMessage() != nil:
		return "[contact]"
	case m.GetLocationMessage() != nil:
		return "[location]"
	case m.GetEphemeralMessage() != nil:
		return messageText(m.GetEphemeralMessage().GetMessage())
	case m.GetViewOnceMessage() != nil:
		return "[view once]"
	}
	return ""
}

// voiceText is how a voice message reads: its length, then what was said. It matches
// messaging-mcp's own label for Telegram.
func voiceText(seconds uint32, transcript *string) string {
	label := fmt.Sprintf("[audio %d:%02d]", seconds/60, seconds%60)
	switch {
	case transcript == nil:
		return label + " (not transcribed)"
	case *transcript == "":
		return label + " (nothing said)"
	}
	return label + " " + *transcript
}

// fold lowercases and removes accents, so "joao" finds "João".
func fold(s string) string {
	var b strings.Builder
	for _, r := range norm.NFD.String(strings.ToLower(s)) {
		if r < 0x300 || r > 0x36f {
			b.WriteRune(r)
		}
	}
	return b.String()
}
