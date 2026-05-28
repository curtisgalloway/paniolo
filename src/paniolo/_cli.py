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

from __future__ import annotations

import os
import shutil
import subprocess
import time
from pathlib import Path
from typing import Annotated, Optional

import typer
from rich.console import Console
from rich.table import Table

from . import _config, _hid, _netboot, _ocr, _power, _serial, _state, _video

app = typer.Typer(help="Paniolo — agent-controlled target machine wrangler.", no_args_is_help=True)
target_app = typer.Typer(help="Manage target configurations.", no_args_is_help=True)
netboot_app = typer.Typer(help="Control DHCP+TFTP netboot for a target.", no_args_is_help=True)
video_app = typer.Typer(help="Capture screen frames via HDMI/USB capture device.", no_args_is_help=True)
serial_app = typer.Typer(help="Manage serial console connection to a target.", no_args_is_help=True)
hid_app = typer.Typer(help="Inject USB keyboard/mouse input via the HID rig.", no_args_is_help=True)
app.add_typer(target_app, name="target")
app.add_typer(netboot_app, name="netboot")
app.add_typer(video_app, name="video")
app.add_typer(serial_app, name="serial")
app.add_typer(hid_app, name="hid")

console = Console()
err = Console(stderr=True)


def _resolve(name: Optional[str]) -> _config.TargetConfig:
    if name is None:
        targets = _config.list_targets()
        if len(targets) == 1:
            name = targets[0]
        elif not targets:
            err.print("[red]No targets configured.[/red] Run: paniolo target set <name> --interface <iface>")
            raise typer.Exit(1)
        else:
            err.print(f"[red]Multiple targets ({', '.join(targets)}) — specify one.[/red]")
            raise typer.Exit(1)
    try:
        return _config.load_target(name)
    except FileNotFoundError:
        err.print(f"[red]Target '{name}' not found.[/red]")
        raise typer.Exit(1)


# ── target ────────────────────────────────────────────────────────────────────


@target_app.command("set")
def target_set(
    name: Annotated[str, typer.Argument(help="Target name (e.g. fortune)")],
    interface: Annotated[
        Optional[str],
        typer.Option("--interface", "-i", help="USB-Ethernet interface (e.g. en3); auto-detected if omitted"),
    ] = None,
    tftp_root: Annotated[Optional[str], typer.Option("--tftp-root", "-r", help="Path to TFTP files directory")] = None,
    host_ip: Annotated[str, typer.Option("--host-ip", help="Static IP to assign to the interface")] = "192.168.99.1",
    power_cycle_cmd: Annotated[
        Optional[str],
        typer.Option("--power-cycle-cmd", help="Shell command or script path to power-cycle the target"),
    ] = None,
    power_serial: Annotated[
        Optional[str],
        typer.Option(
            "--power-serial",
            help="Serial interface name used for DTR power cycling via J2 (e.g. console)",
        ),
    ] = None,
) -> None:
    """Create or update a target configuration.

    Serial consoles are managed separately with `paniolo serial setup` (a target
    can have several named interfaces), so they're preserved across updates here."""
    if interface is None:
        candidates = _netboot.list_usb_ethernet_interfaces()
        if not candidates:
            err.print("[red]No USB-Ethernet interfaces found.[/red] Specify one with --interface.")
            raise typer.Exit(1)
        active = [c for c in candidates if c["active"]]
        if len(active) == 1:
            interface = active[0]["device"]
            console.print(f"[dim]Auto-detected interface:[/dim] {interface} ({active[0]['port']})")
        elif len(candidates) == 1:
            interface = candidates[0]["device"]
            console.print(
                f"[dim]Auto-detected interface:[/dim] {interface} ({candidates[0]['port']}) "
                "[dim](no cable detected)[/dim]"
            )
        else:
            console.print("[yellow]Multiple USB-Ethernet interfaces found — use --interface to choose:[/yellow]")
            for c in candidates:
                status = "[green]active[/green]" if c["active"] else "[dim]inactive[/dim]"
                console.print(f"  {c['device']:6s}  {c['port']}  {status}")
            raise typer.Exit(1)

    try:
        existing = _config.load_target(name)
    except FileNotFoundError:
        existing = None

    cfg = _config.TargetConfig(
        name=name,
        interface=interface,
        host_ip=host_ip,
        tftp_root=tftp_root,
        power_cycle_cmd=(
            power_cycle_cmd if power_cycle_cmd is not None
            else (existing.power_cycle_cmd if existing else None)
        ),
        power_serial_interface=(
            power_serial if power_serial is not None
            else (existing.power_serial_interface if existing else None)
        ),
        serial_interfaces=existing.serial_interfaces if existing else [],
    )
    _config.save_target(cfg)
    console.print(f"[green]Target '[bold]{name}[/bold]' saved.[/green]")
    console.print(f"  interface   : {interface}")
    console.print(f"  host_ip     : {host_ip}")
    if tftp_root:
        console.print(f"  tftp_root   : {tftp_root}")
    if cfg.power_cycle_cmd:
        console.print(f"  power_cycle : {cfg.power_cycle_cmd}")
    if cfg.power_serial_interface:
        console.print(f"  power_serial: {cfg.power_serial_interface}")
    for iface in cfg.serial_interfaces:
        console.print(f"  serial      : {iface.name}: {iface.device} @ {iface.baud}")


