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

"""The editable lab document: surgical, comment-preserving writes via tomlkit.

The lab file is the single, git-tracked source of truth for paniolo's
configuration (see docs/config-redesign.md). It is human-authored, so the CLI
must edit it *politely* — preserving hand-written comments, key ordering, and
formatting, touching only the tables it changes. That rules out
parse-then-regenerate; this module wraps a live ``tomlkit`` document and mutates
it in place.

Reading/resolution and the typed model live in :mod:`paniolo._lab`; this module
is purely the write side plus the shared :func:`validate` (run on load and after
every mutation). The machine-generated remote slice
(``PANIOLO_TARGET_CONFIG``) does *not* go through here — it is ephemeral and
never hand-edited, so it keeps using a plain dump.
"""

from __future__ import annotations

import os
from collections.abc import Mapping
from pathlib import Path
from typing import Optional

import tomlkit

from ._config import VALID_SENSE_SIGNALS
from ._lab import LabError
from ._ssh import LOCAL

# Singleton channel types: at most one per target, so the type *is* the name.
SINGLETON_CHANNELS = ("netboot", "power", "video")

_HEADER = "paniolo lab — managed by the `paniolo` CLI; hand-edits are preserved"


def _is_table(value: object) -> bool:
    """True for a TOML table (mapping), False for an array-of-tables or scalar."""
    return isinstance(value, Mapping)


def _check_host_ref(host: str, declared: set[str], where: str) -> None:
    if host not in declared:
        known = ", ".join(sorted(declared)) or "(none)"
        raise LabError(f"{where} references unknown host '{host}' (declared: {known})")


def validate(data: Mapping) -> None:
    """Raise :class:`LabError` if ``data`` is not a structurally valid lab.

    Works on either a ``tomlkit`` document or a plain dict (from ``tomllib``),
    so the loader and the writer share one rulebook. Checks: every host has an
    ``ssh`` destination; every host reference (target default and per-channel)
    points at ``local`` or a declared host; serial names are unique within a
    target; the singleton channels are single tables, not arrays.
    """
    hosts = data.get("hosts") or {}
    if not _is_table(hosts):
        raise LabError("'hosts' must be a table")
    declared: set[str] = set(hosts) | {LOCAL}
    for hname, h in hosts.items():
        if not _is_table(h) or not h.get("ssh"):
            raise LabError(f"host '{hname}': missing required 'ssh' field")

    targets = data.get("targets") or {}
    if not _is_table(targets):
        raise LabError("'targets' must be a table")
    for tname, t in targets.items():
        if not _is_table(t):
            raise LabError(f"target '{tname}': must be a table")
        default_host = t.get("host", LOCAL)
        _check_host_ref(default_host, declared, f"target '{tname}'")

        for ch in SINGLETON_CHANNELS:
            c = t.get(ch)
            if c is None:
                continue
            if not _is_table(c):
                raise LabError(
                    f"target '{tname}': [{ch}] must be a single table, not an array"
                )
            _check_host_ref(
                c.get("host", default_host), declared, f"target '{tname}' {ch}"
            )

        serial = t.get("serial")
        if serial is not None:
            if _is_table(serial):
                raise LabError(
                    f"target '{tname}': serial must be [[serial]] array entries"
                )
            seen: set[str] = set()
            for s in serial:
                name = s.get("name")
                if not name or not s.get("device"):
                    raise LabError(
                        f"target '{tname}': each [[serial]] needs name + device"
                    )
                if name in seen:
                    raise LabError(f"target '{tname}': duplicate serial name '{name}'")
                seen.add(name)
                sense = s.get("power_sense_signal")
                if sense is not None and sense not in VALID_SENSE_SIGNALS:
                    raise LabError(
                        f"target '{tname}' serial '{name}': invalid "
                        f"power_sense_signal '{sense}' "
                        f"(valid: {', '.join(VALID_SENSE_SIGNALS)})"
                    )
                _check_host_ref(
                    s.get("host", default_host),
                    declared,
                    f"target '{tname}' serial '{name}'",
                )


