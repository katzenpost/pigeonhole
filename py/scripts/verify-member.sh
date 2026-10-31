#!/bin/sh
# Check the copied member against a katzenqt checkout at the commit member/SOURCES names.
#
#   py/scripts/verify-member.sh /path/to/katzenqt
set -eu
katzenqt=${1:?path to a katzenqt checkout}
here=$(cd "$(dirname "$0")/.." && pwd)
member="$here/pigeonhole_protocols/katzenqt1/member"
commit=$(awk '{print $2}' "$member/SOURCES")
status=0
for f in "$member"/*.py; do
  name=$(basename "$f")
  case $name in
    ack_codec.py) src=src/katzenqt/ack_codec.py ;;
    *) src=src/katzenqt/groupchat/$name ;;
  esac
  # The one edit: ack_codec is imported from beside the member, not from katzenqt's root.
  if ! git -C "$katzenqt" show "$commit:$src" | sed 's/^from \.\. import ack_codec$/from . import ack_codec/' | cmp -s - "$f"; then
    echo "differs: $src at $commit vs $f"
    status=1
  fi
done
[ $status -eq 0 ] && echo "member matches katzenqt $commit"
exit $status