@target_app.command("show")
def target_show(
    name: Annotated[Optional[str], typer.Argument()] = None,
) -> None:
    """Show target configuration(s)."""
    names = [name] if name else _config.list_targets()
    if not names:
        console.print("No targets configured.")
        return
    for tname in names:
        try:
            cfg = _config.load_target(tname)
        except FileNotFoundError:
            err.print(f"[red]Target '{tname}' not found.[/red]")
            continue
        t = Table(title=f"Target: {cfg.name}", show_header=False, box=None, padding=(0, 2))
        t.add_row("interface", cfg.interface)
        t.add_row("host_ip", cfg.host_ip)
        t.add_row("tftp_root", cfg.tftp_root or "[dim]not set[/dim]")
        t.add_row("power_cycle_cmd", cfg.power_cycle_cmd or "[dim]not set[/dim]")
        t.add_row("power_serial", cfg.power_serial_interface or "[dim]not set[/dim]")
        if cfg.serial_interfaces:
            for idx, iface in enumerate(cfg.serial_interfaces):
                t.add_row("serial" if idx == 0 else "", f"{iface.name}: {iface.device} @ {iface.baud}")
        else:
            t.add_row("serial", "[dim]not set[/dim]")
        console.print(t)


@target_app.command("clear")
def target_clear(
    name: Annotated[str, typer.Argument()],
) -> None:
    """Remove a target configuration."""
    path = _config.target_path(name)
    if not path.exists():
        err.print(f"[red]Target '{name}' not found.[/red]")
        raise typer.Exit(1)
    path.unlink()
    console.print(f"Target '[bold]{name}[/bold]' cleared.")


# ── netboot ───────────────────────────────────────────────────────────────────


@netboot_app.command("start")
def netboot_start(
    target: Annotated[Optional[str], typer.Argument()] = None,
) -> None:
    """Start DHCP + TFTP netboot for a target."""
    cfg = _resolve(target)
    try:
        _netboot.start(cfg)
    except RuntimeError as exc:
        err.print(f"[red]Error:[/red] {exc}")
        raise typer.Exit(1)

    time.sleep(0.5)
    s = _netboot.get_status(cfg.name)
    if s["running"]:
        console.print(f"[green]Netboot started[/green] for [bold]{cfg.name}[/bold]")
        console.print(f"  DHCP+TFTP  {cfg.interface}  ({cfg.host_ip}/24)")
        console.print(f"  tftp_root  {s['tftp_root']}")
        console.print(f"  log        {_state.netboot_log_path(cfg.name)}")
    else:
        err.print("[red]Failed to start — check log:[/red]")
        err.print(f"  {_state.netboot_log_path(cfg.name)}")
        raise typer.Exit(1)


@netboot_app.command("stop")
def netboot_stop(
    target: Annotated[Optional[str], typer.Argument()] = None,
) -> None:
    """Stop netboot for a target."""
    cfg = _resolve(target)
    if not _state.is_netboot_running(cfg.name):
        console.print(f"Netboot is not running for '{cfg.name}'.")
        return
    try:
        _netboot.stop(cfg.name)
        console.print("[green]Stopped.[/green]")
    except RuntimeError as exc:
        err.print(f"[red]Error:[/red] {exc}")
        raise typer.Exit(1)


@netboot_app.command("status")
def netboot_status(
    target: Annotated[Optional[str], typer.Argument()] = None,
) -> None:
    """Show netboot status for a target."""
    cfg = _resolve(target)
    s = _netboot.get_status(cfg.name)

    if not s["running"]:
        console.print(f"Netboot: [red]stopped[/red]  (target: {cfg.name})")
        return

    uptime = int(s.get("uptime_seconds") or 0)
    h, rem = divmod(uptime, 3600)
    m, sec = divmod(rem, 60)

    t = Table(show_header=False, box=None, padding=(0, 2))
    t.add_row("target", cfg.name)
    t.add_row("status", "[green]running[/green]")
    t.add_row("interface", s["interface"])
    t.add_row("dhcp", f"pid {s['dhcp_pid']}  {'[green]alive[/green]' if s['dhcp_alive'] else '[red]dead[/red]'}")
    t.add_row("tftp-now", f"pid {s['tftp_pid']}  {'[green]alive[/green]' if s['tftp_alive'] else '[red]dead[/red]'}")
    t.add_row("tftp_root", s["tftp_root"])
    t.add_row("uptime", f"{h:02d}:{m:02d}:{sec:02d}")
    console.print(t)


@netboot_app.command("tftp-root")
def netboot_tftp_root(
    target: Annotated[Optional[str], typer.Argument()] = None,
) -> None:
    """Print the TFTP root path (bare, for shell command substitution via SSH)."""
    cfg = _resolve(target)
    state = _state.load_netboot_state(cfg.name)
    if state:
        print(state.tftp_root)
    elif cfg.tftp_root:
        print(cfg.tftp_root)
    else:
        err.print("[red]No tftp_root configured for this target.[/red]")
        raise typer.Exit(1)


