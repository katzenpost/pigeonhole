# SPDX-FileCopyrightText: © 2026 David Stainton
# SPDX-License-Identifier: AGPL-3.0-only
"""katzenqt's group chat as of October 2026. ``member`` is katzenqt's own
state machine, copied unchanged (see member/SOURCES); ``wire`` its formats;
``engine`` the client without IO; ``store`` its state in SQLite; ``client``
the engine over a transport."""
