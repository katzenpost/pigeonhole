# pigeonhole

Group communication protocols over Katzenpost's Pigeonhole storage, in Go and Rust.

Each protocol under `protocols/` has a sans-IO state machine and a client that drives it over the
thin client.

- `protocols/katzenqt1`: katzenqt's group chat as of October 2026.

`testdata/` holds vectors shared with cryptlib, katzenqt and group_chat_client; `SOURCES` names
the commits they were copied from.

```
make test
make verify-testdata
```