class LabFile:
    """A lab file open for editing, backed by a live ``tomlkit`` document.

    Load with :meth:`load` (validates) or :meth:`create` (a fresh empty lab).
    Mutators change the in-memory document; :meth:`save` validates and writes it
    back, preserving all surrounding comments and formatting.
    """

    def __init__(self, path: str, doc: tomlkit.TOMLDocument):
        self.path = path
        self.doc = doc

    # ── lifecycle ──────────────────────────────────────────────────────────

    @classmethod
    def load(cls, path: str) -> "LabFile":
        full = os.path.expanduser(path)
        with open(full, encoding="utf-8") as f:
            doc = tomlkit.parse(f.read())
        validate(doc)
        return cls(path, doc)

    @classmethod
    def create(cls, path: str) -> "LabFile":
        """A new empty lab document (not yet written; call :meth:`save`)."""
        doc = tomlkit.document()
        doc.add(tomlkit.comment(_HEADER))
        return cls(path, doc)

    def save(self) -> None:
        validate(self.doc)
        full = Path(os.path.expanduser(self.path))
        full.parent.mkdir(parents=True, exist_ok=True)
        full.write_text(tomlkit.dumps(self.doc), encoding="utf-8")

    # ── internal helpers ───────────────────────────────────────────────────

    def _super(self, key: str):
        if key not in self.doc:
            self.doc[key] = tomlkit.table(is_super_table=True)
        return self.doc[key]

    def _target(self, name: str):
        targets = self.doc.get("targets") or {}
        if name not in targets:
            raise LabError(f"no target '{name}'")
        return targets[name]

    def _host_references(self, host: str) -> list[str]:
        """Targets/channels that bind to ``host`` (blocks host removal)."""
        refs: list[str] = []
        for tname, t in (self.doc.get("targets") or {}).items():
            default_host = t.get("host", LOCAL)
            if default_host == host:
                refs.append(tname)
                continue
            bound = [t.get(ch, {}).get("host") for ch in SINGLETON_CHANNELS]
            bound += [s.get("host") for s in (t.get("serial") or [])]
            if host in bound:
                refs.append(tname)
        return refs

    # ── hosts ──────────────────────────────────────────────────────────────

    def add_host(
        self,
        name: str,
        ssh: str,
        *,
        identity: Optional[str] = None,
        control_path: Optional[str] = None,
        paniolo_cmd: Optional[str] = None,
    ) -> None:
        hosts = self._super("hosts")
        if name in hosts:
            raise LabError(f"host '{name}' already exists")
        t = tomlkit.table()
        t["ssh"] = ssh
        if identity is not None:
            t["identity"] = identity
        if control_path is not None:
            t["control_path"] = control_path
        if paniolo_cmd is not None:
            t["paniolo_cmd"] = paniolo_cmd
        hosts[name] = t

    def update_host(
        self,
        name: str,
        *,
        ssh: Optional[str] = None,
        identity: Optional[str] = None,
        control_path: Optional[str] = None,
        paniolo_cmd: Optional[str] = None,
    ) -> None:
        hosts = self.doc.get("hosts") or {}
        if name not in hosts:
            raise LabError(f"no host '{name}'")
        t = hosts[name]
        for key, val in (
            ("ssh", ssh),
            ("identity", identity),
            ("control_path", control_path),
            ("paniolo_cmd", paniolo_cmd),
        ):
            if val is not None:
                t[key] = val

    def remove_host(self, name: str) -> None:
        hosts = self.doc.get("hosts") or {}
        if name not in hosts:
            raise LabError(f"no host '{name}'")
        refs = self._host_references(name)
        if refs:
            raise LabError(f"host '{name}' is still used by: {', '.join(sorted(refs))}")
        del hosts[name]

    # ── targets ────────────────────────────────────────────────────────────

    def add_target(
        self, name: str, *, host: Optional[str] = None, note: Optional[str] = None
    ) -> None:
        targets = self._super("targets")
        if name in targets:
            raise LabError(f"target '{name}' already exists")
        t = tomlkit.table()
        if host is not None:
            t["host"] = host
        if note is not None:
            t["note"] = note
        targets[name] = t

    def update_target(
        self, name: str, *, host: Optional[str] = None, note: Optional[str] = None
    ) -> None:
        t = self._target(name)
        if host is not None:
            t["host"] = host
        if note is not None:
            t["note"] = note

    def remove_target(self, name: str) -> None:
        targets = self.doc.get("targets") or {}
        if name not in targets:
            raise LabError(f"no target '{name}'")
        del targets[name]

    # ── serial channels (collection) ───────────────────────────────────────

    def _find_serial(self, arr, name: str) -> Optional[int]:
        for i, s in enumerate(arr):
            if s.get("name") == name:
                return i
        return None

    def add_serial(
        self,
        target: str,
        name: str,
        device: str,
        *,
        baud: int = 115200,
        power_sense_signal: Optional[str] = None,
        host: Optional[str] = None,
    ) -> None:
        t = self._target(target)
        arr = t.get("serial")
        if arr is None:
            arr = tomlkit.aot()
            t["serial"] = arr
        if self._find_serial(arr, name) is not None:
            raise LabError(f"target '{target}': serial '{name}' already exists")
        s = tomlkit.table()
        s["name"] = name
        s["device"] = device
        s["baud"] = baud
        if power_sense_signal is not None:
            s["power_sense_signal"] = power_sense_signal
        if host is not None:
            s["host"] = host
        arr.append(s)

    def update_serial(
        self,
        target: str,
        name: str,
        *,
        device: Optional[str] = None,
        baud: Optional[int] = None,
        power_sense_signal: Optional[str] = None,
        host: Optional[str] = None,
    ) -> None:
        t = self._target(target)
        arr = t.get("serial") or []
        idx = self._find_serial(arr, name)
        if idx is None:
            raise LabError(f"target '{target}': no serial '{name}'")
        s = arr[idx]
        for key, val in (
            ("device", device),
            ("baud", baud),
            ("power_sense_signal", power_sense_signal),
            ("host", host),
        ):
            if val is not None:
                s[key] = val

    def remove_serial(self, target: str, name: str) -> None:
        t = self._target(target)
        arr = t.get("serial") or []
        idx = self._find_serial(arr, name)
        if idx is None:
            raise LabError(f"target '{target}': no serial '{name}'")
        del arr[idx]
        if len(arr) == 0:
            del t["serial"]

    # ── singleton channels (netboot / power / video) ───────────────────────

    def _set_singleton(self, target: str, kind: str, fields: dict) -> None:
        t = self._target(target)
        c = t.get(kind)
        if c is None:
            c = tomlkit.table()
            t[kind] = c
        for key, val in fields.items():
            if val is not None:
                c[key] = val

    def _remove_singleton(self, target: str, kind: str) -> None:
        t = self._target(target)
        if kind not in t:
            raise LabError(f"target '{target}': no {kind} channel")
        del t[kind]

    def set_netboot(
        self,
        target: str,
        *,
        interface: Optional[str] = None,
        host_ip: Optional[str] = None,
        tftp_root: Optional[str] = None,
        host: Optional[str] = None,
    ) -> None:
        self._set_singleton(
            target,
            "netboot",
            {
                "interface": interface,
                "host_ip": host_ip,
                "tftp_root": tftp_root,
                "host": host,
            },
        )

    def remove_netboot(self, target: str) -> None:
        self._remove_singleton(target, "netboot")

    def set_power(
        self,
        target: str,
        *,
        cycle_cmd: Optional[str] = None,
        serial_interface: Optional[str] = None,
        host: Optional[str] = None,
    ) -> None:
        self._set_singleton(
            target,
            "power",
            {
                "cycle_cmd": cycle_cmd,
                "serial_interface": serial_interface,
                "host": host,
            },
        )

    def remove_power(self, target: str) -> None:
        self._remove_singleton(target, "power")

    def set_video(
        self,
        target: str,
        *,
        device: Optional[str] = None,
        host: Optional[str] = None,
    ) -> None:
        self._set_singleton(target, "video", {"device": device, "host": host})

    def remove_video(self, target: str) -> None:
        self._remove_singleton(target, "video")
