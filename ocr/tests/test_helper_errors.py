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

"""Tests for issue #168: every failure in ocr/linuxocr and ocr/rapidocr leaves
by `die()` -- one line on stderr, non-zero exit -- rather than as a traceback.

docs/dev/ocr.md makes that a promise of the helper protocol ("Errors are one
line, on stderr, non-zero exit"), and two paths broke it: a bare `open()` on a
missing input file, and `_preprocess`'s `except ImportError`, which caught a
missing Pillow but not Pillow's own `UnidentifiedImageError` on bytes that are
no image at all.

`linuxocr` is exercised end-to-end, as a subprocess, because that is the only
way to see what a caller actually gets: the exit status, and stderr with no
traceback on it. `rapidocr` is exercised through the same functions instead --
its `main()` imports numpy, rapidocr and cv2 before it reads anything, so a
subprocess here would only ever report those as missing. Loading the module by
path runs module-level code alone, which needs none of them (the property
test_input_bounds.py already relies on).

Both scripts have no .py suffix, so each is loaded by path with importlib
rather than imported as a regular module.
"""

from __future__ import annotations

import importlib.machinery
import importlib.util
import io
import pathlib
import subprocess
import sys
import types

import pytest

_OCR_DIR = pathlib.Path(__file__).resolve().parent.parent


def _load_by_path(path: pathlib.Path, name: str) -> types.ModuleType:
    loader = importlib.machinery.SourceFileLoader(name, str(path))
    spec = importlib.util.spec_from_loader(loader.name, loader)
    assert spec is not None
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module


linuxocr = _load_by_path(_OCR_DIR / "linuxocr", "linuxocr_errors_under_test")
rapidocr = _load_by_path(_OCR_DIR / "rapidocr", "rapidocr_errors_under_test")

_HELPERS = [
    pytest.param(linuxocr, "linuxocr", id="linuxocr"),
    pytest.param(rapidocr, "rapidocr", id="rapidocr"),
]


def _assert_one_line_error(err: str, helper: str) -> None:
    """The shape docs/dev/ocr.md promises, asserted in one place.

    A traceback fails all three ways at once -- it does not start with the
    helper's name, it runs to many lines, and it names the exception class --
    so checking the shape is enough; there is no need to also grep for
    "Traceback", which the line count already rules out.
    """
    assert err.startswith(f"{helper}: "), f"expected a {helper}: prefix, got {err!r}"
    assert err.count("\n") == 1, f"expected exactly one line, got {err!r}"
    assert err.endswith("\n"), f"expected a trailing newline, got {err!r}"


def _run_linuxocr(args: list, stdin: bytes = b"") -> subprocess.CompletedProcess:
    """Run ocr/linuxocr as a subprocess under *this* interpreter.

    `sys.executable` rather than the script's `#!/usr/bin/env python3`
    shebang, so the child sees the same environment pytest is running in --
    Pillow in particular, which the CI job installs and which decides which
    branch of `_preprocess` runs.
    """
    return subprocess.run(
        [sys.executable, str(_OCR_DIR / "linuxocr"), *args],
        input=stdin,
        capture_output=True,
        check=False,
    )


@pytest.mark.parametrize("module,helper", _HELPERS)
def test_read_input_reports_a_missing_file(module, helper, tmp_path, capsys):
    """The `FileNotFoundError` traceback from #168's first bullet, in both
    helpers. The path is under pytest's tmp_path, so it is certain not to
    exist and certain not to be somewhere the test could create it.
    """
    with pytest.raises(SystemExit):
        module._read_input(str(tmp_path / "no-such-frame.png"))
    _assert_one_line_error(capsys.readouterr().err, helper)


@pytest.mark.parametrize("module,helper", _HELPERS)
def test_read_input_reports_a_directory(module, helper, tmp_path, capsys):
    """A path that exists but is not a file: `IsADirectoryError`, a different
    `OSError` down the same road. Worth its own case because an implementation
    that only caught `FileNotFoundError` would pass the test above.
    """
    with pytest.raises(SystemExit):
        module._read_input(str(tmp_path))
    _assert_one_line_error(capsys.readouterr().err, helper)


