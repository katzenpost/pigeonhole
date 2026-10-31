# SPDX-FileCopyrightText: © 2026 David Stainton
# SPDX-License-Identifier: AGPL-3.0-only
"""katzenqt's group chat scenarios, as golden files, through the member and
its simulator: the same files the Go and Rust members and the Lean model
run. A scenario passes when the simulator prints exactly what the file
says."""

import glob
import os

import pytest

from pigeonhole_protocols.katzenqt1.member import sim

HERE = os.path.dirname(os.path.abspath(__file__))
SCENARIOS = sorted(glob.glob(os.path.join(HERE, "..", "..", "testdata", "katzenqt1", "groupchat_scenarios", "*.txt")))


def test_all_scenarios_present():
    assert len(SCENARIOS) == 14


@pytest.mark.parametrize("path", SCENARIOS, ids=[os.path.basename(p) for p in SCENARIOS])
def test_scenario(path):
    with open(path) as f:
        script = f.read()
    actual = sim.run(script)
    if actual != script:
        a, e = actual.splitlines(), script.splitlines()
        for i, (x, y) in enumerate(zip(a, e)):
            assert x == y, "%s line %d" % (os.path.basename(path), i + 1)
        assert len(a) == len(e)
