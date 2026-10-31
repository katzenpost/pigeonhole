# SPDX-FileCopyrightText: © 2026 David Stainton
# SPDX-License-Identifier: AGPL-3.0-only
"""Replay a member trace (testdata/katzenqt1/groupchat_traces) through the
Python member: one JSON line per call made on a member, with its arguments,
its result and the member's whole state afterwards, as katzenqt's
``trace.py`` records them. The replay makes the same call on a ``Group``
and requires the same result and state, as the Go ``tracejson`` package
and cryptlib's Lean replay do. Messages travel decoded in a trace; the
member's plain codec encodes them again here."""

from __future__ import annotations

import json
from typing import Any, Dict, Iterable, List, Optional

from .member import events as ev
from .member import trace
from .member.group import Group
from .member.sentbox import Retention
from .member.wire import CborCodec, Introduction, IntroRef, Message


class Mismatch(AssertionError):
    pass


def unhex(s: Optional[str]) -> Optional[bytes]:
    return None if s is None else bytes.fromhex(s)


def read_message(j: Optional[dict]) -> Optional[Message]:
    if j is None:
        return None
    intro = j.get("introduction")
    at = j.get("introduced_at")
    return Message(
        content=unhex(j.get("content")),
        introduction=Introduction(intro["name"], unhex(intro["read_cap"])) if intro else None,
        numbers=tuple((k, unhex(ref)) for k, ref in j.get("numbers", [])),
        introduced_at=IntroRef(at[0], unhex(at[1])) if at else None,
        acks={k: v for k, v in j.get("acks", [])},
        membership_hash=unhex(j.get("membership_hash")),
    )


def read_join_reply(j: dict) -> ev.JoinReply:
    return ev.JoinReply(
        members=tuple((unhex(m[0]), unhex(m[1])) if m is not None else None for m in j["members"]),
        rosters=tuple(tuple((i, s) for i, s in r) if r is not None else None for r in j["rosters"]),
        introductions=tuple((by, unhex(idx), m) for by, idx, m in j["introductions"]),
        introducer=j["introducer"],
        newcomer=j["newcomer"],
    )


def read_policy(j: dict) -> Retention:
    return Retention(**{k: v for k, v in j.items() if k != "type"})


def read_event(j: dict, codec) -> ev.Event:
    t = j["type"]
    if t == "Send":
        return ev.Send(unhex(j["content"]))
    if t == "Read":
        return ev.Read(unhex(j["stream"]))
    if t == "Scan":
        return ev.Scan(unhex(j["stream"]))
    if t == "BoxRead":
        r = j["result"]
        if isinstance(r, dict):
            m = read_message(r.get("data"))
            result: Any = codec.encode(m) if m is not None else b""
        else:
            result = ev.Missing(r)
        return ev.BoxRead(unhex(j["stream"]), unhex(j["index"]), result, unhex(j.get("next_index")))
    if t == "IndexesDerived":
        return ev.IndexesDerived(unhex(j["stream"]), tuple(unhex(i) for i in j["indexes"]))
    if t == "Walked":
        return ev.Walked(unhex(j["stream"]), unhex(j.get("target")), tuple(unhex(i) for i in j["skipped"]), j.get("steps", 0))
    if t == "Invite":
        return ev.Invite(j["name"], unhex(j["read_cap"]))
    if t == "Introduce":
        return ev.Introduce(j["name"], unhex(j["read_cap"]))
    if t == "Remove":
        return ev.Remove(unhex(j["member"]))
    if t == "EpochChanged":
        return ev.EpochChanged(j["epoch"])
    raise ValueError("unknown event %s" % t)


def read_payload(j: Any, codec) -> Optional[bytes]:
    """What a written box held: a message, decoded in the trace, or bytes
    the client handed over."""
    if j is None:
        return None
    if isinstance(j, dict) and "bytes" in j and len(j) == 1:
        return unhex(j["bytes"])
    return codec.encode(read_message(j))


def strip(x: Any) -> Any:
    """Results and states are compared whole; this is the place to drop a
    field one writer has and another lacks, and there is none now."""
    return x


def replay(lines: Iterable[str]) -> int:
    """Replay one trace. Returns how many calls were checked; raises
    Mismatch at the first call whose result or state differs."""
    groups: Dict[str, Group] = {}
    codec = CborCodec()
    checked = 0
    for n, raw in enumerate(lines, 1):
        raw = raw.strip()
        if not raw:
            continue
        d = json.loads(raw)
        call, args, me = d["call"], d["args"], d["me"]
        policy, recheck = read_policy(args["policy"]), args["recheck"]
        if call == "found":
            g = Group.found(unhex(args["read_cap"]), [unhex(f) for f in args["founders"]], policy=policy, recheck=recheck, codec=codec)
            groups[me] = g
            result: Any = None
        elif call == "joined":
            g = Group.joined(unhex(args["read_cap"]), read_join_reply(args["reply"]), policy=policy, recheck=recheck, codec=codec)
            groups[me] = g
            result = None
        else:
            g = groups[me]
            # The client may change the retention policy between calls, as a
            # user's disappearing messages setting does.
            g.policy, g.recheck = policy, recheck
            if call == "handle":
                result = g.handle(read_event(args["event"], codec))
            elif call == "observe":
                m = read_message(args.get("message"))
                result = g.observe(unhex(args["stream"]), unhex(args["index"]), codec.encode(m) if m is not None else None, unhex(args["next"]))
            elif call == "compose":
                result = g.compose(unhex(args.get("content")))
            elif call == "written":
                result = g.written(args["token"], unhex(args["index"]), read_payload(args.get("payload"), codec))
            elif call == "abandoned":
                result = g.abandoned(args["token"])
            elif call == "begin_induction":
                result = g.begin_induction()
            elif call == "introduce_now":
                result = g.introduce_now(args["name"], unhex(args["read_cap"]))
            elif call == "cancel_induction":
                result = g.cancel_induction()
            elif call == "reply":
                result = g.reply(unhex(args["own_start"]))
            elif call == "remove":
                result = g.remove(unhex(args["member"]))
            else:
                raise ValueError("line %d: unknown call %s" % (n, call))
        got = strip(trace.plain(result, codec))
        want = strip(d["result"])
        if got != want:
            raise Mismatch("line %d (%s %s): result differs:\n got  %s\n want %s" % (n, d["member"], call, json.dumps(got)[:600], json.dumps(want)[:600]))
        got_state, want_state = strip(trace.snapshot(g)), strip(d["state"])
        if got_state != want_state:
            diff = [k for k in sorted(set(got_state) | set(want_state)) if got_state.get(k) != want_state.get(k)]
            raise Mismatch("line %d (%s %s): state differs in %s:\n got  %s\n want %s" % (
                n, d["member"], call, diff, json.dumps({k: got_state.get(k) for k in diff})[:800], json.dumps({k: want_state.get(k) for k in diff})[:800]))
        checked += 1
    return checked


def replay_file(path: str) -> int:
    with open(path) as f:
        return replay(f)