@netboot_app.command("logs")
def netboot_logs(
    target: Annotated[Optional[str], typer.Argument()] = None,
    follow: Annotated[bool, typer.Option("--follow", "-f")] = False,
) -> None:
    """Show netboot logs for a target."""
    cfg = _resolve(target)
    log_path = _state.netboot_log_path(cfg.name)
    if not log_path.exists():
        console.print("No log file yet.")
        return
    cmd = ["tail", "-f" if follow else "-100", str(log_path)]
    subprocess.run(cmd)


# ── video ─────────────────────────────────────────────────────────────────────


@video_app.command("setup")
def video_setup(
    device: Annotated[Optional[str], typer.Option("--device", help="Device name or substring (auto-detected if omitted)")] = None,
) -> None:
    """Discover and save the HDMI/USB capture device configuration."""
    if device is None:
        devices = _video.list_devices()
        if not devices:
            err.print("[red]No capture devices found.[/red] Is hdmicap installed?")
            raise typer.Exit(1)

        auto = _video.guess_capture_device(devices)
        if auto:
            device = auto["name"]
            console.print(f"[dim]Auto-detected capture device:[/dim] {device}")
        else:
            console.print("Available video devices:")
            for d in devices:
                console.print(f"  [{d['index']}] {d['name']}")
            choice = typer.prompt("Enter device name or index")
            if choice.isdigit():
                matches = [d for d in devices if d["index"] == int(choice)]
                if not matches:
                    err.print(f"[red]No device with index {choice}.[/red]")
                    raise typer.Exit(1)
                device = matches[0]["name"]
            else:
                device = choice

    cfg = _video.VideoConfig(device=device)
    _video.save_video_config(cfg)
    console.print(f"[green]Video device configured:[/green] {device}")


@video_app.command("watch")
def video_watch(
    port: Annotated[int, typer.Option("--port")] = 8723,
) -> None:
    """Start the hdmicap daemon in the background."""
    cfg = _video.load_video_config()
    if not cfg:
        err.print("[red]No video device configured.[/red] Run: paniolo video setup")
        raise typer.Exit(1)

    url = _video.daemon_url()
    if url:
        console.print(f"[dim]Daemon already running at[/dim] {url}")
        return

    binary = _video.hdmicap_binary()
    if not binary:
        err.print("[red]hdmicap not found.[/red] Build: cargo build --release in hdmicap/")
        raise typer.Exit(1)

    # Resolve (build if needed) the Vision OCR tool so the dashboard's OCR
    # button works; harmless if unavailable (the /ocr endpoint just 501s).
    ocr_bin = _ocr.visionocr_binary()
    _video.start_daemon(cfg, port, ocr_bin=ocr_bin)
    console.print("[dim]Starting daemon…[/dim]")

    url = None
    for _ in range(50):
        time.sleep(0.1)
        url = _video.daemon_url()
        if url:
            break

    if url:
        console.print(f"[green]Daemon started.[/green] Preview at {url}")
    else:
        err.print("[red]Daemon did not start within 5 s.[/red]")
        raise typer.Exit(1)


@video_app.command("preview")
def video_preview() -> None:
    """Open the live preview page in the default browser."""
    url = _video.daemon_url()
    if not url:
        err.print("[red]No daemon running.[/red] Start one with: paniolo video watch")
        raise typer.Exit(1)

    import webbrowser

    webbrowser.open(url)
    console.print(f"Opened {url}")


@video_app.command("shot")
def video_shot(
    stable: Annotated[bool, typer.Option("--stable")] = False,
    changed_since: Annotated[Optional[str], typer.Option("--changed-since")] = None,
    timeout: Annotated[int, typer.Option("--timeout")] = 2000,
    out: Annotated[str, typer.Option("--out", "-o")] = "-",
) -> None:
    """Fetch one PNG screenshot from the running daemon."""
    binary = _video.hdmicap_binary()
    if not binary:
        err.print("[red]hdmicap not found.[/red]")
        raise typer.Exit(1)

    cmd = [binary, "shot", "--timeout", str(timeout), "--out", out]
    if stable:
        cmd.append("--stable")
    if changed_since:
        cmd.extend(["--changed-since", changed_since])

    result = subprocess.run(cmd, check=False)
    raise typer.Exit(result.returncode)


