package main

import (
	"context"
	"crypto/subtle"
	"encoding/base64"
	"encoding/json"
	"errors"
	"net/http"
	"strconv"
	"strings"
	"time"
	"unicode/utf8"

	"go.mau.fi/whatsmeow/types"
)

// maxFileBytes is the largest file messaging-mcp sends (it checks the same limit first).
const maxFileBytes = 50 << 20

// maxBodyBytes bounds a send request: the file in base64 plus a 4096-character message.
const maxBodyBytes = maxFileBytes/3*4 + 1<<20

// sendTimeout bounds one text send, and fileSendTimeout one upload and send. After either, the
// outcome is unknown to the caller.
const (
	sendTimeout     = 30 * time.Second
	fileSendTimeout = 5 * time.Minute
)

// errUploadFailed means the file never reached WhatsApp's media servers, so no message was sent.
var errUploadFailed = errors.New("upload failed")

// Status is what the bridge knows about its WhatsApp connection.
type Status struct {
	Connected bool
	Paired    bool
	// JID is the paired account, such as 5511999999999:12@s.whatsapp.net.
	JID string
	// QR is the pairing code to show while unpaired, empty otherwise.
	QR string
}

// File is an attachment: an image is sent as a photo, anything else as a document.
type File struct {
	Name  string
	Mime  string
	Image bool
	Data  []byte
}

// Chat is a person or group found by name.
type Chat struct {
	JID  string `json:"jid"`
	Name string `json:"name"`
}

// Messenger is the WhatsApp side of the bridge. The HTTP layer depends only on this, so it can
// be tested without an account. Send returns errUploadFailed when a file upload failed before
// any message was sent.
type Messenger interface {
	Status() Status
	Send(ctx context.Context, to types.JID, text string, file *File) (string, error)
	// Find looks up the owner's contacts, groups and recent chats by name. Read only.
	Find(ctx context.Context, query string) ([]Chat, error)
	// Read returns a chat's messages from the last three days, oldest first.
	Read(ctx context.Context, chat types.JID, limit int) ([]Stored, error)
}

type fileRequest struct {
	Name string `json:"name"`
	Mime string `json:"mime"`
	Kind string `json:"kind"`
	Data string `json:"data"`
}

type sendRequest struct {
	Recipient string       `json:"recipient"`
	Message   string       `json:"message"`
	File      *fileRequest `json:"file"`
}

// parseFile checks an attachment. The name must be a plain file name: WhatsApp shows it to the
// recipient.
func parseFile(f *fileRequest) (*File, error) {
	invalid := errors.New("invalid file")
	if f.Kind != "image" && f.Kind != "document" {
		return nil, invalid
	}
	if f.Name == "" || len(f.Name) > 255 || strings.ContainsAny(f.Name, "/\\\x00") || !utf8.ValidString(f.Name) {
		return nil, invalid
	}
	if f.Mime == "" || len(f.Mime) > 127 || !strings.Contains(f.Mime, "/") {
		return nil, invalid
	}
	data, err := base64.StdEncoding.DecodeString(f.Data)
	if err != nil || len(data) == 0 || len(data) > maxFileBytes {
		return nil, invalid
	}
	return &File{Name: f.Name, Mime: f.Mime, Image: f.Kind == "image", Data: data}, nil
}

func writeJSON(w http.ResponseWriter, code int, body any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(code)
	_ = json.NewEncoder(w).Encode(body)
}

// requireToken accepts only `Authorization: Bearer <token>`, compared in constant time.
func requireToken(token string, next http.HandlerFunc) http.HandlerFunc {
	expected := []byte("Bearer " + token)
	return func(w http.ResponseWriter, r *http.Request) {
		given := []byte(r.Header.Get("Authorization"))
		if subtle.ConstantTimeCompare(given, expected) != 1 {
			writeJSON(w, http.StatusUnauthorized, map[string]any{"success": false})
			return
		}
		next(w, r)
	}
}

// parseRecipient accepts only a phone number JID or a group JID, the targets messaging-mcp allows.
func parseRecipient(value string) (types.JID, error) {
	jid, err := types.ParseJID(value)
	if err != nil || jid.User == "" {
		return types.JID{}, errors.New("invalid recipient")
	}
	if jid.Server != types.DefaultUserServer && jid.Server != types.GroupServer {
		return types.JID{}, errors.New("invalid recipient")
	}
	for _, r := range strings.ReplaceAll(jid.User, "-", "") {
		if r < '0' || r > '9' {
			return types.JID{}, errors.New("invalid recipient")
		}
	}
	return jid, nil
}

