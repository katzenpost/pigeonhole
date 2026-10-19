#!/bin/sh
# Copy the vectors shared with cryptlib, katzenqt and group_chat_client into testdata/.
set -eu
CRYPTLIB=${CRYPTLIB:-$HOME/cryptlib}
GCC=${GCC:-$HOME/group_chat_client}
here=$(cd "$(dirname "$0")/.." && pwd)
out=$here/testdata
rm -rf "$out/katzenqt1"
mkdir -p "$out/katzenqt1/client_scenarios"
cp "$CRYPTLIB/testdata/acks_vectors.json" "$CRYPTLIB/testdata/groupchat_wire_vectors.json" "$out/katzenqt1/"
cp -r "$CRYPTLIB/testdata/groupchat_traces" "$CRYPTLIB/testdata/groupchat_scenarios" "$out/katzenqt1/"
cp "$GCC"/GroupChatClient/GroupChatKatzenqt1/Tests/scenarios/*.txt "$out/katzenqt1/client_scenarios/"
{
  echo "cryptlib $(git -C "$CRYPTLIB" rev-parse HEAD)"
  echo "group_chat_client $(git -C "$GCC" rev-parse HEAD)"
} > "$out/SOURCES"
"$here/scripts/verify-testdata.sh" --write