@video_app.command("read")
def video_read(
    stable: Annotated[bool, typer.Option("--stable")] = False,
    fast: Annotated[bool, typer.Option("--fast", help="Lower-latency, less accurate recognition")] = False,
    as_json: Annotated[bool, typer.Option("--json", help="Emit text with bounding boxes")] = False,
    timeout: Annotated[int, typer.Option("--timeout")] = 2000,
) -> None:
    """OCR the current captured frame (Apple Vision) and print the text."""
    binary = _video.hdmicap_binary()
    if not binary:
        err.print("[red]hdmicap not found.[/red]")
        raise typer.Exit(1)
    if not _video.daemon_url():
        err.print("[red]No daemon running.[/red] Start one with: paniolo video watch")
        raise typer.Exit(1)

    shot_cmd = [binary, "shot", "--out", "-", "--timeout", str(timeout)]
    if stable:
        shot_cmd.append("--stable")
    shot = subprocess.run(shot_cmd, capture_output=True)
    if shot.returncode != 0:
        err.print(shot.stderr.decode(errors="replace").strip() or "snapshot failed")
        raise typer.Exit(1)

    try:
        text = _ocr.read_text(shot.stdout, fast=fast, as_json=as_json)
    except (FileNotFoundError, RuntimeError) as exc:
        err.print(f"[red]OCR failed:[/red] {exc}")
        raise typer.Exit(1)

    # Print raw — boot logs are full of [brackets] that rich would parse as markup.
    typer.echo(text, nl=False)


@video_app.command("devices")
def video_devices() -> None:
    """List available capture devices."""
    devices = _video.list_devices()
    if not devices:
        console.print("No capture devices found (or hdmicap not available).")
        return
    for d in devices:
        console.print(f"  [{d['index']}] {d['name']}  [{d.get('misc', '')}]")


@video_app.command("show")
def video_show() -> None:
    """Show the video capture configuration and daemon status."""
    cfg = _video.load_video_config()
    if not cfg:
        console.print("No video device configured. Run: paniolo video setup")
        return

    url = _video.daemon_url()
    t = Table(show_header=False, box=None, padding=(0, 2))
    t.add_row("device", cfg.device)
    t.add_row("daemon", f"[green]running[/green] at {url}" if url else "[dim]stopped[/dim]")
    console.print(t)


@video_app.command("stop")
def video_stop() -> None:
    """Stop the running hdmicap daemon."""
    binary = _video.hdmicap_binary()
    if not binary:
        err.print("[red]hdmicap not found.[/red]")
        raise typer.Exit(1)

    result = subprocess.run([binary, "stop"], check=False)
    if result.returncode == 0:
        console.print("[green]Daemon stopped.[/green]")
    else:
        raise typer.Exit(result.returncode)


# ── console ───────────────────────────────────────────────────────────────────


@app.command("console")
def open_dashboard(
    interface: Annotated[
        Optional[str],
        typer.Option("--interface", "-i", help="Serial interface name to preselect"),
    ] = None,
) -> None:
    """Open the combined video+serial dashboard in the default browser.

    Both daemons must be running:
      paniolo video watch
      paniolo serial watch [target]
    """
    video_url = _video.daemon_url()
    if not video_url:
        err.print("[red]No video daemon running.[/red] Start it with: paniolo video watch")
        raise typer.Exit(1)

    if not _serial.daemon_url():
        err.print("[red]No serial daemon running.[/red] Start it with: paniolo serial watch")
        raise typer.Exit(1)

    url = video_url if not interface else f"{video_url}?interface={interface}"

    import webbrowser

    webbrowser.open(url)
    console.print(f"Opened {url}")


# ── power-cycle ───────────────────────────────────────────────────────────────


def _dtr_button(
    cfg: "_config.TargetConfig", interface_name: Optional[str], duration_ms: int, label: str
) -> None:
    """Assert DTR for duration_ms ms via daemon or direct fallback. Exits on error."""
    try:
        iface = cfg.serial_interface(interface_name)
    except ValueError as exc:
        err.print(f"[red]{exc}.[/red] Run: paniolo serial setup")
        raise typer.Exit(1)

    daemon_url = _serial.daemon_url()
    if daemon_url:
        console.print(f"[dim]{label} ({duration_ms} ms via serialcap daemon)[/dim]")
        try:
            _power.dtr_button_press(daemon_url, iface.name, duration_ms)
        except OSError as exc:
            err.print(f"[red]Could not reach serialcap daemon:[/red] {exc}")
            raise typer.Exit(1)
        except RuntimeError as exc:
            err.print(f"[red]{label} failed:[/red] {exc}")
            raise typer.Exit(1)
    else:
        console.print(f"[dim]{label} ({duration_ms} ms via {iface.device} directly)[/dim]")
        try:
            _power.dtr_direct_button_press(iface.device, duration_ms)
        except (OSError, RuntimeError) as exc:
            err.print(f"[red]{label} failed:[/red] {exc}")
            raise typer.Exit(1)


@app.command("power-cycle")
def power_cycle(
    target: Annotated[Optional[str], typer.Argument()] = None,
) -> None:
    """Run the power-cycle script configured for this target.

    Requires power_cycle_cmd to be set. Configure with:
      paniolo target set <name> --power-cycle-cmd /path/to/script"""
    cfg = _resolve(target)
    if not cfg.power_cycle_cmd:
        err.print(
            f"[red]No power_cycle_cmd configured for '{cfg.name}'.[/red] "
            "Set one with: paniolo target set <name> --power-cycle-cmd /path/to/script"
        )
        raise typer.Exit(1)

    console.print(
        f"[dim]Power cycling[/dim] [bold]{cfg.name}[/bold] "
        f"[dim]via {cfg.power_cycle_cmd}[/dim]"
    )
    result = subprocess.run(cfg.power_cycle_cmd, shell=True, check=False)
    if result.returncode == 0:
        console.print("[green]Power cycle complete.[/green]")
    else:
        err.print(f"[red]Power cycle script exited with code {result.returncode}.[/red]")
        raise typer.Exit(result.returncode)


