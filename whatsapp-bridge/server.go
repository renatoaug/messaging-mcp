package main

import (
	"context"
	"crypto/subtle"
	"encoding/json"
	"errors"
	"net/http"
	"strings"
	"time"
	"unicode/utf8"

	"go.mau.fi/whatsmeow/types"
)

// maxBodyBytes bounds a send request: a 4096-character message is at most 16 KB of UTF-8.
const maxBodyBytes = 64 << 10

// sendTimeout bounds one send. After it, the outcome is unknown to the caller.
const sendTimeout = 30 * time.Second

// Status is what the bridge knows about its WhatsApp connection.
type Status struct {
	Connected bool
	Paired    bool
	// JID is the paired account, such as 5511999999999:12@s.whatsapp.net.
	JID string
	// QR is the pairing code to show while unpaired, empty otherwise.
	QR string
}

// Messenger is the WhatsApp side of the bridge. The HTTP layer depends only on this, so it can
// be tested without an account.
type Messenger interface {
	Status() Status
	Send(ctx context.Context, to types.JID, text string) (string, error)
}

type sendRequest struct {
	Recipient string `json:"recipient"`
	Message   string `json:"message"`
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
// send: 400, 401 and 409 mean nothing was attempted; any other failure may have been sent.
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
		if err != nil || strings.TrimSpace(text) == "" || !utf8.ValidString(text) {
			writeJSON(w, http.StatusBadRequest, map[string]any{"success": false})
			return
		}
		if !m.Status().Connected {
			// Checked before sending: nothing was attempted.
			writeJSON(w, http.StatusConflict, map[string]any{"success": false})
			return
		}
		ctx, cancel := context.WithTimeout(r.Context(), sendTimeout)
		defer cancel()
		id, err := m.Send(ctx, to, text)
		if err != nil {
			// The message may have left before the error: never report it as not sent.
			writeJSON(w, http.StatusBadGateway, map[string]any{"success": false})
			return
		}
		writeJSON(w, http.StatusOK, map[string]any{"success": true, "id": id})
	}))
	return mux
}
