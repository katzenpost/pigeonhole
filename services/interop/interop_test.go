// Package interop runs the services against a real client daemon, real
// couriers and storage replicas, and the Go voucher router and service
// directory, all in one process with katzenpost's localnet.
package interop

import (
	"bufio"
	"context"
	"encoding/base64"
	"encoding/hex"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/fxamacker/cbor/v2"
	"github.com/stretchr/testify/require"

	"github.com/katzenpost/katzenpost/client/localnet"
	ps "github.com/katzenpost/katzenpost/pigeonservice"
	"github.com/katzenpost/katzenpost/pigeonservice/directory"
	"github.com/katzenpost/katzenpost/pigeonservice/router"

	client "github.com/katzenpost/pigeonhole_protocols/clients/motd"
	"github.com/katzenpost/pigeonhole_protocols/services/motd"
)

const poll = 200 * time.Millisecond

func pluginKey(t *testing.T) *ps.PluginKey {
	pub, priv, err := ps.DefaultNIKE.GenerateKeyPair()
	require.NoError(t, err)
	return &ps.PluginKey{Scheme: ps.DefaultNIKE, Public: pub, Private: priv}
}

// world is a localnet with storage, a voucher router and a directory.
func world(t *testing.T) *localnet.Network {
	n := localnet.New(t)
	n.AddStorage(3)
	routerKey, dirKey := pluginKey(t), pluginKey(t)
	var r *router.Router
	var d *directory.Directory
	var ready sync.WaitGroup
	ready.Add(1)
	n.AddHandler(ps.RouterCapability, ps.PluginParams(routerKey), func(p []byte) []byte {
		ready.Wait()
		return r.Handle(p)
	})
	n.AddHandler(ps.DirectoryCapability, ps.PluginParams(dirKey), func(p []byte) []byte {
		ready.Wait()
		return d.Handle(p)
	})
	n.Start()
	var err error
	r, err = router.New(router.Config{
		Key: routerKey, DBPath: filepath.Join(t.TempDir(), "router.db"),
		PollInterval: poll, Retention: time.Hour, Log: n.Log.GetLogger("router"),
	}, ps.NewThinPigeonhole(n.ThinClient()))
	require.NoError(t, err)
	t.Cleanup(func() { r.Close() })
	d, err = directory.New(directory.Config{
		Key: dirKey, DBPath: filepath.Join(t.TempDir(), "directory.db"),
		RefreshInterval: time.Hour, Log: n.Log.GetLogger("directory"),
	}, ps.NewThinPigeonhole(n.ThinClient()))
	require.NoError(t, err)
	t.Cleanup(func() { d.Close() })
	ready.Done()
	return n
}

func lookupLinked(t *testing.T, ctx context.Context, ph ps.Pigeonhole, id ps.ServiceID) *ps.ServiceDescriptor {
	reader, err := directory.NewAll(ph, nil, poll)
	require.NoError(t, err)
	for {
		d, err := reader.Lookup(ctx, id)
		if err == nil && len(d.Routers) > 0 {
			return d
		}
		require.NoError(t, ctx.Err(), "the directory never listed the service")
		time.Sleep(poll)
	}
}

func TestMOTD(t *testing.T) {
	if testing.Short() {
		t.Skip("runs real CTIDH replicas; a few minutes")
	}
	n := world(t)
	ctx, cancel := context.WithTimeout(context.Background(), 15*time.Minute)
	t.Cleanup(cancel)

	ph := ps.NewThinPigeonhole(n.ThinClient())
	dataDir := t.TempDir()
	key, err := ps.LoadServiceKey(dataDir)
	require.NoError(t, err)
	adv, err := directory.NewAll(ph, key, poll)
	require.NoError(t, err)
	svc, err := ps.NewService(ps.Config{
		Key: key, DataDir: dataDir, DisplayName: "motd", Advertiser: adv,
		PollInterval: poll, Log: n.Log.GetLogger("motd"),
	}, ph)
	require.NoError(t, err)
	go svc.Run(ctx, ps.HandlerFunc(func(ctx context.Context, s *ps.Session) {
		_ = motd.Serve(ctx, s, func() (string, error) { return "hello from localnet", nil })
	}))
	t.Cleanup(func() { cancel(); svc.Close() })

	clientPh := ps.NewThinPigeonhole(n.ThinClient())
	d := lookupLinked(t, ctx, clientPh, svc.ID())
	conn, err := ps.Dial(ctx, clientPh, d, "client", poll)
	require.NoError(t, err)
	reply, err := client.Fetch(ctx, conn)
	require.NoError(t, err)
	require.Equal(t, "hello from localnet", reply.Message)
}

