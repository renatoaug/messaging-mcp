// Command messaging-mcp-whatsapp-bridge links one personal WhatsApp account as a WhatsApp Web
// device and lets messaging-mcp send text and files through it, over HTTP on 127.0.0.1 only.
//
// It keeps the device session and the text of the last three days of messages (so the owner can
// ask what someone sent), and nothing else: no stored media, no webhooks. A file is held in memory
// only while it is sent. It never logs message contents. See docs/design/04-whatsapp.md.
package main

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"errors"
	"flag"
	"fmt"
	"log"
	"net/http"
	"os"
	"os/signal"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"syscall"
	"time"

	"go.mau.fi/whatsmeow"
	"go.mau.fi/whatsmeow/proto/waE2E"
	"go.mau.fi/whatsmeow/store"
	"go.mau.fi/whatsmeow/store/sqlstore"
	"go.mau.fi/whatsmeow/types"
	"go.mau.fi/whatsmeow/types/events"
	waLog "go.mau.fi/whatsmeow/util/log"
	"google.golang.org/protobuf/proto"
	_ "modernc.org/sqlite"
)

// Exit codes the supervisor reads.
const (
	exitPairingTimedOut = 4
	exitLoggedOut       = 5
)

type whatsapp struct {
	client *whatsmeow.Client
	inbox  *Inbox
	mu     sync.Mutex
	qr     string
}

var errNoInbox = errors.New("messages are not available")

// chatOf is the chat a message belongs to, by phone number when WhatsApp gave a hidden ID, so the
// same person is one chat however their messages arrived.
func (w *whatsapp) chatOf(ctx context.Context, chat types.JID) string {
	chat = chat.ToNonAD()
	if chat.Server == types.HiddenUserServer {
		if pn, err := w.client.Store.LIDs.GetPNForLID(ctx, chat); err == nil && !pn.IsEmpty() {
			chat = pn.ToNonAD()
		}
	}
	return chat.String()
}

// keep stores a received or sent message for three days. Its contents are never logged.
func (w *whatsapp) keep(ctx context.Context, info types.MessageInfo, message *waE2E.Message) {
	if w.inbox == nil || info.Chat.Server == types.BroadcastServer {
		return
	}
	from := info.PushName
	if info.IsFromMe {
		from = "you"
	}
	stored := Stored{
		Chat: w.chatOf(ctx, info.Chat), ID: info.ID, From: from, FromMe: info.IsFromMe,
		At: info.Timestamp.UnixMilli(), Text: messageText(message),
	}
	if err := w.inbox.Add(ctx, stored); err != nil {
		log.Print("could not keep a message")
	}
}

func (w *whatsapp) Find(ctx context.Context, query string) ([]Chat, error) {
	want := fold(query)
	found := map[string]string{}
	contacts, err := w.client.Store.Contacts.GetAllContacts(ctx)
	if err != nil {
		return nil, err
	}
	for jid, info := range contacts {
		for _, name := range []string{info.FullName, info.FirstName, info.PushName, info.BusinessName} {
			if name != "" && strings.Contains(fold(name), want) {
				found[w.chatOf(ctx, jid)] = name
				break
			}
		}
	}
	if groups, err := w.client.GetJoinedGroups(ctx); err == nil {
		for _, group := range groups {
			if strings.Contains(fold(group.Name), want) {
				found[group.JID.String()] = group.Name
			}
		}
	}
	if w.inbox != nil {
		if senders, err := w.inbox.Senders(ctx); err == nil {
			for chat, name := range senders {
				if _, known := found[chat]; !known && strings.Contains(fold(name), want) {
					found[chat] = name
				}
			}
		}
	}
	chats := make([]Chat, 0, len(found))
	for jid, name := range found {
		chats = append(chats, Chat{JID: jid, Name: name})
	}
	sort.Slice(chats, func(i, j int) bool { return chats[i].Name < chats[j].Name })
	if len(chats) > 20 {
		chats = chats[:20]
	}
	return chats, nil
}

func (w *whatsapp) Read(ctx context.Context, chat types.JID, limit int) ([]Stored, error) {
	if w.inbox == nil {
		return nil, errNoInbox
	}
	return w.inbox.Recent(ctx, chat.ToNonAD().String(), limit)
}

func (w *whatsapp) setQR(code string) {
	w.mu.Lock()
	defer w.mu.Unlock()
	w.qr = code
}

func (w *whatsapp) Status() Status {
	w.mu.Lock()
	qr := w.qr
	w.mu.Unlock()
	s := Status{Connected: w.client.IsConnected() && w.client.IsLoggedIn(), QR: qr}
	if id := w.client.Store.ID; id != nil {
		s.Paired = true
		s.JID = id.String()
	}
	return s
}

func (w *whatsapp) Send(ctx context.Context, to types.JID, text string, file *File) (string, error) {
	message := &waE2E.Message{Conversation: proto.String(text)}
	if file != nil {
		var err error
		if message, err = w.fileMessage(ctx, text, file); err != nil {
			return "", err
		}
	}
	response, err := w.client.SendMessage(ctx, to, message)
	if err != nil {
		return "", err
	}
	info := types.MessageInfo{
		MessageSource: types.MessageSource{Chat: to, IsFromMe: true},
		ID:            response.ID, Timestamp: response.Timestamp,
	}
	w.keep(context.WithoutCancel(ctx), info, message)
	return response.ID, nil
}