@pytest.mark.parametrize("module,helper", _HELPERS)
def test_read_input_still_reads_a_real_file(module, helper, tmp_path):
    """The happy path has to survive the error handling around it."""
    frame = tmp_path / "frame.png"
    frame.write_bytes(b"\x89PNG\r\n\x1a\n" + b"payload")
    assert module._read_input(str(frame)) == b"\x89PNG\r\n\x1a\n" + b"payload"


@pytest.mark.parametrize("module,helper", _HELPERS)
def test_read_input_still_enforces_the_byte_cap(module, helper, tmp_path, capsys):
    """`_read_bounded` fails by raising `SystemExit`, which is not an
    `OSError`, so the over-the-cap message must reach the caller unchanged
    rather than being relabelled as a read error.
    """
    frame = tmp_path / "huge.png"
    frame.write_bytes(b"x" * (module.MAX_ENCODED_BYTES + 1))
    with pytest.raises(SystemExit):
        module._read_input(str(frame))
    err = capsys.readouterr().err
    _assert_one_line_error(err, helper)
    assert "encoded-input limit" in err


def test_linuxocr_preprocess_reports_non_image_bytes(capsys):
    """#168's second bullet: with Pillow installed, `Image.open` on bytes that
    are no image raises `UnidentifiedImageError`, which the old
    `except ImportError` did not catch.
    """
    pytest.importorskip("PIL", reason="_preprocess only decodes when Pillow is present")
    with pytest.raises(SystemExit):
        linuxocr._preprocess(b"not a png")
    _assert_one_line_error(capsys.readouterr().err, "linuxocr")


def test_linuxocr_preprocess_does_not_relabel_a_dimension_error(monkeypatch, capsys):
    """The decode handler must not swallow the resource-limit check inside it.

    `_check_dimensions` fails by raising `SystemExit`, which is not an
    `Exception` -- so an over-limit image must still be reported with its own
    message rather than as an undecodable one. Forced here by shrinking the
    limit rather than by building a huge image.
    """
    Image = pytest.importorskip("PIL.Image", reason="needs Pillow to reach the check")
    buf = io.BytesIO()
    Image.new("RGB", (2, 2), (0, 0, 0)).save(buf, "PNG")
    monkeypatch.setattr(linuxocr, "MAX_DIMENSION", 1)
    with pytest.raises(SystemExit):
        linuxocr._preprocess(buf.getvalue())
    err = capsys.readouterr().err
    _assert_one_line_error(err, "linuxocr")
    assert "exceeds the 1px-per-side limit" in err
    assert "could not decode" not in err


def test_linuxocr_subprocess_reports_a_missing_file(tmp_path):
    """End-to-end, which is where the exit status lives: `linuxocr` on a path
    that does not exist must exit non-zero with one line on stderr. It never
    reaches tesseract, so this runs on a host without it.
    """
    proc = _run_linuxocr([str(tmp_path / "no-such-frame.png"), "--json"])
    assert proc.returncode != 0
    _assert_one_line_error(proc.stderr.decode(), "linuxocr")
    assert proc.stdout == b""


def test_linuxocr_subprocess_reports_non_image_stdin():
    """`printf 'not a png' | linuxocr - --json` from the issue, run for real.

    With Pillow present the failure happens in `_preprocess`, before tesseract
    is invoked, so this needs no OCR engine installed either.
    """
    pytest.importorskip("PIL", reason="_preprocess only decodes when Pillow is present")
    proc = _run_linuxocr(["-", "--json"], stdin=b"not a png")
    assert proc.returncode != 0
    _assert_one_line_error(proc.stderr.decode(), "linuxocr")
    assert proc.stdout == b""