// newHandler serves the three routes messaging-mcp uses. Status codes carry what is known about a
// send: 400, 401, 409 and 422 (the file upload failed) mean nothing was sent; any other failure
// may have been sent.
func newHandler(m Messenger, token string) http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("GET /api/health", requireToken(token, func(w http.ResponseWriter, r *http.Request) {
		s := m.Status()
		code := http.StatusOK
		if !s.Connected {
			code = http.StatusServiceUnavailable
		}
		writeJSON(w, code, map[string]any{"connected": s.Connected, "paired": s.Paired, "jid": s.JID})
	}))
	mux.HandleFunc("GET /api/qr", requireToken(token, func(w http.ResponseWriter, r *http.Request) {
		s := m.Status()
		writeJSON(w, http.StatusOK, map[string]any{"code": s.QR, "paired": s.Paired, "connected": s.Connected})
	}))
	mux.HandleFunc("GET /api/chats", requireToken(token, func(w http.ResponseWriter, r *http.Request) {
		query := strings.TrimSpace(r.URL.Query().Get("q"))
		if n := utf8.RuneCountInString(query); n < 2 || n > 64 {
			writeJSON(w, http.StatusBadRequest, map[string]any{"chats": []Chat{}})
			return
		}
		chats, err := m.Find(r.Context(), query)
		if err != nil {
			writeJSON(w, http.StatusServiceUnavailable, map[string]any{"chats": []Chat{}})
			return
		}
		writeJSON(w, http.StatusOK, map[string]any{"chats": chats})
	}))
	mux.HandleFunc("GET /api/messages", requireToken(token, func(w http.ResponseWriter, r *http.Request) {
		chat, err := parseRecipient(r.URL.Query().Get("chat"))
		limit, limitErr := strconv.Atoi(r.URL.Query().Get("limit"))
		if err != nil || limitErr != nil || limit < 1 || limit > 50 {
			writeJSON(w, http.StatusBadRequest, map[string]any{"messages": []Stored{}})
			return
		}
		messages, err := m.Read(r.Context(), chat, limit)
		if err != nil {
			writeJSON(w, http.StatusServiceUnavailable, map[string]any{"messages": []Stored{}})
			return
		}
		writeJSON(w, http.StatusOK, map[string]any{"messages": messages})
	}))
	mux.HandleFunc("POST /api/send", requireToken(token, func(w http.ResponseWriter, r *http.Request) {
		var request sendRequest
		decoder := json.NewDecoder(http.MaxBytesReader(w, r.Body, maxBodyBytes))
		decoder.DisallowUnknownFields()
		if err := decoder.Decode(&request); err != nil {
			writeJSON(w, http.StatusBadRequest, map[string]any{"success": false})
			return
		}
		to, err := parseRecipient(request.Recipient)
		text := request.Message
		var file *File
		if err == nil && request.File != nil {
			file, err = parseFile(request.File)
		}
		// A file may go without a caption; a text message needs text.
		blank := strings.TrimSpace(text) == "" && file == nil
		if err != nil || blank || !utf8.ValidString(text) {
			writeJSON(w, http.StatusBadRequest, map[string]any{"success": false})
			return
		}
		if !m.Status().Connected {
			// Checked before sending: nothing was attempted.
			writeJSON(w, http.StatusConflict, map[string]any{"success": false})
			return
		}
		limit := sendTimeout
		if file != nil {
			limit = fileSendTimeout
		}
		ctx, cancel := context.WithTimeout(r.Context(), limit)
		defer cancel()
		id, err := m.Send(ctx, to, text, file)
		if errors.Is(err, errUploadFailed) {
			writeJSON(w, http.StatusUnprocessableEntity, map[string]any{"success": false})
			return
		}
		if err != nil {
			// The message may have left before the error: never report it as not sent.
			writeJSON(w, http.StatusBadGateway, map[string]any{"success": false})
			return
		}
		writeJSON(w, http.StatusOK, map[string]any{"success": true, "id": id})
	}))
	return mux
}
