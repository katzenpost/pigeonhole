package motd_test

import (
	"context"
	"path/filepath"
	"sync/atomic"
	"testing"
	"time"

	"github.com/stretchr/testify/require"

	"github.com/katzenpost/katzenpost/core/log"
	ps "github.com/katzenpost/katzenpost/pigeonservice"
	"github.com/katzenpost/katzenpost/pigeonservice/directory"
	"github.com/katzenpost/katzenpost/pigeonservice/phtest"
	"github.com/katzenpost/katzenpost/pigeonservice/router"

	client "github.com/katzenpost/pigeonhole_protocols/clients/motd"
	"github.com/katzenpost/pigeonhole_protocols/services/motd"
)

const poll = 10 * time.Millisecond

func pluginKey(t *testing.T) *ps.PluginKey {
	pub, priv, err := ps.DefaultNIKE.GenerateKeyPair()
	require.NoError(t, err)
	return &ps.PluginKey{Scheme: ps.DefaultNIKE, Public: pub, Private: priv}
}

func TestMOTD(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), time.Minute)
	defer cancel()
	logBackend, err := log.New("", "info", false)
	require.NoError(t, err)
	net := phtest.NewNetwork()

	rkey := pluginKey(t)
	r, err := router.New(router.Config{
		Key: rkey, DBPath: filepath.Join(t.TempDir(), "router.db"),
		PollInterval: poll, Retention: time.Hour, Log: logBackend.GetLogger("router"),
	}, net)
	require.NoError(t, err)
	defer r.Close()
	net.AddService(ps.RouterCapability, [32]byte{0xa0}, ps.RouterCapability, ps.PluginParams(rkey),
		func(_ context.Context, p []byte) []byte { return r.Handle(p) })
	dkey := pluginKey(t)
	d, err := directory.New(directory.Config{
		Key: dkey, DBPath: filepath.Join(t.TempDir(), "directory.db"),
		RefreshInterval: time.Hour, Log: logBackend.GetLogger("directory"),
	}, net)
	require.NoError(t, err)
	defer d.Close()
	net.AddService(ps.DirectoryCapability, [32]byte{0xdd}, ps.DirectoryCapability, ps.PluginParams(dkey),
		func(_ context.Context, p []byte) []byte { return d.Handle(p) })

	dataDir := t.TempDir()
	key, err := ps.LoadServiceKey(dataDir)
	require.NoError(t, err)
	adv, err := directory.NewClient(net, key, poll)
	require.NoError(t, err)
	svc, err := ps.NewService(ps.Config{
		Key: key, DataDir: dataDir, DisplayName: "motd", Advertiser: adv,
		PollInterval: poll, Log: logBackend.GetLogger("motd"),
	}, net)
	require.NoError(t, err)
	var message atomic.Value
	message.Store("Be excellent to each other.")
	go svc.Run(ctx, ps.HandlerFunc(func(ctx context.Context, s *ps.Session) {
		_ = motd.Serve(ctx, s, func() (string, error) { return message.Load().(string), nil })
	}))
	t.Cleanup(func() { cancel(); svc.Close() })

	reader, err := directory.NewClient(net, nil, poll)
	require.NoError(t, err)
	var desc *ps.ServiceDescriptor
	require.Eventually(t, func() bool {
		desc, err = reader.Lookup(ctx, svc.ID())
		return err == nil && len(desc.Routers) == 1
	}, 30*time.Second, 20*time.Millisecond)

	conn, err := ps.Dial(ctx, net, desc, "client", poll)
	require.NoError(t, err)
	reply, err := client.Fetch(ctx, conn)
	require.NoError(t, err)
	require.Equal(t, "Be excellent to each other.", reply.Message)
	require.Equal(t, time.Now().UTC().Format(time.DateOnly), reply.Day)

	message.Store("Tomorrow is another day.")
	reply, err = client.Fetch(ctx, conn)
	require.NoError(t, err)
	require.Equal(t, "Tomorrow is another day.", reply.Message, "each request reads the current message")
}
