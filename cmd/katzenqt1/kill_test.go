package main

import (
	"bufio"
	"fmt"
	"io"
	"math/rand/v2"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"syscall"
	"testing"
	"time"
)

// TestKill9 kills a client with SIGKILL again and again while it sends, over a Pigeonhole in a
// process of its own, with its state synced to disk as in use: every message it said it took
// must reach the other member, once.

func build(t *testing.T, dir string) {
	cmd := exec.Command("go", "build", "-o", dir, "github.com/katzenpost/pigeonhole/cmd/...")
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("build: %v\n%s", err, out)
	}
}

type proc struct {
	cmd   *exec.Cmd
	in    io.WriteCloser
	mu    sync.Mutex
	lines []string
	done  chan struct{}
}

func start(t *testing.T, bin, sock, state, name string) *proc {
	t.Helper()
	cmd := exec.Command(filepath.Join(bin, "katzenqt1"), "-sim", sock, "-state", state, "-name", name, "-wait", "60s")
	in, _ := cmd.StdinPipe()
	outp, _ := cmd.StdoutPipe()
	cmd.Stderr = os.Stderr
	if err := cmd.Start(); err != nil {
		t.Fatal(err)
	}
	p := &proc{cmd: cmd, in: in, done: make(chan struct{})}
	go func() {
		sc := bufio.NewScanner(outp)
		for sc.Scan() {
			p.mu.Lock()
			p.lines = append(p.lines, sc.Text())
			p.mu.Unlock()
		}
		close(p.done)
	}()
	p.waitFor(t, func(l string) bool { return l == "READY" })
	return p
}

func (p *proc) send(line string) { fmt.Fprintln(p.in, line) }

// waitFor waits for a line that ok says yes to, returning it.
func (p *proc) waitFor(t *testing.T, ok func(string) bool) string {
	t.Helper()
	for deadline := time.Now().Add(time.Minute); time.Now().Before(deadline); time.Sleep(10 * time.Millisecond) {
		p.mu.Lock()
		for _, l := range p.lines {
			if ok(l) {
				p.mu.Unlock()
				return l
			}
		}
		p.mu.Unlock()
		select {
		case <-p.done:
			t.Fatal("the client stopped")
		default:
		}
	}
	t.Fatal("waited a minute")
	return ""
}

func (p *proc) has(l string) bool {
	p.mu.Lock()
	defer p.mu.Unlock()
	for _, x := range p.lines {
		if x == l {
			return true
		}
	}
	return false
}

func (p *proc) kill() {
	p.cmd.Process.Signal(syscall.SIGKILL)
	p.cmd.Wait()
}

func TestKill9(t *testing.T) {
	if testing.Short() {
		t.Skip("builds and runs processes")
	}
	dir := t.TempDir()
	bin := filepath.Join(dir, "bin")
	os.Mkdir(bin, 0o700)
	build(t, bin)
	sock := filepath.Join(dir, "sim.sock")
	sim := exec.Command(filepath.Join(bin, "pigeonhole-sim"), "-socket", sock)
	if err := sim.Start(); err != nil {
		t.Fatal(err)
	}
	defer func() { sim.Process.Kill(); sim.Wait() }()
	for i := 0; ; i++ {
		if _, err := os.Stat(sock); err == nil {
			break
		}
		if i > 500 {
			t.Fatal("no sim socket")
		}
		time.Sleep(10 * time.Millisecond)
	}
	aliceState, bobState := filepath.Join(dir, "alice"), filepath.Join(dir, "bob")
	alice := start(t, bin, sock, aliceState, "alice")
	bob := start(t, bin, sock, bobState, "bob")
	defer bob.kill()
	alice.send("create g")
	alice.waitFor(t, func(l string) bool { return l == "JOINED g" })
	bob.send("join g")
	code := strings.TrimPrefix(bob.waitFor(t, func(l string) bool { return strings.HasPrefix(l, "VOUCHER=") }), "VOUCHER=")
	alice.send("invite g " + code)
	bob.waitFor(t, func(l string) bool { return l == "JOINED g" })

	rng := rand.New(rand.NewPCG(1, 2))
	var taken []string
	kills := 0
	for round := range 25 {
		for i := range 4 {
			alice.send(fmt.Sprintf("send g m%d.%d", round, i))
		}
		time.Sleep(time.Duration(rng.IntN(400)) * time.Millisecond)
		alice.kill()
		kills++
		for i := range 4 {
			if alice.has(fmt.Sprintf("OK send g m%d.%d", round, i)) {
				taken = append(taken, fmt.Sprintf("m%d.%d", round, i))
			}
		}
		alice = start(t, bin, sock, aliceState, "alice")
	}
	defer alice.kill()
	for _, m := range taken {
		bob.waitFor(t, func(l string) bool { return l == "LINE g alice: "+m })
	}
	bob.send("show g")
	bob.send("chats")
	bob.waitFor(t, func(l string) bool { return strings.HasPrefix(l, "CHAT g") })
	counts := map[string]int{}
	bob.mu.Lock()
	for _, l := range bob.lines {
		if strings.HasPrefix(l, "alice: ") {
			counts[strings.TrimPrefix(l, "alice: ")]++
		}
	}
	bob.mu.Unlock()
	for _, m := range taken {
		if counts[m] != 1 {
			t.Errorf("%s reached bob %d times", m, counts[m])
		}
	}
	// What bob sends still reaches alice, who never wrote over her own boxes.
	bob.send("send g bob here")
	alice.waitFor(t, func(l string) bool { return l == "LINE g bob: bob here" })
	alice.send("show g")
	alice.send("chats")
	alice.waitFor(t, func(l string) bool { return strings.HasPrefix(l, "CHAT g") })
	if alice.has("— a box of our stream holds something else: is this group used from another client?") {
		t.Error("alice wrote other bytes over a box of hers")
	}
	t.Logf("killed alice %d times; %d messages taken, each read once", kills, len(taken))
}
