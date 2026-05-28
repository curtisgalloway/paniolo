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

"""OCR helpers — wraps the `visionocr` Swift tool (Apple Vision framework).

On-device, no network, no model download. macOS only. The tool reads a PNG on
stdin and prints recognized text (or `--json` with bounding boxes).
"""

from __future__ import annotations

import shutil
import subprocess
from pathlib import Path
from typing import Optional


def visionocr_binary() -> Optional[str]:
    """Return the installed visionocr path: PATH, then ~/.cargo/bin. None if absent.

    Built and installed by `paniolo setup`; never resolved from the in-repo build
    tree, so a running daemon can't point at an ephemeral build artifact that a
    checkout/cleanup could delete.
    """
    found = shutil.which("visionocr")
    if found:
        return found
    cargo_bin = Path.home() / ".cargo" / "bin" / "visionocr"
    return str(cargo_bin) if cargo_bin.exists() else None


def visionocr_source() -> Path:
    """Path to the visionocr Swift source in the repo (for `paniolo setup`)."""
    return Path(__file__).parent.parent.parent / "ocr" / "visionocr.swift"


def build_visionocr(dest: Path) -> None:
    """Compile visionocr.swift to `dest` (used by `paniolo setup`). Raises on error."""
    source = visionocr_source()
    if not source.exists():
        raise FileNotFoundError(f"visionocr source not found: {source}")
    if not shutil.which("swiftc"):
        raise FileNotFoundError("swiftc not found (install Xcode command line tools)")
    dest.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(["swiftc", "-O", "-o", str(dest), str(source)], check=True)


def read_text(png: bytes, fast: bool = False, as_json: bool = False) -> str:
    """OCR PNG bytes and return recognized text (or JSON with bboxes)."""
    binary = visionocr_binary()
    if not binary:
        raise FileNotFoundError("visionocr not installed — run: paniolo setup")
    cmd = [binary]
    if fast:
        cmd.append("--fast")
    if as_json:
        cmd.append("--json")
    cmd.append("-")
    result = subprocess.run(cmd, input=png, capture_output=True)
    if result.returncode != 0:
        raise RuntimeError(result.stderr.decode(errors="replace").strip() or "visionocr failed")
    return result.stdout.decode(errors="replace")
