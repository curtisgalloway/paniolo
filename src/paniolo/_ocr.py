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

"""OCR helpers — wraps platform OCR tools.

- macOS: `visionocr` (Apple Vision framework, compiled from ocr/visionocr.swift)
- Linux: `linuxocr` (Tesseract-backed, from ocr/linuxocr)

Both tools share the same interface: read PNG on stdin, print text on stdout.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path
from typing import Optional


def visionocr_binary() -> Optional[str]:
    """Return the installed visionocr path: PATH, then ~/.cargo/bin. None if absent."""
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


def linuxocr_binary() -> Optional[str]:
    """Return the installed linuxocr path: PATH, then ~/.cargo/bin. None if absent."""
    found = shutil.which("linuxocr")
    if found:
        return found
    cargo_bin = Path.home() / ".cargo" / "bin" / "linuxocr"
    return str(cargo_bin) if cargo_bin.exists() else None


def linuxocr_source() -> Path:
    """Path to the linuxocr Python script in the repo (for `paniolo setup`)."""
    return Path(__file__).parent.parent.parent / "ocr" / "linuxocr"


def install_linuxocr(dest: Path) -> None:
    """Copy ocr/linuxocr to `dest` and make it executable (used by `paniolo setup`)."""
    import shutil as _shutil
    source = linuxocr_source()
    if not source.exists():
        raise FileNotFoundError(f"linuxocr source not found: {source}")
    dest.parent.mkdir(parents=True, exist_ok=True)
    _shutil.copy2(source, dest)
    dest.chmod(0o755)


def ocr_binary() -> Optional[str]:
    """Return the platform OCR binary: visionocr on macOS, linuxocr on Linux."""
    if sys.platform == "darwin":
        return visionocr_binary()
    return linuxocr_binary()


def read_text(png: bytes, fast: bool = False, as_json: bool = False) -> str:
    """OCR PNG bytes and return recognized text (or JSON with bboxes).

    `fast` is only meaningful on macOS (visionocr --fast); ignored on Linux.
    `as_json` requests bounding-box JSON output; not yet supported on Linux.
    """
    binary = ocr_binary()
    if not binary:
        platform = "macOS" if sys.platform == "darwin" else "Linux"
        tool = "visionocr" if sys.platform == "darwin" else "linuxocr"
        raise FileNotFoundError(f"{tool} not installed on {platform} — run: paniolo setup")
    cmd = [binary]
    if sys.platform == "darwin":
        if fast:
            cmd.append("--fast")
        if as_json:
            cmd.append("--json")
    cmd.append("-")
    result = subprocess.run(cmd, input=png, capture_output=True)
    if result.returncode != 0:
        raise RuntimeError(result.stderr.decode(errors="replace").strip() or f"{binary} failed")
    return result.stdout.decode(errors="replace")