// The katzenqt chat framing: 'F' and a CBOR GroupChatMessage.
func chatText(t *testing.T, text string) []byte {
	b, err := cbor.Marshal(map[string]any{"version": 3, "membership_hash": make([]byte, 32), "msg_type": 0, "text": text})
	require.NoError(t, err)
	return append([]byte{'F'}, b...)
}

func chatRecv(t *testing.T, ctx context.Context, conn *ps.Conn) string {
	raw, err := conn.Recv(ctx)
	require.NoError(t, err)
	require.Equal(t, byte('F'), raw[0])
	var m struct {
		Text string `cbor:"text"`
	}
	require.NoError(t, cbor.Unmarshal(raw[1:], &m))
	return m.Text
}

// TestEliza runs the Rust Eliza binary named by ELIZA_BIN.
func TestEliza(t *testing.T) {
	bin := os.Getenv("ELIZA_BIN")
	if bin == "" {
		t.Skip("set ELIZA_BIN to the eliza binary")
	}
	if testing.Short() {
		t.Skip("runs real CTIDH replicas; a few minutes")
	}
	n := world(t)
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Minute)
	t.Cleanup(cancel)

	data := filepath.Join(t.TempDir(), "eliza")
	sock := filepath.Join(n.Dir(), "kpclientd.sock")
	cmd := exec.CommandContext(ctx, bin, "--daemon", sock, "--data-dir", data, "--vouchers", "1", "--poll-interval", "0.2")
	cmd.Env = append(os.Environ(), "RUST_LOG=info")
	logFile, err := os.Create(filepath.Join(n.Dir(), "eliza.log"))
	require.NoError(t, err)
	cmd.Stderr = logFile
	stdout, err := cmd.StdoutPipe()
	require.NoError(t, err)
	require.NoError(t, cmd.Start())
	t.Cleanup(func() {
		_ = cmd.Process.Signal(os.Interrupt)
		_ = cmd.Wait()
	})
	line, err := bufio.NewReader(stdout).ReadString('\n')
	require.NoError(t, err)
	raw, err := hex.DecodeString(strings.TrimSpace(line))
	require.NoError(t, err)
	require.Len(t, raw, 32)
	id := ps.ServiceID(raw)
	t.Logf("eliza is %x", id[:8])

	ph := ps.NewThinPigeonhole(n.ThinClient())

	// As katzenqt does: take a voucher from the bot and induct it.
	var code string
	for code == "" {
		require.NoError(t, ctx.Err(), "eliza never wrote a voucher")
		files, _ := filepath.Glob(filepath.Join(data, "vouchers", "*.txt"))
		if len(files) > 0 {
			b, err := os.ReadFile(files[0])
			require.NoError(t, err)
			code = strings.TrimSpace(string(b))
		}
		time.Sleep(poll)
	}
	voucher, err := base64.StdEncoding.DecodeString(code)
	require.NoError(t, err)
	user, err := ps.Induct(ctx, ph, voucher, "alice", poll)
	require.NoError(t, err)
	require.Equal(t, "How do you do. Please tell me your problem.", chatRecv(t, ctx, user))
	require.NoError(t, user.Send(ctx, chatText(t, "I am sad")))
	require.Equal(t, "I am sorry to hear that you are sad.", chatRecv(t, ctx, user))
	t.Log("chatted through an offered voucher")

	// Through the Go router: the Rust service registered with it and
	// advertised itself in the Go directory, both under MACs the Go
	// plugins check, and opens the voucher the client sealed.
	d := lookupLinked(t, ctx, ph, id)
	conn, err := ps.Dial(ctx, ph, d, "bob", poll)
	require.NoError(t, err)
	require.Equal(t, "How do you do. Please tell me your problem.", chatRecv(t, ctx, conn))
	require.NoError(t, conn.Send(ctx, chatText(t, "you are kind")))
	require.Equal(t, "What makes you think I am kind?", chatRecv(t, ctx, conn))
	t.Log("chatted through the router")
}