@app.command("power-state")
def power_state(
    target: Annotated[Optional[str], typer.Argument()] = None,
) -> None:
    """Show whether the target is powered on (requires sense signal wired and daemon running)."""
    cfg = _resolve(target)
    if not cfg.power_serial_interface:
        err.print(
            f"[red]No power serial interface configured for '{cfg.name}'.[/red] "
            "Set one with: paniolo target set <name> --power-serial <interface-name>"
        )
        raise typer.Exit(1)

    daemon_url = _serial.daemon_url()
    if not daemon_url:
        err.print("[red]serialcap daemon is not running.[/red] Start it with: paniolo serial watch")
        raise typer.Exit(1)

    state = _serial.read_power_state(daemon_url, cfg.power_serial_interface)
    if state is None:
        err.print(
            "[yellow]Power state unknown[/yellow] — sense signal may not be configured "
            "on this interface. Run: paniolo serial setup --power-sense <cts|dsr|dcd|ri>"
        )
        raise typer.Exit(1)
    if state:
        console.print(f"[green]Power ON[/green]  ({cfg.name})")
    else:
        console.print(f"[red]Power OFF[/red]  ({cfg.name})")


# ── serial ────────────────────────────────────────────────────────────────────


@serial_app.command("setup")
def serial_setup(
    target: Annotated[Optional[str], typer.Argument()] = None,
    name: Annotated[str, typer.Option("--name", help="Interface name (e.g. console, bmc)")] = _config.DEFAULT_SERIAL_NAME,
    device: Annotated[Optional[str], typer.Option("--device", help="Serial device path; auto-detected if omitted")] = None,
    baud: Annotated[int, typer.Option("--baud", help="Baud rate")] = 115200,
    power_sense: Annotated[
        Optional[str],
        typer.Option(
            "--power-sense",
            help=(
                "FTDI modem-control input wired to the target 3.3 V rail "
                "(cts | dsr | dcd | ri | none). "
                "Enables power-state sensing in GET /status and smart power-cycle waits."
            ),
        ),
    ] = None,
) -> None:
    """Add or update a named serial interface for a target.

    A target may have several (run setup once per interface, e.g. --name console,
    --name bmc). Re-running with an existing name updates that interface.

    Use --power-sense to specify which FTDI input pin is wired to the target's
    3.3 V rail for power-state detection (see hardware notes in AGENTS.md)."""
    cfg = _resolve(target)

    if device is None:
        devices = _serial.list_serial_devices()
        if not devices:
            err.print("[red]No serial devices found.[/red] Specify one with --device.")
            raise typer.Exit(1)
        if len(devices) == 1:
            device = devices[0]
            console.print(f"[dim]Auto-detected serial device:[/dim] {device}")
        else:
            console.print("Available serial devices:")
            for d in devices:
                console.print(f"  {d}")
            err.print("[red]Multiple devices found — specify one with --device.[/red]")
            raise typer.Exit(1)

    if power_sense is not None and power_sense.lower() == "none":
        power_sense = None
    elif power_sense is not None and power_sense.lower() not in _config.VALID_SENSE_SIGNALS:
        err.print(
            f"[red]Unknown sense signal '{power_sense}'.[/red] "
            f"Valid values: {', '.join(_config.VALID_SENSE_SIGNALS)}, none"
        )
        raise typer.Exit(1)
    elif power_sense is not None:
        power_sense = power_sense.lower()

    # Preserve existing sense signal when --power-sense is not given.
    if power_sense is None:
        existing_iface = next((i for i in cfg.serial_interfaces if i.name == name), None)
        sense = existing_iface.power_sense_signal if existing_iface else None
    else:
        sense = power_sense

    cfg.upsert_serial_interface(
        _config.SerialInterface(name=name, device=device, baud=baud, power_sense_signal=sense)
    )
    _config.save_target(cfg)
    sense_label = f"  power_sense : {sense}" if sense else ""
    console.print(
        f"[green]Serial interface '[bold]{name}[/bold]' saved for "
        f"'[bold]{cfg.name}[/bold]':[/green] {device} @ {baud}"
    )
    if sense_label:
        console.print(sense_label)


@serial_app.command("remove")
def serial_remove(
    name: Annotated[str, typer.Argument(help="Interface name to remove")],
    target: Annotated[Optional[str], typer.Option("--target", "-t")] = None,
) -> None:
    """Remove a named serial interface from a target."""
    cfg = _resolve(target)
    if cfg.remove_serial_interface(name):
        _config.save_target(cfg)
        console.print(f"[green]Removed serial interface '[bold]{name}[/bold]'.[/green]")
    else:
        have = ", ".join(i.name for i in cfg.serial_interfaces) or "none"
        err.print(f"[red]No serial interface '{name}'.[/red] (have: {have})")
        raise typer.Exit(1)


