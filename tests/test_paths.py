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

"""Tests for source-checkout resolution.

Regression coverage for the `make install` bug where `paniolo setup`, run from
the *installed* uv tool, climbed a fixed number of parents from __file__ and
landed in `.../lib/python3.12` — silently failing to find the OCR source and the
Rust crates. `repo_root()` must instead find the checkout via __file__ OR the
cwd, and return None (so callers error clearly) when neither points at one."""

from __future__ import annotations

import pytest

from paniolo import _ocr, _paths


def _make_fake_repo(root):
    """Create the minimal marker layout `_is_repo_root` looks for."""
    root.mkdir(parents=True, exist_ok=True)
    (root / "pyproject.toml").write_text("[project]\nname = 'paniolo'\n")
    (root / "ocr").mkdir()
    (root / "ocr" / "visionocr.swift").write_text("// stub\n")
    (root / "hdmicap").mkdir()
    (root / "hdmicap" / "Cargo.toml").write_text("[package]\nname = 'hdmicap'\n")
    return root


def test_repo_root_finds_real_checkout():
    """In the source tree, __file__ resolution locates this very repo."""
    root = _paths.repo_root()
    assert root is not None
    assert (root / "ocr" / "visionocr.swift").exists()
    assert (root / "hdmicap" / "Cargo.toml").exists()


def test_repo_root_via_cwd_when_file_is_elsewhere(tmp_path, monkeypatch):
    """An installed CLI (__file__ outside any clone) still resolves via cwd.

    This is the `make install` case: the uv-tool copy runs from inside the repo.
    """
    fake_repo = _make_fake_repo(tmp_path / "clone")
    monkeypatch.setattr(_paths, "__file__", "/opt/installed/paniolo/_paths.py")
    monkeypatch.chdir(fake_repo)
    assert _paths.repo_root() == fake_repo


def test_repo_root_via_cwd_parent(tmp_path, monkeypatch):
    """cwd resolution walks up to an ancestor checkout, not just cwd itself."""
    fake_repo = _make_fake_repo(tmp_path / "clone")
    sub = fake_repo / "deep" / "nested"
    sub.mkdir(parents=True)
    monkeypatch.setattr(_paths, "__file__", "/opt/installed/paniolo/_paths.py")
    monkeypatch.chdir(sub)
    assert _paths.repo_root() == fake_repo


def test_repo_root_none_outside_checkout(tmp_path, monkeypatch):
    """No checkout from __file__ or cwd -> None (callers surface a clear error)."""
    monkeypatch.setattr(_paths, "__file__", "/opt/installed/paniolo/_paths.py")
    monkeypatch.chdir(tmp_path)
    assert _paths.repo_root() is None


def test_ocr_sources_none_without_repo(monkeypatch):
    """OCR source helpers degrade to None rather than a bogus path."""
    monkeypatch.setattr(_ocr, "repo_root", lambda: None)
    assert _ocr.visionocr_source() is None
    assert _ocr.linuxocr_source() is None


def test_build_visionocr_raises_without_repo(monkeypatch, tmp_path):
    """build_visionocr raises a clear FileNotFoundError when no checkout exists."""
    monkeypatch.setattr(_ocr, "repo_root", lambda: None)
    with pytest.raises(FileNotFoundError, match="source checkout not found"):
        _ocr.build_visionocr(tmp_path / "out")


def test_install_linuxocr_raises_without_repo(monkeypatch, tmp_path):
    """install_linuxocr raises a clear FileNotFoundError when no checkout exists."""
    monkeypatch.setattr(_ocr, "repo_root", lambda: None)
    with pytest.raises(FileNotFoundError, match="source checkout not found"):
        _ocr.install_linuxocr(tmp_path / "out")
