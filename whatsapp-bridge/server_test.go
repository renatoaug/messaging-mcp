package main

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"go.mau.fi/whatsmeow/types"
)

type fakeMessenger struct {
	status   Status
	sent     []string
	err      error
	queries  []string
	read     []string
	messages []Stored
}

func (f *fakeMessenger) Find(_ context.Context, query string) ([]Chat, error) {
	f.queries = append(f.queries, query)
	return []Chat{{JID: "5511988887777@s.whatsapp.net", Name: "Alice"}}, f.err
}

func (f *fakeMessenger) Read(_ context.Context, chat types.JID, limit int) ([]Stored, error) {
	f.read = append(f.read, chat.String())
	if len(f.messages) > limit {
		return f.messages[:limit], f.err
	}
	return f.messages, f.err
}

func (f *fakeMessenger) Status() Status { return f.status }

func (f *fakeMessenger) Send(_ context.Context, to types.JID, text string, file *File) (string, error) {
	sent := to.String() + "|" + text
	if file != nil {
		sent += "|" + file.Name + "|" + file.Mime + "|" + string(file.Data)
		if file.Image {
			sent += "|image"
		}
	}
	f.sent = append(f.sent, sent)
	if f.err != nil {
		return "", f.err
	}
	return "ABC123", nil
}

const token = "test-token-0123456789"

func request(t *testing.T, h http.Handler, method, path, auth, body string) (int, map[string]any) {
	t.Helper()
	r := httptest.NewRequest(method, path, strings.NewReader(body))
	if auth != "" {
		r.Header.Set("Authorization", auth)
	}
	w := httptest.NewRecorder()
	h.ServeHTTP(w, r)
	var decoded map[string]any
	_ = json.Unmarshal(w.Body.Bytes(), &decoded)
	return w.Code, decoded
}

func connected() *fakeMessenger {
	return &fakeMessenger{status: Status{Connected: true, Paired: true, JID: "5511999999999:3@s.whatsapp.net"}}
}

func TestEveryRouteRequiresTheToken(t *testing.T) {
	h := newHandler(connected(), token)
	for _, route := range [][2]string{{"GET", "/api/health"}, {"GET", "/api/qr"}, {"POST", "/api/send"}, {"GET", "/api/chats?q=alice"}, {"GET", "/api/messages?chat=5511988887777@s.whatsapp.net&limit=5"}} {
		for _, auth := range []string{"", "Bearer wrong", token} {
			if code, _ := request(t, h, route[0], route[1], auth, "{}"); code != http.StatusUnauthorized {
				t.Fatalf("%s %s with %q: got %d", route[0], route[1], auth, code)
			}
		}
	}
}

func TestHealthReportsTheConnectionAndTheAccount(t *testing.T) {
	code, body := request(t, newHandler(connected(), token), "GET", "/api/health", "Bearer "+token, "")
	if code != http.StatusOK || body["paired"] != true || body["jid"] != "5511999999999:3@s.whatsapp.net" {
		t.Fatalf("got %d %v", code, body)
	}
	code, body = request(t, newHandler(&fakeMessenger{}, token), "GET", "/api/health", "Bearer "+token, "")
	if code != http.StatusServiceUnavailable || body["connected"] != false {
		t.Fatalf("got %d %v", code, body)
	}
}

func TestTheQRCodeIsServedWhileUnpaired(t *testing.T) {
	m := &fakeMessenger{status: Status{QR: "2@abc,def"}}
	_, body := request(t, newHandler(m, token), "GET", "/api/qr", "Bearer "+token, "")
	if body["code"] != "2@abc,def" || body["paired"] != false {
		t.Fatalf("got %v", body)
	}
}

func TestASendGoesOutOnceWithTheExactText(t *testing.T) {
	m := connected()
	text := "Oi!\nSegunda linha com *negrito* do WhatsApp 👋"
	payload, _ := json.Marshal(map[string]string{"recipient": "5511988887777@s.whatsapp.net", "message": text})
	code, body := request(t, newHandler(m, token), "POST", "/api/send", "Bearer "+token, string(payload))
	if code != http.StatusOK || body["success"] != true || body["id"] != "ABC123" {
		t.Fatalf("got %d %v", code, body)
	}
	if len(m.sent) != 1 || m.sent[0] != "5511988887777@s.whatsapp.net|"+text {
		t.Fatalf("sent %v", m.sent)
	}
}

func TestInvalidRequestsAreRefusedWithoutSending(t *testing.T) {
	for _, body := range []string{
		`not json`,
		`{"recipient":"5511988887777@s.whatsapp.net","message":"hi","extra":1}`,
		`{"recipient":"+55 11 98888-7777","message":"hi"}`,
		`{"recipient":"someone@broadcast","message":"hi"}`,
		`{"recipient":"abc@s.whatsapp.net","message":"hi"}`,
		`{"recipient":"5511988887777@s.whatsapp.net","message":"   "}`,
	} {
		m := connected()
		code, _ := request(t, newHandler(m, token), "POST", "/api/send", "Bearer "+token, body)
		if code != http.StatusBadRequest || len(m.sent) != 0 {
			t.Fatalf("%s: got %d, sent %v", body, code, m.sent)
		}
	}
}

