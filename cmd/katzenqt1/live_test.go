package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

// TestLive runs two clients over the client daemon at $DAEMON, on a real mixnet: one founds a
// group, the other joins it with a voucher, and each reads what the other sends, then sees its
// own message acknowledged.
func TestLive(t *testing.T) {
	daemon := os.Getenv("DAEMON")
	if daemon == "" {
		t.Skip("DAEMON not set")
	}
	dir := t.TempDir()
	bin := filepath.Join(dir, "bin")
	os.Mkdir(bin, 0o700)
	build(t, bin)
	long := 30 * time.Minute
	run := func(name string) *proc {
		return startWith(t, bin, "-daemon", daemon, "-state", filepath.Join(dir, name), "-name", name,
			"-log", filepath.Join(dir, name+".log"), "-wait", long.String())
	}
	alice, bob := run("alice"), run("bob")
	defer alice.kill()
	defer bob.kill()
	step := func(what string) { t.Logf("%s %s", time.Now().Format("15:04:05"), what) }
	alice.send("create g")
	alice.waitLong(t, long, func(l string) bool { return l == "JOINED g" })
	step("alice founded g")
	bob.send("join g")
	code := strings.TrimPrefix(bob.waitLong(t, long, func(l string) bool { return strings.HasPrefix(l, "VOUCHER=") }), "VOUCHER=")
	step("bob minted a voucher")
	alice.send("invite g " + code)
	bob.waitLong(t, long, func(l string) bool { return l == "JOINED g" })
	step("bob joined")
	alice.send("send g hello bob")
	bob.send("send g hello alice")
	bob.waitLong(t, long, func(l string) bool { return l == "LINE g alice: hello bob" })
	alice.waitLong(t, long, func(l string) bool { return l == "LINE g bob: hello alice" })
	step("each read the other")
	alice.send("send g read you")
	bob.send("send g read you too")
	alice.waitLong(t, long, func(l string) bool { return l == "LINE g bob: read you too" })
	bob.waitLong(t, long, func(l string) bool { return l == "LINE g alice: read you" })
	for deadline := time.Now().Add(long); ; time.Sleep(10 * time.Second) {
		alice.send("show g")
		bob.send("show g")
		time.Sleep(time.Second)
		if alice.has("me ✓✓: hello bob") && bob.has("me ✓✓: hello alice") {
			break
		}
		if time.Now().After(deadline) {
			t.Fatal("no acknowledgements")
		}
	}
	step("each saw its first message acknowledged")
}
