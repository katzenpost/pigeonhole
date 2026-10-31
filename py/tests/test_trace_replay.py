# SPDX-FileCopyrightText: © 2026 David Stainton
# SPDX-License-Identifier: AGPL-3.0-only
"""The recorded member traces, replayed through the Python member: every
call gives the same result and leaves the same state."""

import glob
import os

import pytest

from pigeonhole_protocols.katzenqt1 import tracejson

HERE = os.path.dirname(os.path.abspath(__file__))
TRACES = sorted(glob.glob(os.path.join(HERE, "..", "..", "testdata", "katzenqt1", "groupchat_traces", "*.jsonl")))


def test_traces_present():
    assert len(TRACES) >= 14


@pytest.mark.parametrize("path", TRACES, ids=[os.path.basename(p) for p in TRACES])
def test_replay(path):
    assert tracejson.replay_file(path) > 0
