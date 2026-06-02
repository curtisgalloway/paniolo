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

"""Target configuration — load, save, and parse TOML config files for paniolo."""

from __future__ import annotations

import dataclasses
import logging
import re
import tomllib
from pathlib import Path
from typing import Optional

log = logging.getLogger(__name__)

_CTRL_RE = re.compile(r"[\x00-\x1f\x7f]")
_CTRL_ESCAPE = {"\n": "\\n", "\r": "\\r", "\t": "\\t", "\x08": "\\b", "\x0c": "\\f"}

CONFIG_DIR = Path.home() / ".config" / "paniolo"
TARGETS_DIR = CONFIG_DIR / "targets"

DEFAULT_SERIAL_NAME = "console"


VALID_SENSE_SIGNALS = ("cts", "dsr", "dcd", "ri")


@dataclasses.dataclass
class SerialInterface:
    """A named serial console attached to a target (e.g. 'console', 'bmc')."""

    name: str
    device: str
    baud: int = 115200
    power_sense_signal: Optional[str] = None  # "cts" | "dsr" | "dcd" | "ri" | None


@dataclasses.dataclass
class TargetConfig:
    """Configuration record for a single paniolo-managed target machine."""

    name: str
    interface: str
    host_ip: str = "192.168.99.1"
    tftp_root: Optional[str] = None
    power_cycle_cmd: Optional[str] = None
    power_serial_interface: Optional[str] = None
    serial_interfaces: list[SerialInterface] = dataclasses.field(default_factory=list)

    def serial_interface(self, name: Optional[str] = None) -> SerialInterface:
        """Resolve a serial interface by name, defaulting to the sole one.

        Raises ValueError if none are configured, the name is unknown, or no name
        was given but several exist (ambiguous)."""
        if not self.serial_interfaces:
            raise ValueError(f"no serial interfaces configured for '{self.name}'")
        if name is None:
            if len(self.serial_interfaces) == 1:
                return self.serial_interfaces[0]
            have = ", ".join(i.name for i in self.serial_interfaces)
            raise ValueError(
                f"multiple serial interfaces ({have}); specify one with --interface"
            )
        for iface in self.serial_interfaces:
            if iface.name == name:
                return iface
        have = ", ".join(i.name for i in self.serial_interfaces)
        raise ValueError(f"no serial interface '{name}' (have: {have})")

    def upsert_serial_interface(self, iface: SerialInterface) -> None:
        """Add the interface, or replace an existing one with the same name."""
        for idx, existing in enumerate(self.serial_interfaces):
            if existing.name == iface.name:
                self.serial_interfaces[idx] = iface
                return
        self.serial_interfaces.append(iface)

    def remove_serial_interface(self, name: str) -> bool:
        """Drop the named interface; return True if one was removed."""
        kept = [i for i in self.serial_interfaces if i.name != name]
        removed = len(kept) != len(self.serial_interfaces)
        self.serial_interfaces = kept
        return removed


def target_path(name: str) -> Path:
    return TARGETS_DIR / f"{name}.toml"


def save_target(cfg: TargetConfig) -> None:
    TARGETS_DIR.mkdir(parents=True, exist_ok=True)
    target_path(cfg.name).write_text(_to_toml(cfg))


def load_target(name: str) -> TargetConfig:
    path = target_path(name)
    if not path.exists():
        raise FileNotFoundError(name)
    with open(path, "rb") as f:
        data = tomllib.load(f)
    return _from_dict(data)


def load_target_file(path: str) -> TargetConfig:
    """Load a TargetConfig from an arbitrary TOML file (not the targets dir).

    Used for the config slice shipped to a remote host (PANIOLO_TARGET_CONFIG):
    the stateless control host runs against this single injected target.
    """
    with open(path, "rb") as f:
        data = tomllib.load(f)
    return _from_dict(data)


def list_targets() -> list[str]:
    if not TARGETS_DIR.exists():
        return []
    return sorted(p.stem for p in TARGETS_DIR.glob("*.toml"))


def _from_dict(data: dict) -> TargetConfig:
    """Build a TargetConfig from parsed TOML, migrating the legacy single-serial
    fields (`serial_device`/`serial_baud`) into a named interface."""
    data = dict(data)
    serial = data.pop("serial", None)
    legacy_device = data.pop("serial_device", None)
    legacy_baud = data.pop("serial_baud", None)
    data.pop(
        "ha_power_entity", None
    )  # removed field — ignore if present in old configs

    interfaces: list[SerialInterface] = []
    if serial:
        for entry in serial:
            interfaces.append(
                SerialInterface(
                    name=entry["name"],
                    device=entry["device"],
                    baud=int(entry.get("baud", 115200)),
                    power_sense_signal=entry.get("power_sense_signal"),
                )
            )
    elif legacy_device:
        interfaces.append(
            SerialInterface(
                name=DEFAULT_SERIAL_NAME,
                device=legacy_device,
                baud=int(legacy_baud or 115200),
            )
        )

    known_fields = {f.name for f in dataclasses.fields(TargetConfig)} - {
        "serial_interfaces"
    }
    unknown = set(data) - known_fields
    if unknown:
        log.warning("ignoring unknown config keys: %s", ", ".join(sorted(unknown)))
    data = {k: v for k, v in data.items() if k in known_fields}
    return TargetConfig(serial_interfaces=interfaces, **data)


def _escape_toml_string(s: str) -> str:
    """Escape a string for use in a TOML basic string (double-quoted)."""
    s = s.replace("\\", "\\\\").replace('"', '\\"')
    return _CTRL_RE.sub(
        lambda m: _CTRL_ESCAPE.get(m.group(), f"\\u{ord(m.group()):04x}"), s
    )


def _toml_kv(key: str, value) -> str:
    if isinstance(value, bool):
        return f'{key} = {"true" if value else "false"}'
    if isinstance(value, str):
        return f'{key} = "{_escape_toml_string(value)}"'
    return f"{key} = {value}"


def _to_toml(cfg: TargetConfig) -> str:
    scalars = {
        "name": cfg.name,
        "interface": cfg.interface,
        "host_ip": cfg.host_ip,
        "tftp_root": cfg.tftp_root,
        "power_cycle_cmd": cfg.power_cycle_cmd,
        "power_serial_interface": cfg.power_serial_interface,
    }
    lines = [_toml_kv(k, v) for k, v in scalars.items() if v is not None]
    out = "\n".join(lines) + "\n"
    for iface in cfg.serial_interfaces:
        out += "\n[[serial]]\n"
        out += _toml_kv("name", iface.name) + "\n"
        out += _toml_kv("device", iface.device) + "\n"
        out += _toml_kv("baud", iface.baud) + "\n"
        if iface.power_sense_signal is not None:
            out += _toml_kv("power_sense_signal", iface.power_sense_signal) + "\n"
    return out
