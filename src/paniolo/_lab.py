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

"""The lab model: one git-tracked file describing all hosts and targets.

A *lab* is a single TOML file (pointed at by ``--lab`` / ``PANIOLO_LAB``) that
declares every control **host** paniolo reaches over SSH and every **target**,
with each piece of a target's hardware bound to a host. See
docs/distributed-control.md.

```toml
[hosts.bench1]
ssh = "curtisg@bench1.local"     # required; "local" means the dev machine
# identity = "~/.ssh/id_lab"     # optional
# control_path = "~/.ssh/cm-%C"  # optional

[targets.fortune]
host = "bench1"                   # default host for this target's resources

[targets.fortune.netboot]
interface = "enx00e04c08d9a0"
host_ip   = "192.168.99.1"
tftp_root = "/home/curtisg/tftp/fortune"

[[targets.fortune.serial]]
name   = "console"
device = "/dev/serial/by-id/usb-…"
baud   = 115200

[targets.fortune.power]
cycle_cmd = "/path/cycle.sh"
```

**Per-resource host binding** (``host`` on any resource, defaulting to the
target's ``host``, defaulting to ``local``) is parsed so the schema is ready for
targets that span control hosts — but this first implementation **enforces that
a target's resources all resolve to one host** and rejects cross-host targets.
Resolution flattens a target down to the existing :class:`TargetConfig` plus the
single :class:`~paniolo._ssh.Host` it lives on, so command bodies are unchanged.
"""

from __future__ import annotations

import dataclasses
import os
import tomllib
from pathlib import Path
from typing import Optional

from ._config import CONFIG_DIR, SerialInterface, TargetConfig
from ._ssh import LOCAL, Host

# Default host name when neither a resource nor its target names one.
DEFAULT_HOST = LOCAL
DEFAULT_HOST_IP = "192.168.99.1"

# The lab file used when neither --lab nor PANIOLO_LAB is given.
DEFAULT_LAB_PATH = str(CONFIG_DIR / "lab.toml")


class LabError(RuntimeError):
    """The lab file is malformed or describes something unsupported."""


def _without_host(d: dict, drop: tuple[str, ...] = ()) -> dict:
    """A channel's scalar fields, minus ``host`` and any explicitly dropped keys."""
    skip = {"host", *drop}
    return {k: v for k, v in d.items() if k not in skip}


@dataclasses.dataclass
class ResolvedChannel:
    """One channel of a target, with its physical host resolved.

    ``kind`` is the channel type ("netboot" | "serial" | "power" | "video");
    ``name`` is the serial interface name, or the kind for singleton channels.
    ``host`` is the host the channel resolves to (its own ``host``, else the
    target's default). ``fields`` holds the remaining scalar config.
    """

    kind: str
    name: str
    host: str
    fields: dict


@dataclasses.dataclass
class ResolvedTarget:
    """A target's channels with per-channel hosts resolved (no single-host rule)."""

    name: str
    default_host: str
    note: Optional[str]
    channels: list[ResolvedChannel]

    def hosts(self) -> list[str]:
        """The distinct hosts this target's channels live on."""
        return sorted({c.host for c in self.channels} or {self.default_host})


