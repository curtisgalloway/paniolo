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

"""zigplug — Zigbee smart plug control for paniolo power hooks.

Subcommands:
  form                 One-time: form the Zigbee network on the coordinator.
  permit [--time 60]   Open a pairing window and report plugs that join.
  list                 Table of joined plugs (IEEE, model, on/off).
  state <ieee>         Print exactly `on` or `off` (state_cmd contract).
  on <ieee>            Switch a plug on (confirms by reading back).
  off <ieee>           Switch a plug off (confirms by reading back).
  cycle <ieee>         Power-cycle: off → delay → on → confirm.
  remove <ieee>        Unpair a plug from the network.
"""

# Single quotes nested in double-quoted f-strings are required on Python 3.11.
# pylint: disable=inconsistent-quotes

from __future__ import annotations

import asyncio
import logging
from dataclasses import dataclass, field
from pathlib import Path
from typing import Annotated, Optional

import typer
import zigpy.device

from . import _app

app = typer.Typer(
    help="Zigbee smart plug control via a CC2652 (ZNP) coordinator dongle.",
    no_args_is_help=True,
)


@dataclass
class _Options:
    device: str = ""
    db_path: Path = field(default_factory=lambda: _app.DEFAULT_DB)


_options = _Options()


@app.callback()
def main(
    device: Annotated[
        Optional[str],
        typer.Option(
            "--device",
            "-d",
            help="Coordinator UART device path (e.g. /dev/cu.usbserial-8310).",
        ),
    ] = None,
    db: Annotated[
        Optional[Path],
        typer.Option("--db", help="zigpy device database path."),
    ] = None,
    verbose: Annotated[
        int,
        typer.Option("--verbose", "-v", count=True, help="-v info, -vv debug."),
    ] = 0,
) -> None:
    """Store global options and configure logging."""
    if device is None:
        raise typer.BadParameter("required option '--device' (-d) was not provided")
    _options.device = device
    if db is not None:
        _options.db_path = db
    level = {0: logging.WARNING, 1: logging.INFO}.get(verbose, logging.DEBUG)
    logging.basicConfig(level=level, format="%(asctime)s %(name)s %(message)s")


def _run(coro):
    """Run an async command, mapping ZigplugError to a clean CLI failure."""
    try:
        return asyncio.run(coro)
    except _app.ZigplugError as exc:
        typer.echo(f"error: {exc}", err=True)
        raise typer.Exit(code=1) from exc


def _ieee_argument(value: str) -> str:
    """Eagerly validate the IEEE argument so typer reports bad input."""
    try:
        _app.parse_ieee(value)
    except _app.ZigplugError as exc:
        raise typer.BadParameter(str(exc)) from exc
    return value


_IeeeArg = Annotated[
    str,
    typer.Argument(
        callback=_ieee_argument,
        help="Plug IEEE address (from `zigplug list`).",
        metavar="IEEE",
    ),
]


@app.command()
def form(
    channel: Annotated[
        Optional[int],
        typer.Option(
            "--channel",
            min=11,
            max=26,
            help="Zigbee channel (11-26); default: pick by energy scan. "
            "Channels 25-26 avoid 2.4 GHz Wi-Fi.",
        ),
    ] = None,
) -> None:
    """Form the Zigbee network on the coordinator (one-time setup)."""

    async def go() -> None:
        try:
            async with _app.open_coordinator(_options.device, _options.db_path) as zapp:
                typer.echo(f"network already formed: {_app.network_summary(zapp)}")
                return
        except _app.ZigplugError as exc:
            if "no Zigbee network" not in str(exc):
                raise  # a real failure, not just "unformed"
        async with _app.open_coordinator(
            _options.device, _options.db_path, auto_form=True, channel=channel
        ) as zapp:
            typer.echo(f"network formed: {_app.network_summary(zapp)}")

    _run(go())