def _resolve_interface(cfg: "_config.TargetConfig", interface: Optional[str]) -> "_config.SerialInterface":
    try:
        return cfg.serial_interface(interface)
    except ValueError as exc:
        err.print(f"[red]{exc}.[/red] Run: paniolo serial setup")
        raise typer.Exit(1)


@serial_app.command("dtr")
def serial_dtr(
    target: Annotated[Optional[str], typer.Argument()] = None,
    ms: Annotated[int, typer.Option("--ms", help="Duration of the DTR pulse in milliseconds")] = 200,
    interface: Annotated[
        Optional[str],
        typer.Option("--interface", "-i", help="Serial interface name (default: power_serial_interface or the only one)"),
    ] = None,
) -> None:
    """Pulse the DTR line (J2 power button header) on a serial interface.

    Short pulse (≤500 ms) delivers a power-button event to the OS.
    Long pulse (≥3000 ms) triggers a hard PMIC power-off.

    With --interface/-i, any configured serial interface can be targeted.
    Without it, defaults to the target's power_serial_interface (if set),
    then falls back to the only configured interface."""
    cfg = _resolve(target)
    iface_name = interface or cfg.power_serial_interface
    _dtr_button(cfg, iface_name, ms, f"DTR pulse on {cfg.name}")
    console.print("[green]Done.[/green]")


@serial_app.command("reset")
def serial_reset(
    target: Annotated[Optional[str], typer.Argument()] = None,
    ms: Annotated[int, typer.Option("--ms", help="Press duration in milliseconds")] = 200,
    interface: Annotated[
        Optional[str],
        typer.Option("--interface", "-i", help="Serial interface name (default: power_serial_interface or the only one)"),
    ] = None,
) -> None:
    """Send a soft-reset signal via a brief J2 power button press.

    The OS receives a power-button event and responds according to its policy
    (typically a graceful reboot or halt)."""
    cfg = _resolve(target)
    iface_name = interface or cfg.power_serial_interface
    console.print(f"[dim]Soft reset[/dim] [bold]{cfg.name}[/bold]")
    _dtr_button(cfg, iface_name, ms, f"Soft reset on {cfg.name}")
    console.print("[green]Reset signal sent.[/green]")


@serial_app.command("connect")
def serial_connect(
    target: Annotated[Optional[str], typer.Argument()] = None,
    interface: Annotated[Optional[str], typer.Option("--interface", "-i", help="Interface name (default: the only one)")] = None,
) -> None:
    """Open an interactive serial console to a target (via tio)."""
    cfg = _resolve(target)
    iface = _resolve_interface(cfg, interface)
    if not _serial.tio_binary():
        err.print("[red]tio not found in PATH.[/red] Install it (e.g. brew install tio).")
        raise typer.Exit(1)
    cmd = _serial.connect_cmd(iface.device, iface.baud)
    os.execvp(cmd[0], cmd)


@serial_app.command("watch")
def serial_watch(
    target: Annotated[Optional[str], typer.Argument()] = None,
    port: Annotated[int, typer.Option("--port")] = 8724,
) -> None:
    """Start the serialcap daemon (owning every configured interface) so serial
    appears on the video dashboard and is captured for `serial log`."""
    cfg = _resolve(target)
    if not cfg.serial_interfaces:
        err.print(
            f"[red]No serial interfaces configured for '{cfg.name}'.[/red] "
            "Run: paniolo serial setup"
        )
        raise typer.Exit(1)

    url = _serial.daemon_url()
    if url:
        console.print(f"[dim]Serial daemon already running at[/dim] {url}")
        return

    if not _serial.serialcap_binary():
        err.print("[red]serialcap not found.[/red] Build: cargo build --release in serialcap/")
        raise typer.Exit(1)

    _serial.start_daemon(cfg.serial_interfaces, port)
    names = ", ".join(i.name for i in cfg.serial_interfaces)
    console.print(f"[dim]Starting serial daemon for[/dim] {len(cfg.serial_interfaces)} interface(s): {names}…")

    url = None
    for _ in range(50):
        time.sleep(0.1)
        url = _serial.daemon_url()
        if url:
            break

    if url:
        console.print(f"[green]Serial daemon started.[/green] {url}")
        console.print("Open the dashboard with: [bold]paniolo console[/bold]")
    else:
        err.print("[red]Serial daemon did not start within 5 s.[/red]")
        raise typer.Exit(1)


@serial_app.command("stop")
def serial_stop() -> None:
    """Stop the running serialcap daemon."""
    binary = _serial.serialcap_binary()
    if not binary:
        err.print("[red]serialcap not found.[/red]")
        raise typer.Exit(1)

    result = subprocess.run([binary, "stop"], check=False)
    if result.returncode == 0:
        console.print("[green]Serial daemon stopped.[/green]")
    else:
        raise typer.Exit(result.returncode)


@serial_app.command("devices")
def serial_devices() -> None:
    """List available serial devices."""
    devices = _serial.list_serial_devices()
    if not devices:
        console.print("No serial devices found.")
        return
    for d in devices:
        console.print(f"  {d}")


