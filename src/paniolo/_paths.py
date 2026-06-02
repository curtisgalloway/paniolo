# Copyright 2026 Curtis Galloway
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"""Locating the paniolo source checkout.

`paniolo setup` (and `make install`) rebuild the Rust daemons and the OCR helper
*from source*, so they need the git clone — not the installed package tree. The
old approach climbed a fixed number of parents from ``__file__``, which only
points at the repo for a source checkout; from an installed uv tool it lands in
``.../lib/python3.12`` and every source lookup silently fails. ``repo_root()``
resolves the checkout robustly instead.
"""

from __future__ import annotations

from pathlib import Path
from typing import Optional


def _is_repo_root(d: Path) -> bool:
    """True if ``d`` looks like the paniolo source checkout root."""
    return (
        (d / "pyproject.toml").is_file()
        and (d / "ocr").is_dir()
        and (d / "hdmicap" / "Cargo.toml").is_file()
    )


def repo_root() -> Optional[Path]:
    """Locate the paniolo source checkout, or ``None`` if not found.

    Searches, in order:

    1. The tree containing this file. This wins for a source checkout, an
       editable install, or ``uv run``, where ``__file__`` lives under
       ``<repo>/src/paniolo``.
    2. The current working directory and its parents. This covers running an
       *installed* ``paniolo setup`` from inside a clone — notably ``make
       install``, which executes in the repo.

    Returns ``None`` when no checkout can be found (e.g. an installed CLI invoked
    from an unrelated directory); callers should surface a clear "run from a
    clone" error rather than silently skipping the build.
    """
    starts = [Path(__file__).resolve().parent, Path.cwd().resolve()]
    for start in starts:
        for d in (start, *start.parents):
            if _is_repo_root(d):
                return d
    return None
