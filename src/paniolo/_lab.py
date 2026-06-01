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

from ._config import SerialInterface, TargetConfig
from ._ssh import LOCAL, Host

# Default host name when neither a resource nor its target names one.
DEFAULT_HOST = LOCAL
DEFAULT_HOST_IP = "192.168.99.1"


class LabError(RuntimeError):
    """The lab file is malformed or describes something unsupported."""


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

        # No resources named a host → the target falls on its default host.
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
    """The active lab file path: --lab if given, else $PANIOLO_LAB, else None."""
    return _override_path or os.environ.get("PANIOLO_LAB")


def load() -> Optional[Lab]:
    """Load the active lab, or None when none is configured (legacy mode)."""
    path = lab_path()
    if not path:
        return None
    if not Path(os.path.expanduser(path)).exists():
        raise LabError(f"lab file not found: {path}")
    return load_lab(path)