@serial_app.command("log")
def serial_log(
    interface: Annotated[Optional[str], typer.Option("--interface", "-i", help="Interface name (default: the only captured one)")] = None,
    tail: Annotated[Optional[int], typer.Option("--tail", "-n", help="Show only the most recent N lines")] = None,
    from_seq: Annotated[Optional[int], typer.Option("--from", help="Lowest line sequence number (inclusive)")] = None,
    to_seq: Annotated[Optional[int], typer.Option("--to", help="Highest line sequence number (inclusive)")] = None,
    since: Annotated[Optional[int], typer.Option("--since", help="Only lines newer than this sequence number")] = None,
    raw: Annotated[bool, typer.Option("--raw", help="Keep raw bytes (ANSI/control) instead of cleaning")] = False,
    as_json: Annotated[bool, typer.Option("--json", help="Emit JSON Lines instead of formatted text")] = False,
    no_pending: Annotated[bool, typer.Option("--no-pending", help="Exclude the current unterminated line")] = False,
) -> None:
    """Print captured serial output, timestamped and addressable by line range.

    Thin passthrough to `serialcap log`, which reads the daemon's on-disk capture
    log directly — so this works whether or not the daemon is currently running.
    With multiple interfaces, pass --interface to choose one."""
    binary = _serial.serialcap_binary()
    if not binary:
        err.print("[red]serialcap not found.[/red] Build: cargo build --release in serialcap/")
        raise typer.Exit(1)
    cmd = _serial.log_cmd(
        binary,
        interface=interface,
        tail=tail,
        from_seq=from_seq,
        to_seq=to_seq,
        since=since,
        raw=raw,
        as_json=as_json,
        no_pending=no_pending,
    )
    result = subprocess.run(cmd, check=False)
    if result.returncode != 0:
        raise typer.Exit(result.returncode)


@serial_app.command("show")
def serial_show(
    target: Annotated[Optional[str], typer.Argument()] = None,
) -> None:
    """Show the serial interfaces configured for a target, and daemon status."""
    cfg = _resolve(target)
    if not cfg.serial_interfaces:
        console.print(f"No serial interfaces configured for '{cfg.name}'. Run: paniolo serial setup")
        return
    url = _serial.daemon_url()
    t = Table(show_header=False, box=None, padding=(0, 2))
    for iface in cfg.serial_interfaces:
        label = f"{iface.device} @ {iface.baud}"
        if iface.power_sense_signal:
            label += f"  [dim](sense: {iface.power_sense_signal})[/dim]"
        t.add_row(iface.name, label)
    t.add_row("daemon", f"[green]running[/green] at {url}" if url else "[dim]stopped[/dim]")
    console.print(t)


# ── hid ───────────────────────────────────────────────────────────────────────


def _open_rig() -> "_hid.HidRig":
    cfg = _hid.load_hid_config()
    if not cfg:
        err.print("[red]No HID control board configured.[/red] Run: paniolo hid setup")
        raise typer.Exit(1)
    try:
        return _hid.HidRig(cfg.port)
    except (RuntimeError, OSError, ValueError) as exc:
        err.print(f"[red]Could not open {cfg.port}:[/red] {exc}")
        raise typer.Exit(1)


@hid_app.command("setup")
def hid_setup(
    port: Annotated[Optional[str], typer.Option("--port", help="Data CDC port; auto-suggested if omitted")] = None,
) -> None:
    """Detect and save the control board's data serial port."""
    if port is None:
        ports = _hid.list_serial_ports()
        if not ports:
            err.print("[red]No USB serial ports found.[/red] Is the control board plugged in?")
            raise typer.Exit(1)
        if len(ports) == 1:
            port = ports[0]
        else:
            console.print("Candidate ports (the data port is usually the higher-numbered):")
            for p in ports:
                console.print(f"  {p}")
            port = typer.prompt("Enter the data port", default=_hid.guess_data_port())
    _hid.save_hid_config(_hid.HidConfig(port=port))
    console.print(f"[green]HID control board configured:[/green] {port}")


@hid_app.command("type")
def hid_type(
    text: Annotated[list[str], typer.Argument(help="Text to type")],
) -> None:
    """Type a string."""
    rig = _open_rig()
    try:
        rig.type(" ".join(text))
    finally:
        rig.close()


@hid_app.command("key")
def hid_key(name: Annotated[str, typer.Argument(help="Keycode name, e.g. ENTER")]) -> None:
    """Tap a key (press + release)."""
    rig = _open_rig()
    try:
        rig.key(name)
    finally:
        rig.close()


@hid_app.command("releaseall")
def hid_releaseall() -> None:
    """Release all held keys and mouse buttons."""
    rig = _open_rig()
    try:
        rig.releaseall()
    finally:
        rig.close()


@hid_app.command("combo")
def hid_combo(
    names: Annotated[list[str], typer.Argument(help="Keycode names, e.g. LEFT_CONTROL C")],
) -> None:
    """Chord: press all keys, then release all."""
    rig = _open_rig()
    try:
        rig.combo(*names)
    finally:
        rig.close()


