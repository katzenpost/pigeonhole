package client

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"slices"
	"strconv"
	"strings"
	"testing"

	"github.com/katzenpost/pigeonhole/protocols/katzenqt1/engine"
	"github.com/katzenpost/pigeonhole/protocols/katzenqt1/member"
	"github.com/katzenpost/pigeonhole/protocols/katzenqt1/member/tracejson"
	"github.com/katzenpost/pigeonhole/protocols/katzenqt1/store"
	"github.com/katzenpost/pigeonhole/protocols/katzenqt1/wire"
	"github.com/katzenpost/pigeonhole/transport/sim"
)

// Members, each a client engine, over the in-memory Pigeonhole, driven by a scenario written as
// group_chat_client's client scenarios are. The network runs on a logical clock, and only when
// told to: run carries out what the members asked for, in order, and lets time pass.

type memFiles map[string][]byte

func (f memFiles) Read(name string) ([]byte, error) {
	b, ok := f[name]
	if !ok {
		return nil, errors.New("no such file")
	}
	return b, nil
}

func (f memFiles) Remove(name string) error {
	delete(f, name)
	return nil
}

type queued struct {
	who int
	a   engine.Action
}

type later struct {
	at  int
	who int
	a   engine.Action
	in  engine.Input
}

type scMember struct {
	name  string
	eng   *engine.Engine
	files memFiles
	// st, when set, keeps the member's state on disk, saved after every step as a client does.
	st *store.Store
}

type scWorld struct {
	sim     *sim.World
	members []*scMember
	queue   []queued
	later   []later
	clock   int
	held    map[int]bool
	epoch   uint64
	unsaved []string
	// crash, if set, is told of each action done; when it says so, the member restarts from what
	// is on disk, before (lost) or after (kept) what came of the action is taken in.
	crash func(who int, a engine.Action) (restart, lost bool)
	// restarts is how many times each member restarted.
	restarts map[int]int
}

func (w *scWorld) who(name string) int {
	for i, m := range w.members {
		if m.name == name {
			return i
		}
	}
	w.members = append(w.members, &scMember{name: name, eng: engine.New(&engine.State{Me: name}, 1530), files: memFiles{}})
	return len(w.members) - 1
}

func (w *scWorld) feed(i int, in engine.Input) {
	m := w.members[i]
	w.release(i, m.eng.Step(0, in))
	for _, c := range m.eng.State.Chats {
		if c.Group != nil && !savesWhole(c.Group) {
			w.unsaved = append(w.unsaved, fmt.Sprintf("%s %s after %T", m.name, c.Name, in))
		}
	}
}

// release saves what a step changed, then queues the work it asks for.
func (w *scWorld) release(i int, res engine.Result) {
	m := w.members[i]
	if m.st != nil {
		if err := m.st.Commit(m.eng.State, &res); err != nil {
			panic(err)
		}
	} else {
		for _, f := range res.Files {
			m.files[f.File] = f.Bytes
		}
	}
	for _, a := range res.Actions {
		w.queue = append(w.queue, queued{i, a})
	}
}

func (m *scMember) filesOf() Files {
	if m.st != nil {
		return store.Files{S: m.st}
	}
	return m.files
}

// savesWhole says whether a group reads back as it was written.
func savesWhole(g *member.Group) bool {
	b, _ := json.Marshal(tracejson.Group(g))
	j, _ := tracejson.Parse(append(append([]byte(`{"g":`), b...), '}'))
	back, err := tracejson.ReadGroup(j["g"])
	if err != nil {
		return false
	}
	b2, _ := json.Marshal(tracejson.Group(back))
	return string(b) == string(b2)
}

func (w *scWorld) act() bool {
	k := slices.IndexFunc(w.queue, func(q queued) bool { return !w.held[q.who] })
	if k < 0 {
		return false
	}
	q := w.queue[k]
	w.queue = slices.Delete(w.queue, k, k+1)
	x := &Executor{Backend: sim.Backend{World: w.sim}, Files: w.members[q.who].filesOf(),
		Sleep: func(context.Context, int) error { return nil }}
	o := x.Perform(context.Background(), q.a)
	if w.crash != nil {
		if restart, lost := w.crash(q.who, q.a); restart {
			if !lost && o.Input != nil && o.After == 0 && o.Retry == nil {
				w.feed(q.who, o.Input)
			}
			w.restartMember(q.who)
			return true
		}
	}
	switch {
	case o.Retry != nil:
		w.later = append(w.later, later{at: w.clock + o.After, who: q.who, a: o.Retry})
	case o.Input != nil && o.After > 0:
		w.later = append(w.later, later{at: w.clock + o.After, who: q.who, in: o.Input})
	case o.Input != nil:
		w.feed(q.who, o.Input)
	}
	return true
}

