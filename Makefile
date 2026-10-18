CRYPTLIB ?= $(HOME)/cryptlib
KATZENQT ?= $(HOME)/katzenqt
GCC ?= $(HOME)/group_chat_client

.PHONY: test vet sync-testdata verify-testdata

export TMPDIR ?= $(HOME)/scratch/tmp

test:
	mkdir -p $(TMPDIR)
	go test ./...

vet:
	go vet ./...

sync-testdata:
	CRYPTLIB=$(CRYPTLIB) GCC=$(GCC) scripts/sync-testdata.sh

verify-testdata:
	scripts/verify-testdata.sh

# Cross-checks against katzenqt and Lean's model; OUT holds what they exchange.
OUT ?= $(HOME)/scratch/pigeonhole-conformance
PYTHON ?= $(KATZENQT)/.venv/bin/python
RUNS ?= 200

.PHONY: conformance replay-katzenqt replay-lean diff-katzenqt

conformance: replay-katzenqt replay-lean diff-katzenqt

# Fresh random runs of katzenqt's stateful tests, replayed through the Go member.
replay-katzenqt:
	rm -rf $(OUT)/katzenqt && mkdir -p $(OUT)/katzenqt
	cd $(KATZENQT) && $(PYTHON) tools/groupchat_traces.py $(OUT)/katzenqt 20
	TRACES=$(OUT)/katzenqt go test -count=1 -run More ./protocols/katzenqt1/member/tracejson/

# Go's scenarios and random runs, replayed through Lean's model.
replay-lean:
	rm -rf $(OUT)/go && mkdir -p $(OUT)/go
	TRACE_OUT=$(OUT)/go go test -count=1 -run WriteTraces ./protocols/katzenqt1/member/sim/
	RANDOM_RUNS=$(RUNS) RANDOM_OUT=$(OUT)/go go test -count=1 -run Random ./protocols/katzenqt1/member/sim/
	cd $(CRYPTLIB) && lake exe Cryptlib.Protocols.GroupComm.GroupChatKatzenqt1.replay $(OUT)/go

# Go's random scripts, run through katzenqt's simulator.
diff-katzenqt:
	rm -rf $(OUT)/scripts && mkdir -p $(OUT)/scripts
	RANDOM_RUNS=$(RUNS) RANDOM_OUT=$(OUT)/scripts go test -count=1 -run Random ./protocols/katzenqt1/member/sim/
	cd $(KATZENQT) && PYTHONPATH=src $(PYTHON) $(CURDIR)/scripts/katzenqt_diff.py $(OUT)/scripts
