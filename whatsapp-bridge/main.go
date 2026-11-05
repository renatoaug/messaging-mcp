// Command messaging-mcp-whatsapp-bridge links one personal WhatsApp account as a WhatsApp Web
// device and lets messaging-mcp send text through it, over HTTP on 127.0.0.1 only.
//
// It keeps the device session and nothing else: no message history, no media, no webhooks. It
// never logs message contents. See docs/design/04-whatsapp.md.
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
	mu     sync.Mutex
	qr     string
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

func (w *whatsapp) Send(ctx context.Context, to types.JID, text string) (string, error) {
	response, err := w.client.SendMessage(ctx, to, &waE2E.Message{Conversation: proto.String(text)})
	if err != nil {
		return "", err
	}
	return response.ID, nil
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

	exit := make(chan int, 1)
	client.AddEventHandler(func(event any) {
		switch event.(type) {
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
