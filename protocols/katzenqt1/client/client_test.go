package client

import (
	"context"
	"errors"
	"slices"
	"testing"
	"time"

	"github.com/katzenpost/pigeonhole/protocols/katzenqt1/engine"
	"github.com/katzenpost/pigeonhole/transport/sim"
)

func open(t *testing.T, w *sim.World, dir, name string) *Client {
	t.Helper()
	c, err := Open(context.Background(), Config{StateDir: dir, Name: name, Backend: sim.Backend{World: w}, EpochPoll: time.Second})
	if err != nil {
		t.Fatal(err)
	}
	return c
}

// waitFor polls until ok holds, for up to a minute.
func waitFor(t *testing.T, what string, ok func() bool) {
	t.Helper()
	for deadline := time.Now().Add(time.Minute); !ok(); time.Sleep(20 * time.Millisecond) {
		if time.Now().After(deadline) {
			t.Fatalf("waited a minute for %s", what)
		}
	}
}

func has(t *testing.T, c *Client, id engine.ChatID, text string) bool {
	ls, err := c.Lines(id, -1)
	if err != nil {
		t.Fatal(err)
	}
	return slices.ContainsFunc(ls, func(l LineView) bool { return l.Notice == nil && l.Text == text })
}

func TestClient(t *testing.T) {
	w := sim.NewWorld()
	aliceDir, bobDir := t.TempDir(), t.TempDir()
	alice, bob := open(t, w, aliceDir, "alice"), open(t, w, bobDir, "bob")
	events := alice.Subscribe()
	g, err := alice.Create("g")
	if err != nil {
		t.Fatal(err)
	}
	if _, err := alice.Create("g"); !errors.Is(err, ErrNameTaken) {
		t.Fatalf("a name taken: %v", err)
	}
	if err := alice.Send("nope", "x"); !errors.Is(err, ErrNoSuchChat) {
		t.Fatalf("no such group: %v", err)
	}
	bg, err := bob.Join("g")
	if err != nil {
		t.Fatal(err)
	}
	var code string
	waitFor(t, "a voucher code", func() bool {
		v, _ := bob.Chat(bg)
		code = v.Code
		return code != ""
	})
	waitFor(t, "alice's group", func() bool { v, _ := alice.Chat(g); return v.Joined })
	if err := alice.Invite(g, "abc!"); !errors.Is(err, ErrNotAVoucher) {
		t.Fatalf("junk taken for a voucher: %v", err)
	}
	if err := alice.Invite(g, code); err != nil {
		t.Fatal(err)
	}
	waitFor(t, "bob to join", func() bool { v, _ := bob.Chat(bg); return v.Joined })
	if err := bob.Send(bg, "hi alice"); err != nil {
		t.Fatal(err)
	}
	if err := alice.Send(g, "hi bob"); err != nil {
		t.Fatal(err)
	}
	waitFor(t, "each to read the other", func() bool { return has(t, alice, g, "hi alice") && has(t, bob, bg, "hi bob") })
	select {
	case e := <-events:
		if e.Chat != g {
			t.Fatalf("event for %s", e.Chat)
		}
	default:
		t.Fatal("no events")
	}
	bobKey, err := alice.MemberNamed(g, "bob")
	if err != nil {
		t.Fatal(err)
	}
	if bobKey == "" {
		t.Fatal("bob not found by name")
	}
	v, _ := alice.Chat(g)
	if err := alice.Remove(g, v.Me); !errors.Is(err, ErrRemoveSelf) {
		t.Fatalf("remove ourselves: %v", err)
	}
	if err := alice.Remove(g, "nobody"); !errors.Is(err, ErrNoSuchMember) {
		t.Fatalf("remove nobody: %v", err)
	}
	// Alice closes, and opens again where she was.
	if err := alice.Close(); err != nil {
		t.Fatal(err)
	}
	for range events {
	}
	if err := alice.Send(g, "closed"); !errors.Is(err, ErrClosed) {
		t.Fatalf("send after close: %v", err)
	}
	alice = open(t, w, aliceDir, "alice")
	defer alice.Close()
	defer bob.Close()
	if !has(t, alice, g, "hi alice") {
		t.Fatal("alice forgets what she read")
	}
	if err := alice.Send(g, "back again"); err != nil {
		t.Fatal(err)
	}
	if err := bob.Send(bg, "read you"); err != nil {
		t.Fatal(err)
	}
	waitFor(t, "the conversation to carry on", func() bool { return has(t, bob, bg, "back again") && has(t, alice, g, "read you") })
	waitFor(t, "bob to acknowledge", func() bool {
		ls, _ := alice.Lines(g, -1)
		return slices.ContainsFunc(ls, func(l LineView) bool { return l.Text == "hi bob" && l.Mine && l.Acked })
	})
	if len(w.Refused) > 0 {
		t.Fatalf("refused: %v", w.Refused)
	}
}
