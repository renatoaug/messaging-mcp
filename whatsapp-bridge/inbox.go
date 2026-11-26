package main

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"os"
	"strings"
	"time"

	"go.mau.fi/whatsmeow"
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
	// Attachment is the message ID when it has a photo, video or file to download.
	Attachment string `json:"attachment,omitempty"`
	// Media is the encrypted media's keys and location (a waE2E.Message), never the file.
	Media []byte `json:"-"`
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
	if err == nil {
		err = addMediaColumn(ctx, db)
	}
	if err != nil {
		db.Close()
		return nil, err
	}
	inbox := &Inbox{db: db, now: time.Now}
	return inbox, inbox.Prune(ctx)
}

// addMediaColumn adds the media column to a database made before files could be saved.
func addMediaColumn(ctx context.Context, db *sql.DB) error {
	var found int
	err := db.QueryRowContext(ctx, "SELECT COUNT(*) FROM pragma_table_info('message') WHERE name = 'media'").Scan(&found)
	if err != nil || found > 0 {
		return err
	}
	_, err = db.ExecContext(ctx, "ALTER TABLE message ADD COLUMN media BLOB")
	return err
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
		"INSERT OR IGNORE INTO message (chat, id, sender, from_me, at, text, media) VALUES (?, ?, ?, ?, ?, ?, ?)",
		m.Chat, m.ID, m.From, m.FromMe, m.At, m.Text, m.Media)
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
		"SELECT id, sender, from_me, at, text, media IS NOT NULL FROM message WHERE chat = ? AND at >= ? ORDER BY at DESC LIMIT ?",
		chat, in.now().Add(-retention).UnixMilli(), limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	messages := []Stored{}
	for rows.Next() {
		var m Stored
		var media bool
		if err := rows.Scan(&m.ID, &m.From, &m.FromMe, &m.At, &m.Text, &media); err != nil {
			return nil, err
		}
		if media {
			m.Attachment = m.ID
		}
		messages = append(messages, m)
	}
	for i, j := 0, len(messages)-1; i < j; i, j = i+1, j-1 {
		messages[i], messages[j] = messages[j], messages[i]
	}
	return messages, rows.Err()
}

// Media returns a kept message's media keys, or nil when it has none or is too old.
func (in *Inbox) Media(ctx context.Context, chat, id string) ([]byte, error) {
	var media []byte
	err := in.db.QueryRowContext(ctx,
		"SELECT media FROM message WHERE chat = ? AND id = ? AND at >= ?",
		chat, id, in.now().Add(-retention).UnixMilli()).Scan(&media)
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	return media, err
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
	case m.GetDocumentWithCaptionMessage() != nil:
		return messageText(m.GetDocumentWithCaptionMessage().GetMessage())
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

// mediaOf keeps only the photo, video or document of a message, to download it later. Voice
// messages are transcribed instead, and stickers are left out.
func mediaOf(m *waE2E.Message) *waE2E.Message {
	if m == nil {
		return nil
	}
	switch {
	case m.GetImageMessage() != nil:
		return &waE2E.Message{ImageMessage: m.GetImageMessage()}
	case m.GetVideoMessage() != nil:
		return &waE2E.Message{VideoMessage: m.GetVideoMessage()}
	case m.GetDocumentMessage() != nil:
		return &waE2E.Message{DocumentMessage: m.GetDocumentMessage()}
	case m.GetDocumentWithCaptionMessage() != nil:
		return mediaOf(m.GetDocumentWithCaptionMessage().GetMessage())
	case m.GetEphemeralMessage() != nil:
		return mediaOf(m.GetEphemeralMessage().GetMessage())
	}
	return nil
}

// mediaFile is what the owner gets for a kept media message.
type mediaFile struct {
	name string
	mime string
	size uint64
}

// describeMedia names the file in a kept media message: a document keeps its own name, a photo or
// video gets one from the message ID.
func describeMedia(m *waE2E.Message, id string) (mediaFile, whatsmeow.DownloadableMessage) {
	switch {
	case m.GetImageMessage() != nil:
		image := m.GetImageMessage()
		return mediaFile{name: "photo-" + id + extension(image.GetMimetype(), ".jpg"), mime: image.GetMimetype(), size: image.GetFileLength()}, image
	case m.GetVideoMessage() != nil:
		video := m.GetVideoMessage()
		return mediaFile{name: "video-" + id + extension(video.GetMimetype(), ".mp4"), mime: video.GetMimetype(), size: video.GetFileLength()}, video
	case m.GetDocumentMessage() != nil:
		document := m.GetDocumentMessage()
		name := document.GetFileName()
		if name == "" {
			name = "file-" + id + extension(document.GetMimetype(), "")
		}
		return mediaFile{name: name, mime: document.GetMimetype(), size: document.GetFileLength()}, document
	}
	return mediaFile{}, nil
}

// extension is the usual extension for a MIME type, or fallback.
func extension(mime, fallback string) string {
	switch strings.SplitN(mime, ";", 2)[0] {
	case "image/jpeg":
		return ".jpg"
	case "image/png":
		return ".png"
	case "image/webp":
		return ".webp"
	case "video/mp4":
		return ".mp4"
	case "application/pdf":
		return ".pdf"
	}
	return fallback
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