@app.command()
def permit(
    time_s: Annotated[
        int,
        typer.Option(
            "--time", min=10, max=254, help="Seconds to keep the join window open."
        ),
    ] = 60,
) -> None:
    """Open a pairing window; put plugs in pairing mode while it runs."""

    async def go() -> None:
        async with _app.open_coordinator(_options.device, _options.db_path) as zapp:
            joined: list[zigpy.device.Device] = []

            class Listener:
                """Print join progress as devices arrive and interview."""

                def device_joined(self, device: zigpy.device.Device) -> None:
                    typer.echo(f"joined: {device.ieee} (interviewing...)")

                def device_initialized(self, device: zigpy.device.Device) -> None:
                    joined.append(device)
                    typer.echo(
                        f"paired: {device.ieee}  "
                        f"{device.manufacturer or '?'} {device.model or '?'}"
                    )

            zapp.add_listener(Listener())
            await zapp.permit(time_s=time_s)
            typer.echo(
                f"pairing window open for {time_s}s — "
                "put the plug in pairing mode now (Ctrl-C to stop early)"
            )
            try:
                await asyncio.sleep(time_s)
            except asyncio.CancelledError:
                pass
            if not joined:
                typer.echo("no plugs paired", err=True)
                raise typer.Exit(code=1)
            typer.echo(f"{len(joined)} plug(s) paired")

    _run(go())


@app.command(name="list")
def list_cmd() -> None:
    """List joined plugs: IEEE, NWK, manufacturer, model, state."""

    async def go() -> None:
        async with _app.open_coordinator(_options.device, _options.db_path) as zapp:
            devices = _app.plug_devices(zapp)
            if not devices:
                typer.echo("no plugs joined — run `zigplug permit` to pair one")
                return
            typer.echo(
                f"{'IEEE':<25} {'NWK':<8} {'Manufacturer':<16} {'Model':<16} State"
            )
            typer.echo("-" * 75)
            for dev in devices:
                try:
                    cluster = _app.on_off_cluster(dev)
                    plug_state = "on" if await _app.read_on_off(cluster) else "off"
                except (_app.ZigplugError, asyncio.TimeoutError, OSError):
                    plug_state = "?"
                typer.echo(
                    f"{str(dev.ieee):<25} 0x{dev.nwk:04x}   "
                    f"{(dev.manufacturer or '?'):<16} {(dev.model or '?'):<16} "
                    f"{plug_state}"
                )

    _run(go())


async def _with_cluster(ieee_text: str, fn) -> None:
    """Open the coordinator, resolve the plug's OnOff cluster, run fn."""
    ieee = _app.parse_ieee(ieee_text)
    async with _app.open_coordinator(_options.device, _options.db_path) as zapp:
        device = _app.find_device(zapp, ieee)
        cluster = _app.on_off_cluster(device)
        await fn(cluster)


@app.command()
def state(ieee: _IeeeArg) -> None:
    """Print exactly `on` or `off` (paniolo state_cmd contract)."""

    async def go(cluster) -> None:
        typer.echo("on" if await _app.read_on_off(cluster) else "off")

    _run(_with_cluster(ieee, go))


@app.command()
def on(ieee: _IeeeArg) -> None:
    """Switch a plug on and confirm."""

    async def go(cluster) -> None:
        await _app.set_on_off(cluster, True)
        typer.echo(f"plug {ieee}: on")

    _run(_with_cluster(ieee, go))


@app.command()
def off(ieee: _IeeeArg) -> None:
    """Switch a plug off and confirm."""

    async def go(cluster) -> None:
        await _app.set_on_off(cluster, False)
        typer.echo(f"plug {ieee}: off")

    _run(_with_cluster(ieee, go))


@app.command()
def cycle(
    ieee: _IeeeArg,
    delay_ms: Annotated[
        int,
        typer.Option("--delay-ms", help="Milliseconds to hold the plug off."),
    ] = 3000,
) -> None:
    """Power-cycle a plug: off → delay → on → confirm."""

    async def go(cluster) -> None:
        await _app.set_on_off(cluster, False)
        await asyncio.sleep(delay_ms / 1000.0)
        await _app.set_on_off(cluster, True)
        typer.echo(f"plug {ieee}: cycled (held off {delay_ms} ms, now on)")

    _run(_with_cluster(ieee, go))


@app.command()
def remove(ieee: _IeeeArg) -> None:
    """Unpair a plug (send a ZDO leave and forget it)."""

    async def go() -> None:
        parsed = _app.parse_ieee(ieee)
        async with _app.open_coordinator(_options.device, _options.db_path) as zapp:
            _app.find_device(zapp, parsed)  # error early if unknown
            await zapp.remove(parsed)
            # The leave request is sent from a background task; give it a
            # moment to go out before the one-shot process exits.
            await asyncio.sleep(2)
            typer.echo(f"plug {parsed}: removed")

    _run(go())


if __name__ == "__main__":
    app()
