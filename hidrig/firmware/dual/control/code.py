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

"""
Dual-board rig — CONTROL firmware: HID relay + DUT power + console bridge.

Reads binary frames from the host over usb_cdc.data and routes them by type:
  - 0x01 HID report frames are relayed VERBATIM over I2C1 to the target board
    (0x41), which injects them as USB-HID into the DUT. This board never
    interprets HID semantics — it is a routing relay.
  - 0x02 control frames are handled locally (ping, version, power) and answered
    on usb_cdc.data. `power` drives a relay/load-switch on the DUT's 5 V so the
    rig can power-cycle a wedged target.
  - 0x03 console frames are a byte-pipe to/from the DUT's serial console on the
    hardware UART (TX=GP0 / RX=GP1): inbound payloads are written to the UART,
    and bytes read from the UART are framed back to the host. This board never
    interprets console content — the hidrig daemon re-exports it as a PTY.

Uniform frame format (length-prefixed for byte-stream parsing, same shape the
target uses):
    [type][b1][len][payload .. len bytes]
      0x01: b1 = report-id, payload = HID report bytes
      0x02: b1 = cmd,       payload = args
      0x03: b1 = port (0),  payload = raw DUT console bytes

The host composes the report bytes and demuxes the console (the Rust
`hidrig serve` daemon); host_send.py is the M2 test driver.

I2C1 controller on D10 (GP10, SDA) / MOSI (GP19, SCL); target peripheral 0x41.
"""

import time

import board
import busio
import digitalio
import neopixel_write
import usb_cdc

I2C_ADDR = 0x41
DEBUG = True

# Control-frame commands (type 0x02).
CMD_PING = 0x01
CMD_VERSION = 0x02
CMD_POWER = 0x03
IMPL_ID = b"dual-control/1"

# Power-relay actions (payload byte 0 of a CMD_POWER frame).
POWER_OFF = 0
POWER_ON = 1
POWER_CYCLE = 2

# DUT power relay: a control-board GPIO driving a load-switch / relay on the
# DUT's 5 V (a Pi 5 pulls ~5 A, so this is a real switch, not the rail itself).
# Pick any free pin — I2C1 uses GP10/GP19, the NeoPixel GP17, and the DUT console
# UART GP0/GP1. RELAY_ACTIVE_HIGH = True means driving the pin high powers the
# DUT; flip it for an active-low switch. DEFAULT_CYCLE_OFF_S is the off-time used
# when `power cycle` does not carry one.
RELAY_PIN = board.D5
RELAY_ACTIVE_HIGH = True
DEFAULT_CYCLE_OFF_S = 2

# DUT serial console on the hardware UART (UART0: TX=GP0, RX=GP1). Bridged to
# the host as 0x03 frames. A larger RX ring absorbs DUT boot-log bursts while
# the loop is busy relaying HID over I2C.
CONSOLE_BAUD = 115200
CONSOLE_RX_BUF = 256
CONSOLE_PORT = 0  # frame selector (leaves room for a second UART later)

# --- Status NeoPixel (core neopixel_write; WS2812 is GRB) --------------------
_px = digitalio.DigitalInOut(board.NEOPIXEL)
_px.direction = digitalio.Direction.OUTPUT


def status(r, g, b):
    neopixel_write.neopixel_write(_px, bytearray((g, r, b)))


# --- DUT power relay --------------------------------------------------------
_relay = digitalio.DigitalInOut(RELAY_PIN)
_relay.direction = digitalio.Direction.OUTPUT


def set_power(on):
    _relay.value = on if RELAY_ACTIVE_HIGH else (not on)


def apply_power(action, secs):
    """Drive the relay for a CMD_POWER frame. Cycle blocks the loop for the
    off-time, which is fine — the DUT (and thus HID) is down during a cycle."""
    if action == POWER_OFF:
        set_power(False)
    elif action == POWER_ON:
        set_power(True)
    elif action == POWER_CYCLE:
        set_power(False)
        time.sleep(secs if secs else DEFAULT_CYCLE_OFF_S)
        set_power(True)


