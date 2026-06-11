# Copyright 2026 Curtis Galloway
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0

"""
Minimal i2ctarget diagnostic v2 — the `with r:` context manager throws
(AttributeError: I2CTargetRequest has no 'deinit') in CircuitPython 9.2.9, which
crashed the loop after frame 1 and wedged the bus. This version drops `with r:`
entirely and relies on read-until-empty to consume the transaction.

It also prints dir(r) once so we can see the real finalize method (close()?).

Watch the NeoPixel + console:
  - alternating green/blue + "MINRX n" climbing  -> fix works, no `with` needed
  - stuck after MINRX 1                           -> need explicit finalize
"""

import board
import digitalio
import neopixel_write
from i2ctarget import I2CTarget

_px = digitalio.DigitalInOut(board.NEOPIXEL)
_px.direction = digitalio.Direction.OUTPUT


def px(r, g, b):
    neopixel_write.neopixel_write(_px, bytearray((g, r, b)))


i2c = I2CTarget(board.MOSI, board.D10, (0x41,))
px(0, 0, 16)
print("MIN v2: i2ctarget up on 0x41 (MOSI/D10), waiting…")

count = 0
while True:
    r = i2c.request()
    if not r:
        continue
    if count == 0:
        print("RDIR", [m for m in dir(r) if m[0] != "_"])
    if r.is_read:
        r.write(b"\x00")
    else:
        while True:
            b = r.read(1)
            if not b:
                break
    count += 1
    print("MINRX", count)
    px(0, 16, 0) if (count % 2) else px(0, 0, 16)
