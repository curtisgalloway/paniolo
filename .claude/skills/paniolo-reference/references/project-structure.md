# Directory Structure

```
.claude/
  scheduled_tasks.lock (1 lines)
.github/
  workflows/
    ci.yml (155 lines)
cambrionix/
  src/
    main.rs (216 lines)
    proto.rs (234 lines)
  .gitignore (1 lines)
  Cargo.toml (33 lines)
cli/
  src/
    daemons.rs (140 lines)
    discover.rs (257 lines)
    dispatch.rs (297 lines)
    doctor.rs (236 lines)
    labfile.rs (587 lines)
    main.rs (2341 lines)
    model.rs (551 lines)
    netboot.rs (163 lines)
    netif.rs (428 lines)
    power.rs (61 lines)
    serial.rs (184 lines)
    setup.rs (247 lines)
    ssh.rs (294 lines)
    state.rs (134 lines)
    video.rs (77 lines)
  .gitignore (1 lines)
  Cargo.toml (57 lines)
docs/
  ci-integration/
    design.md (276 lines)
    gap-analysis.md (228 lines)
    redfish-provider.md (147 lines)
    related-work.md (158 lines)
  architecture.md (267 lines)
  ch9329-spec.md (126 lines)
  config-redesign.md (325 lines)
  dashboard.md (74 lines)
  distributed-control-plan.md (209 lines)
  distributed-control.md (266 lines)
  hid-serial-protocol.md (157 lines)
  hid.md (127 lines)
  netboot.md (190 lines)
  netif.md (112 lines)
  power.md (227 lines)
  README.md (68 lines)
  requirements.md (259 lines)
  serial.md (226 lines)
  video.md (103 lines)
hdmicap/
  assets/
    index.html (285 lines)
    xterm-addon-fit.js (2 lines)
    xterm.css (209 lines)
    xterm.js (2 lines)
  src/
    capture_thread.rs (212 lines)
    capture.rs (515 lines)
    daemon.rs (163 lines)
    frame.rs (203 lines)
    main.rs (217 lines)
    server.rs (407 lines)
  vendor/
    nokhwa-bindings-macos/
      src/
        lib.rs (2463 lines)
      .cargo_vcs_info.json (6 lines)
      .cargo-ok (1 lines)
      .gitignore (86 lines)
      build.rs (24 lines)
      Cargo.toml (75 lines)
      README.md (6 lines)
  .gitignore (1 lines)
  Cargo.toml (62 lines)
hidrig/
  firmware/
    boot.py (48 lines)
    code.py (187 lines)
  host/
    hid_seize_reports.c (156 lines)
    Makefile (8 lines)
  src/
    main.rs (184 lines)
    proto.rs (171 lines)
  .gitignore (1 lines)
  Cargo.toml (33 lines)
  README.md (130 lines)
  SETUP.md (128 lines)
netbootd/
  src/
    bin/
      netbootd-bpf-helper.rs (73 lines)
    bpf.rs (130 lines)
    dhcp.rs (418 lines)
    frame.rs (182 lines)
    handoff.rs (259 lines)
    lib.rs (24 lines)
    main.rs (205 lines)
    netcfg.rs (194 lines)
    tftp.rs (919 lines)
  .gitignore (1 lines)
  Cargo.toml (58 lines)
ocr/
  .gitignore (1 lines)
  linuxocr (87 lines)
  visionocr.swift (138 lines)
serialcap/
  src/
    capture.rs (683 lines)
    daemon.rs (183 lines)
    main.rs (194 lines)
    serial_io.rs (614 lines)
    server.rs (270 lines)
  .gitignore (1 lines)
  Cargo.toml (45 lines)
skills/
  paniolo/
    SKILL.md (345 lines)
src/
  paniolo/
    __init__.py (13 lines)
    _cli.py (2600 lines)
    _config.py (208 lines)
    _dhcp.py (354 lines)
    _hid.py (236 lines)
    _lab.py (370 lines)
    _labfile.py (418 lines)
    _netboot.py (601 lines)
    _netif.py (295 lines)
    _ocr.py (134 lines)
    _paths.py (61 lines)
    _power.py (76 lines)
    _remote.py (144 lines)
    _serial.py (297 lines)
    _ssh.py (282 lines)
    _state.py (147 lines)
    _tftp.py (563 lines)
    _video.py (187 lines)
tests/
  test_cli.py (208 lines)
  test_config_cli.py (148 lines)
  test_config.py (164 lines)
  test_hid.py (181 lines)
  test_lab.py (249 lines)
  test_labfile.py (150 lines)
  test_netboot.py (158 lines)
  test_netif.py (161 lines)
  test_paths.py (95 lines)
  test_remote.py (202 lines)
  test_serial.py (115 lines)
  test_ssh.py (209 lines)
  test_state.py (203 lines)
  test_video.py (146 lines)
.gitignore (10 lines)
AGENTS.md (871 lines)
LICENSE (201 lines)
Makefile (82 lines)
pyproject.toml (36 lines)
README.md (148 lines)
```