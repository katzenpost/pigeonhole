# SPDX-FileCopyrightText: © 2026 David Stainton
# SPDX-License-Identifier: AGPL-3.0-only
"""Protocols over Katzenpost's Pigeonhole storage, in Python: the same
packages as the Go and Rust implementations beside this one. ``katzenqt1``
is the group chat: a sans-IO member, an engine that is a whole client
without IO, and a client that carries the engine's work out over a
``transport``."""