@hid_app.command("click")
def hid_click(
    button: Annotated[str, typer.Argument(help="left | right | middle")] = "left",
) -> None:
    """Click a mouse button."""
    rig = _open_rig()
    try:
        rig.click(button)
    finally:
        rig.close()


@hid_app.command("move", context_settings={"ignore_unknown_options": True})
def hid_move(
    dx: Annotated[str, typer.Argument()],
    dy: Annotated[str, typer.Argument()],
) -> None:
    """Relative mouse move (auto-split into HID steps on the board)."""
    rig = _open_rig()
    try:
        rig.move(int(dx), int(dy))
    finally:
        rig.close()


@hid_app.command("scroll", context_settings={"ignore_unknown_options": True})
def hid_scroll(amount: Annotated[str, typer.Argument()]) -> None:
    """Scroll the wheel (positive = up, negative = down)."""
    rig = _open_rig()
    try:
        rig.scroll(int(amount))
    finally:
        rig.close()


@hid_app.command("run")
def hid_run(
    file: Annotated[Path, typer.Argument(help="Command file (one per line; # comments; delay/sleep directives)")],
    delay: Annotated[int, typer.Option("--delay", help="Default ms between commands")] = 0,
) -> None:
    """Run a sequence of commands from a file, with optional timing."""
    if not file.exists():
        err.print(f"[red]File not found:[/red] {file}")
        raise typer.Exit(1)
    steps = _hid.parse_sequence(file.read_text())
    rig = _open_rig()
    try:
        _hid.run_sequence(rig, steps, default_delay=delay / 1000.0)
    finally:
        rig.close()
    console.print(f"[green]Ran {len(steps)} step(s).[/green]")


@hid_app.command("show")
def hid_show() -> None:
    """Show the HID control board configuration."""
    cfg = _hid.load_hid_config()
    if not cfg:
        console.print("No HID control board configured. Run: paniolo hid setup")
        return
    present = Path(cfg.port).exists()
    t = Table(show_header=False, box=None, padding=(0, 2))
    t.add_row("port", cfg.port)
    t.add_row("device", "[green]present[/green]" if present else "[yellow]not found[/yellow]")
    console.print(t)


# ── setup ─────────────────────────────────────────────────────────────────────


@app.command()
def setup() -> None:
    """Install system tools (tftp-now) and build/install paniolo's binaries.

    Builds hdmicap and serialcap (cargo install) and visionocr (swiftc) into
    ~/.cargo/bin so the daemons resolve from a stable installed path, not the
    in-repo build tree.
    """
    repo = Path(__file__).parent.parent.parent
    cargo_bin = Path.home() / ".cargo" / "bin"

    # 1. System tool: tftp-now via Homebrew.
    if not shutil.which("brew"):
        err.print("[red]Homebrew not found.[/red] Install it: https://brew.sh")
        raise typer.Exit(1)
    tftp = shutil.which("tftp-now") or next(
        (str(p) for d in _netboot._BREW_PATHS if (p := Path(d) / "tftp-now").exists()),
        None,
    )
    if tftp:
        console.print(f"  [green]✓[/green] tftp-now     {tftp}")
    else:
        console.print("  [dim]…[/dim] installing tftp-now via brew")
        try:
            subprocess.run(["brew", "install", "tftp-now"], check=True)
        except subprocess.CalledProcessError:
            err.print(
                "[yellow]tftp-now not in default tap.[/yellow] "
                "Try: brew tap curl/curl && brew install tftp-now"
            )
            raise typer.Exit(1)

    # 2. Rust daemons: cargo install into ~/.cargo/bin.
    cargo = shutil.which("cargo")
    if not cargo:
        err.print(
            "  [yellow]✗[/yellow] cargo not found — install Rust (https://rustup.rs) "
            "to build hdmicap/serialcap"
        )
    else:
        for crate in ("hdmicap", "serialcap"):
            crate_dir = repo / crate
            if not (crate_dir / "Cargo.toml").exists():
                console.print(f"  [yellow]…[/yellow] {crate}: source not found at {crate_dir}, skipping")
                continue
            console.print(f"  [dim]building {crate} (cargo install — may take a few minutes)…[/dim]")
            try:
                subprocess.run([cargo, "install", "--path", str(crate_dir), "--force"], check=True)
                console.print(f"  [green]✓[/green] {crate:12s} {cargo_bin / crate}")
            except subprocess.CalledProcessError:
                err.print(f"  [red]✗[/red] {crate}: cargo install failed")
                raise typer.Exit(1)

    # 3. visionocr (Apple Vision OCR) via swiftc.
    try:
        dest = cargo_bin / "visionocr"
        _ocr.build_visionocr(dest)
        console.print(f"  [green]✓[/green] visionocr    {dest}")
    except (FileNotFoundError, subprocess.CalledProcessError) as exc:
        console.print(f"  [yellow]…[/yellow] visionocr: skipped ({exc})")

    console.print("\n[green]Setup complete.[/green]")
    if cargo and str(cargo_bin) not in os.environ.get("PATH", "").split(os.pathsep):
        console.print(f"[yellow]Note:[/yellow] add {cargo_bin} to your PATH so the daemons resolve.")