@dataclasses.dataclass
class Lab:
    """A parsed lab: named hosts plus raw target tables (resolved on demand)."""

    hosts: dict[str, Host]
    targets: dict[str, dict]

    @classmethod
    def from_dict(cls, data: dict) -> "Lab":
        hosts: dict[str, Host] = {}
        for name, h in (data.get("hosts") or {}).items():
            if "ssh" not in h:
                raise LabError(f"host '{name}': missing required 'ssh' field")
            hosts[name] = Host(
                name=name,
                ssh=h["ssh"],
                identity=h.get("identity"),
                control_path=h.get("control_path"),
                paniolo_cmd=h.get("paniolo_cmd"),
            )
        targets = data.get("targets") or {}
        return cls(hosts=hosts, targets=targets)

    def target_names(self) -> list[str]:
        return sorted(self.targets)

    def _host(self, name: str) -> Host:
        if name == LOCAL:
            # The dev machine is implicit; an explicit [hosts.local] may override.
            return self.hosts.get(LOCAL, Host(name=LOCAL, ssh=LOCAL))
        if name not in self.hosts:
            raise LabError(f"reference to unknown host '{name}'")
        return self.hosts[name]

    def resolve_target(self, name: str) -> tuple[TargetConfig, Host]:
        """Flatten a target to a (TargetConfig, Host), enforcing one host.

        Raises KeyError if the target is unknown, LabError if its resources span
        more than one host (not yet supported) or reference an unknown host.
        """
        if name not in self.targets:
            raise KeyError(name)
        t = self.targets[name]
        default_host = t.get("host", DEFAULT_HOST)
        used: set[str] = set()

        netboot = t.get("netboot") or {}
        if netboot:
            used.add(netboot.get("host", default_host))

        serial_interfaces: list[SerialInterface] = []
        for s in t.get("serial") or []:
            if "name" not in s or "device" not in s:
                raise LabError(f"target '{name}': each [[serial]] needs name + device")
            serial_interfaces.append(
                SerialInterface(
                    name=s["name"],
                    device=s["device"],
                    baud=int(s.get("baud", 115200)),
                    power_sense_signal=s.get("power_sense_signal"),
                )
            )
            used.add(s.get("host", default_host))

        power = t.get("power") or {}
        if power:
            used.add(power.get("host", default_host))

        if not used:
            used.add(default_host)
        if len(used) > 1:
            raise LabError(
                f"target '{name}' spans multiple hosts {sorted(used)}; "
                "multi-host targets are not yet supported"
            )

        cfg = TargetConfig(
            name=name,
            interface=netboot.get("interface", ""),
            host_ip=netboot.get("host_ip", DEFAULT_HOST_IP),
            tftp_root=netboot.get("tftp_root"),
            power_cycle_cmd=power.get("cycle_cmd"),
            power_serial_interface=power.get("serial_interface"),
            serial_interfaces=serial_interfaces,
        )
        return cfg, self._host(next(iter(used)))

    def host_slice(self, name: str, host_name: str) -> TargetConfig:
        """Flatten the channels of ``name`` that live on ``host_name`` to a config.

        This is the slice a single host sees — what runs locally on the dev
        machine (``host_name == "local"``) and what is shipped to a control host
        for remote re-exec. Channels on *other* hosts are omitted, so the result
        is always single-host. Raises KeyError if the target is unknown.
        """
        rt = self.resolved_target(name)
        serial_interfaces: list[SerialInterface] = []
        netboot: dict = {}
        power: dict = {}
        for ch in rt.channels:
            if ch.host != host_name:
                continue
            if ch.kind == "serial":
                serial_interfaces.append(
                    SerialInterface(
                        name=ch.name,
                        device=ch.fields.get("device", ""),
                        baud=int(ch.fields.get("baud", 115200)),
                        power_sense_signal=ch.fields.get("power_sense_signal"),
                    )
                )
            elif ch.kind == "netboot":
                netboot = ch.fields
            elif ch.kind == "power":
                power = ch.fields
        return TargetConfig(
            name=name,
            interface=netboot.get("interface", ""),
            host_ip=netboot.get("host_ip", DEFAULT_HOST_IP),
            tftp_root=netboot.get("tftp_root"),
            power_cycle_cmd=power.get("cycle_cmd"),
            power_serial_interface=power.get("serial_interface"),
            serial_interfaces=serial_interfaces,
        )

    def resolved_target(self, name: str) -> ResolvedTarget:
        """Flatten a target to its channels with per-channel hosts resolved.

        Unlike :meth:`resolve_target`, this imposes no single-host rule — it is
        the read view, and the basis for per-channel dispatch. Raises KeyError
        if the target is unknown.
        """
        if name not in self.targets:
            raise KeyError(name)
        t = self.targets[name]
        default_host = t.get("host", DEFAULT_HOST)
        channels: list[ResolvedChannel] = []

        nb = t.get("netboot")
        if nb:
            channels.append(
                ResolvedChannel(
                    "netboot",
                    "netboot",
                    nb.get("host", default_host),
                    _without_host(nb),
                )
            )
        for s in t.get("serial") or []:
            channels.append(
                ResolvedChannel(
                    "serial",
                    s.get("name", ""),
                    s.get("host", default_host),
                    _without_host(s, drop=("name",)),
                )
            )
        for kind in ("power", "video"):
            c = t.get(kind)
            if c:
                channels.append(
                    ResolvedChannel(
                        kind, kind, c.get("host", default_host), _without_host(c)
                    )
                )
        return ResolvedTarget(name, default_host, t.get("note"), channels)

    def channels_on_host(self, host: str) -> list[tuple[str, ResolvedChannel]]:
        """Every (target, channel) pair whose channel resolves to ``host``."""
        out: list[tuple[str, ResolvedChannel]] = []
        for tname in self.target_names():
            for ch in self.resolved_target(tname).channels:
                if ch.host == host:
                    out.append((tname, ch))
        return out


