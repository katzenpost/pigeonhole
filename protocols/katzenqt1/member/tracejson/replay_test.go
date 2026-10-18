package tracejson

import (
	"os"
	"path/filepath"
	"testing"
)

func TestReplayKatzenqtTraces(t *testing.T) {
	files, _ := filepath.Glob("../../../../testdata/katzenqt1/groupchat_traces/*.jsonl")
	if len(files) != 15 {
		t.Fatalf("%d traces", len(files))
	}
	calls, checks := 0, 0
	for _, f := range files {
		r, err := os.Open(f)
		if err != nil {
			t.Fatal(err)
		}
		n, c, err := Replay(r)
		r.Close()
		calls += n
		checks += c
		if err != nil {
			t.Errorf("%s:%v", filepath.Base(f), err)
		}
	}
	t.Logf("%d traces, %d calls, %d BACAP checks", len(files), calls, checks)
}
