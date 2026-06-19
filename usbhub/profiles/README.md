<!--
Copyright 2026 Curtis Galloway

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
-->

# Shared usbhub profiles

Community-contributed, **human-verified** model profiles for off-the-shelf USB
hubs, so you don't have to re-run the `usbhub learn` bench workflow for a hub
someone has already mapped.

Each `<model>.toml` records a hub's internal chip cascade and, per physical
port, whether that port actually cuts VBUS. A port is marked `controllable =
true` **only** because a human watched the device on that port physically lose
power — hubs routinely *claim* per-port switching they can't do, so this flag is
an assertion of observed reality, never inferred from descriptors. See the
[usbhub README](../README.md) for the full mental model.

## Available profiles

| Model | Hub | Ports | Verified |
|-------|-----|:-----:|----------|
| `rosonway-rsh-a37s` | Rosonway RSH-A37S, 7-port USB 3 | 7 (all controllable) | flash-drive probe, 2026-06-19 |

## Using a profile

usbhub loads profiles from a single directory. Pick whichever fits:

**Option A — copy it into your usbhub profiles dir** (it's then found with no
extra flags):

```bash
# standalone usbhub
cp rosonway-rsh-a37s.toml ~/.config/usbhub/profiles/

# running under paniolo (helper state dir)
cp rosonway-rsh-a37s.toml ~/.config/paniolo/helpers/usbhub/profiles/
```

**Option B — point `--profile-dir` straight at this directory:**

```bash
usbhub --profile-dir /path/to/paniolo/usbhub/profiles --model rosonway-rsh-a37s status
```

Then drive power as usual (`status` / `state <port>` / `on` / `off` / `cycle`).
The profile resolves by matching its chip cascade against the live hub, so it
keeps working after the hub is replugged into a different host port, and pins
with `--at` only if several identical hubs share one host.

## Important: re-verify if your hardware differs

A profile matches by chip cascade (vendor:product ids + internal topology). If
your unit is a different hardware revision — different chips, or different port
numbering between the USB 2 and USB 3 sides — it may not resolve, or worse,
resolve with stale `controllable` flags. When in doubt, re-run `usbhub learn`
and verify the ports on *your* hub. A wrong `controllable = true` is the one
failure this whole workflow exists to prevent.

## Contributing a profile

1. Run `usbhub learn run` and follow the bench workflow — plug a probe with a
   visible power state (LED, or a phone's charging indicator) into each port and
   confirm with your own eyes whether it loses power. (Don't trust the software
   bus readout alone; it lags the real disconnect by a second or two.)
2. Copy the resulting `<model>.toml` (from your profiles dir) into this
   directory.
3. Add a comment header recording what you verified, with what probe, and when
   — see `rosonway-rsh-a37s.toml` for the format.
4. Add a row to the table above and open a PR.