set_power(True)  # DUT powered by default when the rig comes up


# scl = MOSI (GP19), sda = D10 (GP10)  ->  I2C1, the inter-board link.
i2c = busio.I2C(board.MOSI, board.D10, frequency=100000)

data = usb_cdc.data  # binary frame channel from the host (None if not enabled)

# DUT serial console — hardware UART0 (TX=GP0, RX=GP1).
try:
    uart = busio.UART(
        board.TX,
        board.RX,
        baudrate=CONSOLE_BAUD,
        timeout=0,  # non-blocking reads
        receiver_buffer_size=CONSOLE_RX_BUF,
    )
except (ValueError, RuntimeError) as e:
    uart = None
    if DEBUG:
        print("control: DUT console UART unavailable:", e)


def relay_hid(frame):
    """Forward an HID frame verbatim over I2C to the target board."""
    while not i2c.try_lock():
        pass
    try:
        i2c.writeto(I2C_ADDR, frame)
        status(0, 16, 0)  # green: relayed + ACKed
    except OSError as e:
        status(16, 0, 0)  # red: I2C relay failed
        if DEBUG:
            print("control: I2C relay failed:", e)
    finally:
        i2c.unlock()


def handle_control(frame):
    """Answer a local control frame on usb_cdc.data."""
    cmd = frame[1]
    if cmd == CMD_PING:
        if data is not None:
            data.write(bytes((0x02, CMD_PING, 0)))  # ping ack
    elif cmd == CMD_VERSION:
        if data is not None:
            data.write(bytes((0x02, CMD_VERSION, len(IMPL_ID))) + IMPL_ID)
    elif cmd == CMD_POWER:
        action = frame[3] if len(frame) > 3 else POWER_OFF
        secs = frame[4] if len(frame) > 4 else 0
        if data is not None:
            data.write(bytes((0x02, CMD_POWER, 0)))  # ack before acting (cycle blocks)
        apply_power(action, secs)
    if DEBUG:
        print("control: ctrl cmd 0x%02X" % cmd)


def relay_console_tx(frame):
    """Write a host->DUT console frame's payload to the DUT UART (verbatim)."""
    if uart is not None and len(frame) > 3:
        uart.write(frame[3:])


def pump_console_rx():
    """Drain the DUT UART and frame received bytes up to the host as 0x03."""
    if uart is None or data is None:
        return
    pending = uart.in_waiting
    while pending:
        chunk = uart.read(pending if pending < 255 else 255)
        if not chunk:
            break
        data.write(bytes((0x03, CONSOLE_PORT, len(chunk))) + chunk)
        pending = uart.in_waiting


_rxbuf = bytearray()


def route_frames():
    # Walk an index and reassign the tail (MicroPython bytearray has no
    # slice-delete). Same length-prefixed parse as the target.
    global _rxbuf
    i = 0
    n = len(_rxbuf)
    while n - i >= 1:
        ftype = _rxbuf[i]
        if ftype == 0x01 or ftype == 0x02 or ftype == 0x03:
            if n - i < 3:
                break  # header incomplete
            need = 3 + _rxbuf[i + 2]
            if n - i < need:
                break  # payload incomplete
            frame = bytes(_rxbuf[i:i + need])
            i += need
            if ftype == 0x01:
                relay_hid(frame)
            elif ftype == 0x02:
                handle_control(frame)
            else:
                relay_console_tx(frame)
        else:
            i += 1  # unframed/unknown byte — resync
    if i:
        _rxbuf = _rxbuf[i:]


if data is not None:
    data.timeout = 0  # non-blocking reads

status(0, 0, 16)  # blue: up, waiting for host frames
if DEBUG:
    print("control: relay up — target 0x%02X, console %s"
          % (I2C_ADDR, "on" if uart is not None else "off"))

while True:
    if data is not None:
        n = data.in_waiting
        if n:
            _rxbuf.extend(data.read(n))
            route_frames()
            status(0, 0, 16)  # back to blue between bursts
    pump_console_rx()