def propose_target_block(name: str, host: str, inventory: dict) -> str:
    """Render a proposed ``[targets.<name>]`` lab block from a host's hardware.

    ``inventory`` is the shape emitted by ``paniolo discover --json``:
    ``{"ethernet": [{device, active, ...}], "serial": [path, ...], ...}``. One
    value is best-guessed per field (a carrier-up interface, the first serial as
    ``console``); other candidates are listed as comments. Power and tftp_root
    aren't discoverable, so they're left as commented stubs. The result is meant
    to be reviewed, pasted into the lab file, and committed — paniolo never
    writes it for you.
    """
    eths = sorted(inventory.get("ethernet") or [], key=lambda e: (not e.get("active"),))
    serials = inventory.get("serial") or []
    out = [f"[targets.{name}]", f'host = "{host}"', ""]

    out.append(f"[targets.{name}.netboot]")
    if eths:
        note = "  # carrier up" if eths[0].get("active") else ""
        out.append(f'interface = "{eths[0]["device"]}"{note}')
        for e in eths[1:]:
            out.append(f'# interface = "{e["device"]}"  # alternative')
    else:
        out.append('# interface = ""  # no USB-Ethernet interface discovered')
    out.append('# tftp_root = "/path/to/tftp"  # set to enable netboot')
    out.append("")

    if serials:
        out.append(f"[[targets.{name}.serial]]")
        out.append('name = "console"')
        out.append(f'device = "{serials[0]}"')
        out.append("baud = 115200")
        for extra in serials[1:]:
            out.append(f"# another serial device: {extra}")
    else:
        out.append(f"# [[targets.{name}.serial]]  # no serial devices discovered")
    out.append("")
    out.append(f"# [targets.{name}.power]")
    out.append('# cycle_cmd = "/path/to/power-cycle.sh"  # not discoverable')
    return "\n".join(out).rstrip() + "\n"


def load_lab(path: str) -> Lab:
    with open(os.path.expanduser(path), "rb") as f:
        data = tomllib.load(f)
    return Lab.from_dict(data)


# ── "which lab" resolution ───────────────────────────────────────────────────

_override_path: Optional[str] = None


def set_lab_path(path: Optional[str]) -> None:
    """Record a lab path from the CLI (--lab), overriding PANIOLO_LAB."""
    global _override_path
    _override_path = path


def lab_path() -> Optional[str]:
    """The active lab file path.

    Resolution order: ``--lab`` (recorded via :func:`set_lab_path`), then
    ``$PANIOLO_LAB``, then the default ``~/.config/paniolo/lab.toml`` *if it
    exists*. Returns None when none of those resolve, leaving the legacy
    per-target files in play until they are retired.
    """
    explicit = _override_path or os.environ.get("PANIOLO_LAB")
    if explicit:
        return explicit
    if Path(os.path.expanduser(DEFAULT_LAB_PATH)).exists():
        return DEFAULT_LAB_PATH
    return None


def load() -> Optional[Lab]:
    """Load the active lab, or None when none is configured (legacy mode)."""
    path = lab_path()
    if not path:
        return None
    if not Path(os.path.expanduser(path)).exists():
        raise LabError(f"lab file not found: {path}")
    return load_lab(path)
