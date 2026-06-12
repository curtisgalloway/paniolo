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

# Build and install paniolo: the Rust CLI (cli/) plus the daemons and helpers
# (hdmicap, serialcap, netbootd, cambrionix, hidrig, usbhub, shellyplug), the
# OCR helper, and the zigplug Zigbee helper (Python, installed by `paniolo
# setup` as a uv tool).
# Only the `paniolo` CLI lands on PATH (~/.cargo/bin); the helpers install
# into the private libexec dir (~/.local/libexec/paniolo/bin), run via
# `paniolo helper <name> ...` when needed directly.
# `make install` from a fresh clone is the only command you need; re-run it
# after editing anything to rebuild and reinstall.

CRATES = cli hdmicap serialcap netbootd cambrionix hidrig ch9329 usbhub shellyplug

# The installed CLI, by absolute path: immune to a stale `paniolo` shadowing
# ~/.cargo/bin earlier in PATH (e.g. the retired Python CLI's uv-tools shim).
PANIOLO ?= $(HOME)/.cargo/bin/paniolo

.PHONY: help install reinstall rust test fmt clean check-shadow check-deps

help:
	@echo "paniolo build targets:"
	@echo "  make install    Build + install everything: the paniolo CLI, the daemons,"
	@echo "                   and platform steps via 'paniolo setup' (OCR helper,"
	@echo "                   bpf-helper setuid on macOS, group setup on Linux)."
	@echo "  make reinstall   Alias for 'make install' (install is already a full rebuild)."
	@echo "  make rust        Fast path: build + install the Rust crates ($(CRATES)) only,"
	@echo "                   skipping the OCR/setuid/zigplug/group steps."
	@echo "  make test        cargo test every crate."
	@echo "  make fmt         rustfmt every crate."
	@echo "  make clean       cargo clean every crate."

# Full build + install. Bootstrap the CLI with cargo, then let `paniolo setup`
# do the rest — it rebuilds all crates and applies the platform-specific steps,
# so all that logic lives in one place (cli/src/setup.rs).
install: check-shadow check-deps
	cargo install --path cli
	$(PANIOLO) setup

reinstall: install

# Fail before the first cargo invocation when a Linux build prerequisite is
# missing, with an install hint — instead of a cryptic build error minutes in.
# pkg-config + libudev-dev: serial-port enumeration (libudev-sys).
# cmake + nasm: hdmicap's turbojpeg dep builds a vendored libjpeg-turbo
#   (Debian's system libturbojpeg is too old for the crate), and the crate's
#   require-simd default makes nasm mandatory on x86-64.
# libclang-dev: V4L2 bindgen (v4l2-sys-mit) in hdmicap.
check-deps:
	@if [ "$$(uname -s)" = "Linux" ]; then \
		fail=0; missing=""; \
		if ! command -v cargo >/dev/null 2>&1; then \
			echo "ERROR: cargo not found — install Rust (https://rustup.rs)"; \
			fail=1; \
		fi; \
		command -v pkg-config >/dev/null 2>&1 || missing="$$missing pkg-config"; \
		command -v cmake >/dev/null 2>&1 || missing="$$missing cmake"; \
		command -v nasm >/dev/null 2>&1 || missing="$$missing nasm"; \
		if command -v pkg-config >/dev/null 2>&1; then \
			pkg-config --exists libudev || missing="$$missing libudev-dev"; \
		elif [ ! -e /usr/include/libudev.h ]; then \
			missing="$$missing libudev-dev"; \
		fi; \
		set -- /usr/lib/llvm-*/lib/libclang.so; \
		[ -e "$$1" ] || missing="$$missing libclang-dev"; \
		if [ -n "$$missing" ]; then \
			echo "ERROR: missing system packages:$$missing"; \
			echo "       install with: sudo apt-get install$$missing"; \
			fail=1; \
		fi; \
		[ "$$fail" = "0" ] || exit 1; \
	fi

# Warn when a different `paniolo` shadows the installed one. The pre-Rust
# `make install` used to register the Python CLI as a uv tool; anyone who ran
# it has a ~/.local/bin/paniolo shim that silently wins over ~/.cargo/bin.
check-shadow:
	@found=$$(command -v paniolo 2>/dev/null); \
	if [ -n "$$found" ] && [ "$$found" != "$(PANIOLO)" ]; then \
		echo "WARNING: 'paniolo' in PATH is $$found, not $(PANIOLO)."; \
		echo "         If it is the retired Python CLI, remove it:"; \
		echo "             uv tool uninstall paniolo"; \
	fi

# Fast path for iterating on the Rust code without re-running the full setup
# (skips OCR and the macOS setuid step — re-run `make install` if you need
# those). Bootstraps the CLI first so install-layout changes in setup.rs take
# effect, then lets `paniolo setup --rust-only` place the helpers — the
# libexec-vs-PATH layout logic lives in one place (cli/src/setup.rs).
rust: check-deps
	cargo install --path cli
	$(PANIOLO) setup --rust-only

test:
	@for crate in $(CRATES); do \
		echo "==> cargo test ($$crate)"; \
		( cd $$crate && cargo test ) || exit 1; \
	done

fmt:
	@for crate in $(CRATES); do \
		( cd $$crate && cargo fmt ); \
	done

clean:
	@for crate in $(CRATES); do \
		( cd $$crate && cargo clean ); \
	done