// fileMessage uploads the file, encrypted, to WhatsApp's media servers and builds the message that
// points at it. A failure here happens before any message exists.
func (w *whatsapp) fileMessage(ctx context.Context, caption string, file *File) (*waE2E.Message, error) {
	kind := whatsmeow.MediaDocument
	if file.Image {
		kind = whatsmeow.MediaImage
	}
	up, err := w.client.Upload(ctx, file.Data, kind)
	if err != nil {
		return nil, errUploadFailed
	}
	var text *string
	if caption != "" {
		text = proto.String(caption)
	}
	if file.Image {
		return &waE2E.Message{ImageMessage: &waE2E.ImageMessage{
			Caption: text, Mimetype: proto.String(file.Mime), URL: proto.String(up.URL),
			DirectPath: proto.String(up.DirectPath), MediaKey: up.MediaKey,
			FileEncSHA256: up.FileEncSHA256, FileSHA256: up.FileSHA256, FileLength: proto.Uint64(up.FileLength),
		}}, nil
	}
	return &waE2E.Message{DocumentMessage: &waE2E.DocumentMessage{
		Caption: text, Mimetype: proto.String(file.Mime), FileName: proto.String(file.Name),
		Title: proto.String(file.Name), URL: proto.String(up.URL), DirectPath: proto.String(up.DirectPath),
		MediaKey: up.MediaKey, FileEncSHA256: up.FileEncSHA256, FileSHA256: up.FileSHA256,
		FileLength: proto.Uint64(up.FileLength),
	}}, nil
}

// loadToken reads the bridge token, creating a random one (0600) on first start.
func loadToken(storeDir string) (string, error) {
	if value := strings.TrimSpace(os.Getenv("WHATSAPP_BRIDGE_TOKEN")); value != "" {
		return value, nil
	}
	path := filepath.Join(storeDir, ".bridge-token")
	if data, err := os.ReadFile(path); err == nil {
		if value := strings.TrimSpace(string(data)); value != "" {
			return value, nil
		}
	}
	buffer := make([]byte, 32)
	if _, err := rand.Read(buffer); err != nil {
		return "", err
	}
	value := hex.EncodeToString(buffer)
	file, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	if err != nil {
		return "", err
	}
	defer file.Close()
	_, err = file.WriteString(value + "\n")
	return value, err
}

func run() error {
	storeDir := flag.String("store", "store", "folder for the session and the token, created 0700")
	port := flag.Int("port", 8080, "port on 127.0.0.1")
	flag.Parse()
	if err := os.MkdirAll(*storeDir, 0o700); err != nil {
		return err
	}
	if err := os.Chmod(*storeDir, 0o700); err != nil {
		return err
	}
	token, err := loadToken(*storeDir)
	if err != nil {
		return fmt.Errorf("token: %w", err)
	}

	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
	defer stop()

	store.DeviceProps.Os = proto.String("Messaging MCP")
	address := "file:" + filepath.Join(*storeDir, "session.db") + "?_pragma=foreign_keys(1)&_pragma=busy_timeout(5000)"
	container, err := sqlstore.New(ctx, "sqlite", address, waLog.Noop)
	if err != nil {
		return fmt.Errorf("session store: %w", err)
	}
	device, err := container.GetFirstDevice(ctx)
	if err != nil {
		return fmt.Errorf("device: %w", err)
	}
	client := whatsmeow.NewClient(device, waLog.Noop)
	bridge := &whatsapp{client: client}
	inbox, err := OpenInbox(ctx, filepath.Join(*storeDir, "messages.db"))
	if err != nil {
		// Sending still works; only reading is unavailable.
		log.Print("messages store unavailable")
	} else {
		bridge.inbox = inbox
		defer inbox.Close()
	}

	exit := make(chan int, 1)
	client.AddEventHandler(func(event any) {
		switch event := event.(type) {
		case *events.Message:
			bridge.keep(context.Background(), event.Info, event.Message)
		case *events.PairSuccess:
			bridge.setQR("")
			log.Print("paired")
		case *events.Connected:
			log.Print("connected")
		case *events.LoggedOut:
			// Unlinked from the phone: forget the session so the next start pairs again.
			log.Print("logged out")
			_ = client.Store.Delete(context.Background())
			exit <- exitLoggedOut
		}
	})

	if client.Store.ID == nil {
		codes, err := client.GetQRChannel(ctx)
		if err != nil {
			return fmt.Errorf("pairing: %w", err)
		}
		go func() {
			for item := range codes {
				switch item.Event {
				case "code":
					bridge.setQR(item.Code)
					log.Print("waiting for the pairing scan")
				case "success":
					bridge.setQR("")
				default:
					bridge.setQR("")
					log.Printf("pairing ended: %s", item.Event)
					exit <- exitPairingTimedOut
					return
				}
			}
		}()
	}
	if err := client.Connect(); err != nil {
		return fmt.Errorf("connect: %w", err)
	}
	defer client.Disconnect()

	server := &http.Server{
		Addr:              fmt.Sprintf("127.0.0.1:%d", *port),
		Handler:           newHandler(bridge, token),
		ReadHeaderTimeout: 10 * time.Second,
	}
	serverErr := make(chan error, 1)
	go func() { serverErr <- server.ListenAndServe() }()
	log.Printf("listening on %s", server.Addr)

	code := 0
	select {
	case <-ctx.Done():
	case code = <-exit:
	case err := <-serverErr:
		if !errors.Is(err, http.ErrServerClosed) {
			return fmt.Errorf("http: %w", err)
		}
	}
	shutdown, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	_ = server.Shutdown(shutdown)
	if code != 0 {
		client.Disconnect()
		os.Exit(code)
	}
	return nil
}

func main() {
	log.SetFlags(log.LstdFlags)
	if err := run(); err != nil {
		log.Print(err)
		os.Exit(1)
	}
}