func TestADisconnectedBridgeAttemptsNothing(t *testing.T) {
	m := &fakeMessenger{status: Status{Paired: true}}
	payload := `{"recipient":"5511988887777@s.whatsapp.net","message":"hi"}`
	code, _ := request(t, newHandler(m, token), "POST", "/api/send", "Bearer "+token, payload)
	if code != http.StatusConflict || len(m.sent) != 0 {
		t.Fatalf("got %d, sent %v", code, m.sent)
	}
}

func TestAFailedSendIsNeverReportedAsNotSent(t *testing.T) {
	m := connected()
	m.err = errors.New("timeout")
	payload := `{"recipient":"120363000000000000@g.us","message":"hi"}`
	code, body := request(t, newHandler(m, token), "POST", "/api/send", "Bearer "+token, payload)
	if code != http.StatusBadGateway || body["success"] != false || len(m.sent) != 1 {
		t.Fatalf("got %d %v, sent %v", code, body, m.sent)
	}
}

func filePayload(kind, name, data string, caption string) string {
	payload, _ := json.Marshal(map[string]any{
		"recipient": "5511988887777@s.whatsapp.net",
		"message":   caption,
		"file": map[string]string{
			"name": name, "mime": "application/pdf", "kind": kind,
			"data": base64.StdEncoding.EncodeToString([]byte(data)),
		},
	})
	return string(payload)
}

func TestAFileGoesOutWithItsNameAndCaption(t *testing.T) {
	for _, tc := range []struct{ kind, caption, want string }{
		{"document", "", "5511988887777@s.whatsapp.net||report.pdf|application/pdf|%PDF"},
		{"image", "look", "5511988887777@s.whatsapp.net|look|report.pdf|application/pdf|%PDF|image"},
	} {
		m := connected()
		code, body := request(t, newHandler(m, token), "POST", "/api/send", "Bearer "+token, filePayload(tc.kind, "report.pdf", "%PDF", tc.caption))
		if code != http.StatusOK || body["success"] != true || len(m.sent) != 1 || m.sent[0] != tc.want {
			t.Fatalf("%s: got %d %v, sent %v", tc.kind, code, body, m.sent)
		}
	}
}

func TestInvalidFilesAreRefusedWithoutSending(t *testing.T) {
	for _, body := range []string{
		filePayload("video", "a.pdf", "x", ""),
		filePayload("document", "../a.pdf", "x", ""),
		filePayload("document", "", "x", ""),
		filePayload("document", "a.pdf", "", ""),
		strings.Replace(filePayload("document", "a.pdf", "x", ""), `"data":"eA=="`, `"data":"%%%"`, 1),
	} {
		m := connected()
		code, _ := request(t, newHandler(m, token), "POST", "/api/send", "Bearer "+token, body)
		if code != http.StatusBadRequest || len(m.sent) != 0 {
			t.Fatalf("%s: got %d, sent %v", body, code, m.sent)
		}
	}
}

func TestAFailedUploadIsReportedAsNotSent(t *testing.T) {
	m := connected()
	m.err = errUploadFailed
	code, _ := request(t, newHandler(m, token), "POST", "/api/send", "Bearer "+token, filePayload("document", "a.pdf", "x", ""))
	if code != http.StatusUnprocessableEntity {
		t.Fatalf("got %d", code)
	}
}

func TestChatsAreFoundByName(t *testing.T) {
	m := connected()
	code, body := request(t, newHandler(m, token), "GET", "/api/chats?q=nest", "Bearer "+token, "")
	chats, _ := body["chats"].([]any)
	if code != http.StatusOK || len(chats) != 1 || len(m.queries) != 1 || m.queries[0] != "nest" {
		t.Fatalf("got %d %v, queries %v", code, body, m.queries)
	}
	for _, q := range []string{"", "a", strings.Repeat("x", 65)} {
		if code, _ := request(t, newHandler(m, token), "GET", "/api/chats?q="+q, "Bearer "+token, ""); code != http.StatusBadRequest {
			t.Fatalf("%q: got %d", q, code)
		}
	}
}

func TestMessagesAreReadFromOneChat(t *testing.T) {
	m := connected()
	m.messages = []Stored{{From: "Alice", At: 1, Text: "oi"}, {From: "you", FromMe: true, At: 2, Text: "e ai"}}
	code, body := request(t, newHandler(m, token), "GET", "/api/messages?chat=5511988887777@s.whatsapp.net&limit=10", "Bearer "+token, "")
	messages, _ := body["messages"].([]any)
	if code != http.StatusOK || len(messages) != 2 || m.read[0] != "5511988887777@s.whatsapp.net" {
		t.Fatalf("got %d %v, read %v", code, body, m.read)
	}
	first, _ := messages[0].(map[string]any)
	if first["from"] != "Alice" || first["text"] != "oi" || first["fromMe"] != false {
		t.Fatalf("got %v", first)
	}
	for _, query := range []string{"chat=alice&limit=5", "chat=5511988887777@s.whatsapp.net&limit=0", "chat=5511988887777@s.whatsapp.net&limit=51", "chat=5511988887777@s.whatsapp.net"} {
		if code, _ := request(t, newHandler(m, token), "GET", "/api/messages?"+query, "Bearer "+token, ""); code != http.StatusBadRequest {
			t.Fatalf("%s: got %d", query, code)
		}
	}
}