// runTo lets the network run until stop on the logical clock.
func (w *scWorld) runTo(stop int) {
	for {
		if w.act() {
			continue
		}
		due := -1
		for _, l := range w.later {
			if l.at <= stop && !w.held[l.who] && (due < 0 || l.at < due) {
				due = l.at
			}
		}
		if due < 0 {
			w.clock = max(w.clock, stop)
			return
		}
		var now, rest []later
		for _, l := range w.later {
			if l.at <= due && !w.held[l.who] {
				now = append(now, l)
			} else {
				rest = append(rest, l)
			}
		}
		w.later, w.clock = rest, max(w.clock, due)
		for _, l := range now {
			if l.a != nil {
				w.queue = append(w.queue, queued{l.who, l.a})
			} else {
				w.feed(l.who, l.in)
			}
		}
	}
}

func (w *scWorld) chatOf(i int, group string) *engine.Chat {
	return w.members[i].eng.State.Chat(engine.ChatID(group))
}

// nameOf is the member whose key k is: the truth, for printing.
func (w *scWorld) nameOf(k member.Key) string {
	for _, m := range w.members {
		for _, c := range m.eng.State.Chats {
			if c.Group != nil && c.Group.Me() == k {
				return m.name
			}
		}
	}
	return "?"
}

func firstOf(c *engine.Chat) uint64 {
	return wire.Counter(c.WriteCap[len(c.WriteCap)-wire.IndexSize:])
}

func (w *scWorld) show(i int, group string) []string {
	c := w.chatOf(i, group)
	if c == nil {
		return []string{"no such group"}
	}
	var out []string
	for _, l := range c.Log {
		if l.Notice != nil {
			out = append(out, "— "+engine.Plain(l.Notice))
			continue
		}
		mark := ""
		if c.Group != nil && c.Group.Me() == l.Sender {
			switch {
			case l.Status == engine.Pending:
				mark = " …"
			case c.Acknowledged(l):
				mark = " ✓✓"
			default:
				mark = " ✓"
			}
		}
		out = append(out, w.nameOf(l.Sender)+mark+": "+strings.ReplaceAll(l.Text, "\n", " | "))
	}
	return out
}

func (w *scWorld) membersOf(i int, group string) []string {
	c := w.chatOf(i, group)
	switch {
	case c == nil:
		return []string{"no such group"}
	case c.Group == nil:
		return []string{"not joined"}
	}
	var names []string
	for _, k := range c.Group.Readers.Keys() {
		names = append(names, w.nameOf(k))
	}
	slices.Sort(names)
	s := strings.TrimSpace(strings.Join(names, " "))
	if s == "" {
		s = "nobody"
	}
	return []string{s}
}

func (w *scWorld) acked(i int, group string) []string {
	c := w.chatOf(i, group)
	switch {
	case c == nil:
		return []string{"no such group"}
	case c.Group == nil:
		return []string{"not joined"}
	}
	first := firstOf(c)
	var acks []string
	for k, n := range c.Group.Ledger.AckedBy.All {
		acks = append(acks, fmt.Sprintf("%s:%d", w.nameOf(k), n-first))
	}
	if len(acks) == 0 {
		return []string{"none"}
	}
	slices.Sort(acks)
	return []string{strings.Join(acks, " ")}
}

func (w *scWorld) state(i int, group string) []string {
	c := w.chatOf(i, group)
	if c == nil {
		return []string{"no such group"}
	}
	parts := []string{"not joined"}
	if c.Group != nil {
		parts[0] = "joined"
	}
	if c.Joining != nil {
		if c.Joining.Code == "" {
			parts = append(parts, "minting")
		} else {
			parts = append(parts, "awaiting a reply")
		}
	}
	if c.Inducting != nil {
		parts = append(parts, "adding a member")
	}
	if len(c.Jobs) > 0 {
		parts = append(parts, fmt.Sprintf("%d to write", len(c.Jobs)))
	}
	return []string{strings.Join(parts, ", ")}
}

func (w *scWorld) jobs(i int, group string) []string {
	c := w.chatOf(i, group)
	if c == nil {
		return []string{"no such group"}
	}
	if len(c.Jobs) == 0 {
		return []string{"none"}
	}
	first := firstOf(c)
	var out []string
	for _, j := range c.Jobs {
		at := "next box"
		if j.Index != nil {
			at = fmt.Sprintf("box %d", wire.Counter([]byte(*j.Index))-first)
		}
		what := "rewrite"
		switch {
		case j.Token != nil:
			what = "message"
		case len(j.Box) == 0:
			what = "tombstone"
		}
		out = append(out, fmt.Sprintf("%s to %s, %d bytes", what, at, len(j.Box)))
	}
	return out
}

func (w *scWorld) refused() []string {
	var names []string
	for _, r := range w.sim.Refused {
		name, n := "?", r.Idx64
		for _, m := range w.members {
			for _, c := range m.eng.State.Chats {
				if len(c.WriteCap) > 0 && string(c.WriteCap[32:64]) == r.Stream {
					name, n = m.name, r.Idx64-firstOf(c)
				}
			}
		}
		if s := fmt.Sprintf("%s@%d", name, n); !slices.Contains(names, s) {
			names = append(names, s)
		}
	}
	if len(names) == 0 {
		return []string{"none"}
	}
	return []string{strings.Join(names, " ")}
}

func (w *scWorld) code(i int, group string) string {
	if c := w.chatOf(i, group); c != nil && c.Joining != nil {
		return c.Joining.Code
	}
	return ""
}

