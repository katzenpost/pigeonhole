CRYPTLIB ?= $(HOME)/cryptlib
KATZENQT ?= $(HOME)/katzenqt
GCC ?= $(HOME)/group_chat_client

.PHONY: test vet sync-testdata verify-testdata

test:
	go test ./...

vet:
	go vet ./...

sync-testdata:
	CRYPTLIB=$(CRYPTLIB) GCC=$(GCC) scripts/sync-testdata.sh

verify-testdata:
	scripts/verify-testdata.sh
