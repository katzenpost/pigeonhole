#!/bin/sh
# Check testdata/ against its recorded sha256 sums, or with --write record them.
# With CRYPTLIB or GCC set, also check the copies against those checkouts.
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
cd "$here/testdata"
if [ "${1:-}" = --write ]; then
  find katzenqt1 -type f | LC_ALL=C sort | xargs sha256sum > SHA256SUMS
  exit 0
fi
sha256sum --quiet -c SHA256SUMS
status=0
check() {
  if ! cmp -s "$1" "$2"; then echo "differs: $1 $2"; status=1; fi
}
if [ -n "${CRYPTLIB:-}" ]; then
  for f in acks_vectors.json groupchat_wire_vectors.json; do check "katzenqt1/$f" "$CRYPTLIB/testdata/$f"; done
  for d in groupchat_traces groupchat_scenarios; do
    for f in "$CRYPTLIB/testdata/$d"/*; do check "katzenqt1/$d/$(basename "$f")" "$f"; done
  done
fi
if [ -n "${GCC:-}" ]; then
  for f in "$GCC"/GroupChatClient/GroupChatKatzenqt1/Tests/scenarios/*.txt; do
    check "katzenqt1/client_scenarios/$(basename "$f")" "$f"
  done
fi
exit $status