func (w *scWorld) command(words []string) ([]string, error) {
	n := len(words)
	switch {
	case words[0] == "create" && n == 3:
		w.feed(w.who(words[1]), engine.Create{Chat: engine.ChatID(words[2]), Name: words[2]})
	case words[0] == "join" && n == 3:
		w.feed(w.who(words[1]), engine.Join{Chat: engine.ChatID(words[2]), Name: words[2]})
	case words[0] == "invite" && n >= 4:
		theirs := words[2]
		if n > 4 {
			theirs = words[4]
		}
		code := w.code(w.who(words[3]), theirs)
		if code == "" {
			return []string{words[3] + " has no voucher code"}, nil
		}
		w.feed(w.who(words[1]), engine.Induct{Chat: engine.ChatID(words[2]), Code: code})
	case words[0] == "send" && n >= 3:
		w.feed(w.who(words[1]), engine.Send{Chat: engine.ChatID(words[2]), Text: strings.Join(words[3:], " ")})
	case words[0] == "remove" && n == 4:
		i := w.who(words[1])
		c := w.chatOf(w.who(words[3]), words[2])
		if c == nil || c.Group == nil {
			return []string{words[3] + " is not in " + words[2]}, nil
		}
		w.feed(i, engine.RemoveMember{Chat: engine.ChatID(words[2]), Who: c.Group.Me()})
	case words[0] == "scan" && n == 3:
		w.feed(w.who(words[1]), engine.Scan{Chat: engine.ChatID(words[2])})
	case words[0] == "leave" && n == 3:
		w.feed(w.who(words[1]), engine.Leave{Chat: engine.ChatID(words[2])})
	case words[0] == "run" && n <= 2:
		ms := 600000
		if n == 2 {
			s, _ := strconv.Atoi(words[1])
			ms = s * 1000
		}
		w.runTo(w.clock + ms)
	case words[0] == "hold" && n == 2:
		w.held[w.who(words[1])] = true
	case words[0] == "release" && n == 2:
		delete(w.held, w.who(words[1]))
	case words[0] == "epoch" && n == 1:
		w.epoch++
		w.sim.SetEpoch(w.epoch)
		for i := range w.members {
			w.feed(i, engine.Epoch{N: int(w.epoch)})
		}
	case words[0] == "fail-writes" && n == 2:
		k, _ := strconv.Atoi(words[1])
		w.sim.FailWrites(k)
	case words[0] == "show" && n == 3:
		return w.show(w.who(words[1]), words[2]), nil
	case words[0] == "members" && n == 3:
		return w.membersOf(w.who(words[1]), words[2]), nil
	case words[0] == "acked" && n == 3:
		return w.acked(w.who(words[1]), words[2]), nil
	case words[0] == "state" && n == 3:
		return w.state(w.who(words[1]), words[2]), nil
	case words[0] == "jobs" && n == 3:
		return w.jobs(w.who(words[1]), words[2]), nil
	case words[0] == "refused" && n == 1:
		return w.refused(), nil
	default:
		return nil, fmt.Errorf("not a command: %s", strings.Join(words, " "))
	}
	return nil, nil
}

// runScenario runs a scenario: the script with every checked command's output replaced by what
// it printed now.
func runScenario(script string) (string, []string, error) {
	lines := strings.Split(script, "\n")
	if len(lines) > 0 && lines[len(lines)-1] == "" {
		lines = lines[:len(lines)-1]
	}
	w := &scWorld{sim: sim.NewWorld(), held: map[int]bool{}, epoch: 1}
	var out []string
	for i := 0; i < len(lines); {
		line := lines[i]
		out = append(out, line)
		i++
		cmd := strings.TrimSpace(strings.SplitN(line, "#", 2)[0])
		if cmd == "" {
			continue
		}
		printed, err := w.command(strings.Fields(cmd))
		if err != nil {
			return "", nil, err
		}
		if i < len(lines) && lines[i] == "----" {
			out = append(out, "----")
			i++
			for i < len(lines) && strings.TrimSpace(lines[i]) != "" {
				i++
			}
			out = append(out, printed...)
		}
	}
	return strings.Join(out, "\n") + "\n", w.unsaved, nil
}

func TestClientScenarios(t *testing.T) {
	files, _ := filepath.Glob("../../../testdata/katzenqt1/client_scenarios/*.txt")
	if len(files) != 7 {
		t.Fatalf("%d scenarios", len(files))
	}
	for _, f := range files {
		name := filepath.Base(f)
		script, _ := os.ReadFile(f)
		got, unsaved, err := runScenario(string(script))
		if err != nil {
			t.Errorf("%s: %v", name, err)
			continue
		}
		if len(unsaved) > 0 {
			t.Errorf("%s: groups that do not save whole: %v", name, unsaved)
		}
		if got != string(script) {
			a, e := strings.Split(got, "\n"), strings.Split(string(script), "\n")
			for i := range min(len(a), len(e)) {
				if a[i] != e[i] {
					t.Errorf("%s: line %d: expected %q, got %q", name, i+1, e[i], a[i])
					break
				}
			}
		}
	}
}
