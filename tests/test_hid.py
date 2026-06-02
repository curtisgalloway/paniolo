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

"""Host-side tests for the HID rig client — no hardware, no pyserial."""

from __future__ import annotations

import pytest

from paniolo import _hid


class FakeTransport:
    """Stand-in for a pyserial port: records writes, replies from a queue."""

    def __init__(self, replies=None):
        self.writes: list[bytes] = []
        self._replies = list(replies) if replies else []
        self.closed = False

    def write(self, data: bytes) -> None:
        self.writes.append(data)

    def readline(self) -> bytes:
        return self._replies.pop(0) if self._replies else b"OK\n"

    def close(self) -> None:
        self.closed = True


def make_rig(replies=None):
    t = FakeTransport(replies)
    return _hid.HidRig(transport=t), t


# --- command construction ---------------------------------------------------


@pytest.mark.parametrize(
    "call, expected",
    [
        (lambda r: r.type("hello world"), b"type hello world\n"),
        (lambda r: r.key("ENTER"), b"key ENTER\n"),
        (lambda r: r.combo("LEFT_CONTROL", "C"), b"combo LEFT_CONTROL C\n"),
        (lambda r: r.down("LEFT_SHIFT"), b"down LEFT_SHIFT\n"),
        (lambda r: r.up("LEFT_SHIFT"), b"up LEFT_SHIFT\n"),
        (lambda r: r.releaseall(), b"releaseall\n"),
        (lambda r: r.move(300, -50), b"move 300 -50\n"),
        (lambda r: r.click(), b"click left\n"),
        (lambda r: r.click("right"), b"click right\n"),
        (lambda r: r.mdown("middle"), b"mdown middle\n"),
        (lambda r: r.mup("middle"), b"mup middle\n"),
        (lambda r: r.scroll(-3), b"scroll -3\n"),
    ],
)
def test_command_construction(call, expected):
    rig, t = make_rig()
    call(rig)
    assert t.writes == [expected]


def test_cmd_returns_reply():
    rig, _ = make_rig(replies=[b"OK\n"])
    assert rig.cmd("releaseall") == "OK"


def test_cmd_raises_on_err():
    rig, _ = make_rig(replies=[b"ERR unknown command: frob\n"])
    with pytest.raises(RuntimeError, match="control board rejected"):
        rig.cmd("frob")


def test_close_delegates():
    rig, t = make_rig()
    rig.close()
    assert t.closed


# --- absolute-mouse scaling -------------------------------------------------


@pytest.mark.parametrize(
    "px, screen, expected",
    [
        (0, 1920, 0),
        (1919, 1920, 32767),
        (960, 1920, 16392),
        (-100, 1920, 0),  # clamp low
        (99999, 1920, 32767),  # clamp high
        (5, 1, 0),  # degenerate screen size
    ],
)
def test_scale_to_logical(px, screen, expected):
    assert _hid.scale_to_logical(px, screen) == expected


# --- sequence parsing -------------------------------------------------------


def test_parse_sequence_skips_blanks_and_comments():
    text = "\n# a comment\n  \nkey ENTER\n# another\ntype hi\n"
    assert _hid.parse_sequence(text) == [("cmd", "key ENTER"), ("cmd", "type hi")]


def test_parse_sequence_timing_directives():
    text = "delay 250\nkey A\nsleep 2\n"
    assert _hid.parse_sequence(text) == [
        ("delay", 0.25),
        ("cmd", "key A"),
        ("delay", 2.0),
    ]


def test_run_sequence_executes_in_order_with_delays():
    rig, t = make_rig(replies=[b"OK\n", b"OK\n"])
    slept: list[float] = []
    steps = [("cmd", "key A"), ("delay", 0.5), ("cmd", "type hi")]
    _hid.run_sequence(rig, steps, default_delay=0.0, sleep=slept.append)
    assert t.writes == [b"key A\n", b"type hi\n"]
    assert slept == [0.5]


def test_run_sequence_default_delay_between_commands():
    rig, _ = make_rig(replies=[b"OK\n", b"OK\n"])
    slept: list[float] = []
    steps = [("cmd", "key A"), ("cmd", "key B")]
    _hid.run_sequence(rig, steps, default_delay=0.1, sleep=slept.append)
    assert slept == [0.1, 0.1]


def test_repeat_key():
    rig, t = make_rig(replies=[b"OK\n"] * 3)
    slept: list[float] = []
    _hid.repeat_key(rig, "TAB", 3, delay=0.2, sleep=slept.append)
    assert t.writes == [b"key TAB\n"] * 3
    assert slept == [0.2, 0.2]  # no trailing delay after the last tap


# --- S3: newline injection guard -------------------------------------------


def test_cmd_rejects_newline():
    rig, _ = make_rig()
    with pytest.raises(ValueError, match="newline"):
        rig.cmd("type hello\nkey ENTER")


def test_cmd_rejects_carriage_return():
    rig, _ = make_rig()
    with pytest.raises(ValueError, match="newline"):
        rig.cmd("type hello\rworld")


def test_type_rejects_embedded_newline():
    rig, _ = make_rig()
    with pytest.raises(ValueError, match="newline"):
        rig.type("line1\nline2")


# --- C3: parse_sequence error messages -------------------------------------


def test_parse_sequence_bad_delay_raises_friendly_error():
    with pytest.raises(ValueError, match="invalid delay value"):
        _hid.parse_sequence("delay abc\nkey A\n")


def test_parse_sequence_bad_sleep_raises_friendly_error():
    with pytest.raises(ValueError, match="invalid sleep value"):
        _hid.parse_sequence("sleep xyz\n")
