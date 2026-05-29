# Directory Structure

```
.github/
  workflows/
    ci.yml (103 lines)
docs/
  dashboard.md (71 lines)
  hid.md (105 lines)
  netboot.md (129 lines)
  power.md (145 lines)
  serial.md (157 lines)
  video.md (91 lines)
hdmicap/
  assets/
    index.html (285 lines)
    xterm-addon-fit.js (2 lines)
    xterm.css (209 lines)
    xterm.js (2 lines)
  src/
    capture_thread.rs (212 lines)
    capture.rs (359 lines)
    daemon.rs (143 lines)
    frame.rs (202 lines)
    main.rs (202 lines)
    server.rs (395 lines)
  vendor/
    nokhwa-bindings-macos/
      src/
        lib.rs (2463 lines)
      .cargo_vcs_info.json (6 lines)
      .cargo-ok (1 lines)
      .gitignore (86 lines)
      build.rs (24 lines)
      Cargo.toml (64 lines)
      README.md (6 lines)
  .gitignore (1 lines)
  Cargo.toml (62 lines)
hidrig/
  control/
    boot.py (29 lines)
    code.py (167 lines)
  host/
    hid_seize_reports.c (156 lines)
    Makefile (8 lines)
  HANDOFF.md (97 lines)
  README.md (139 lines)
  SETUP.md (142 lines)
ocr/
  .gitignore (1 lines)
  linuxocr (87 lines)
  visionocr.swift (138 lines)
serialcap/
  src/
    capture.rs (683 lines)
    daemon.rs (162 lines)
    main.rs (194 lines)
    serial_io.rs (435 lines)
    server.rs (229 lines)
  .gitignore (1 lines)
  Cargo.toml (45 lines)
skills/
  paniolo/
    SKILL.md (174 lines)
src/
  paniolo/
    __init__.py (13 lines)
    _cli.py (1388 lines)
    _config.py (185 lines)
    _dhcp.py (334 lines)
    _hid.py (236 lines)
    _netboot.py (415 lines)
    _ocr.py (110 lines)
    _power.py (73 lines)
    _serial.py (252 lines)
    _state.py (130 lines)
    _tftp.py (558 lines)
    _video.py (178 lines)
tests/
  test_config.py (139 lines)
  test_hid.py (176 lines)
  test_serial.py (73 lines)
.gitignore (10 lines)
AGENTS.md (603 lines)
LICENSE (201 lines)
pyproject.toml (32 lines)
README.md (117 lines)
```