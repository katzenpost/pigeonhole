# SPDX-FileCopyrightText: © 2026 David Stainton
# SPDX-License-Identifier: AGPL-3.0-only
"""Writes boxes as katzenqt frames them, for the katzenchat codec's tests.

Run with katzenqt's environment: katzenqt/.venv/bin/python katzenchat_vectors.py out.json
"""
import json
import sys
import uuid

from katzenqt import models, persistent


def msg(**kw):
    return models.GroupChatMessage(version=models.GROUP_CHAT_VERSION, membership_hash=bytes(32), **kw)


def payloads(messages, chunk_size):
    op = models.SendOperation(messages=messages, bacap_stream=uuid.uuid4())
    _, rows = op.serialize(chunk_size=chunk_size, conversation_id=1)
    return [r.bacap_payload for r in rows if isinstance(r, persistent.PlaintextWAL) and r.bacap_payload]


intro = models.GroupChatPleaseAdd(display_name="carol", read_cap=bytes(range(136)))
long_text = "a long message " * 40
out = {
    "text": payloads([msg(text="hello eliza")], 1530)[0].hex(),
    "two_in_one_box": payloads([msg(text="first"), msg(text="second")], 1530)[0].hex(),
    "introduction": payloads([msg(msg_type=models.GroupChatTypeEnum.INTRODUCTION, introduction=intro)], 1530)[0].hex(),
    "control": payloads([msg(msg_type=models.GroupChatTypeEnum.CONTROL, numbers=b"\x01\x02")], 1530)[0].hex(),
    "acked_text": payloads([msg(text="with acks", acks=bytes([9]) + bytes(104))], 1530)[0].hex(),
    "newer_version": (b"F" + models.cbor2.dumps({"version": models.GROUP_CHAT_VERSION + 1, "membership_hash": bytes(32), "msg_type": 0, "text": "from the future"})).hex(),
    "long_text": long_text,
    "long_substream": [p.hex() for p in payloads([msg(text=long_text)], 200)],
}
with open(sys.argv[1], "w") as f:
    json.dump(out, f, indent=2)
    f.write("\n")
