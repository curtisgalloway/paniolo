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

# Build and install the entire paniolo package: the Python CLI plus the native
# binaries (hdmicap, serialcap, netbootd) and the OCR helper. `make install`
# from a fresh clone is the only command you need; re-run it after editing
# anything to rebuild and reinstall.

CRATES = hdmicap serialcap netbootd
PANIOLO ?= paniolo

.PHONY: help install reinstall python rust native test fmt clean

help:
	@echo "paniolo build targets:"
	@echo "  make install    Build + install everything: Python CLI, Rust daemons, OCR helper"
	@echo "                   (+ tftp-now/setuid on macOS, group setup on Linux). Re-run anytime."
	@echo "  make reinstall   Alias for 'make install' (install is already a full rebuild)."
	@echo "  make python      Reinstall just the Python CLI (uv tool install --reinstall)."
	@echo "  make rust        Rebuild + install just the Rust crates ($(CRATES))."
	@echo "  make native      Run 'paniolo setup' only (Rust crates + OCR + macOS setuid)."
	@echo "  make test        Run the Python (pytest) and Rust (cargo test) suites."
	@echo "  make fmt         rustfmt every crate."
	@echo "  make clean       cargo clean every crate."

# Full build + install. uv installs/reinstalls the Python CLI, then `paniolo
# setup` (run from the freshly installed CLI) builds and installs the Rust
# daemons and OCR helper and applies the platform-specific steps — tftp-now and
# the setuid bpf-helper on macOS, dialout/video group membership on Linux.
install: python native

reinstall: install

python:
	uv tool install --reinstall .

# Native side via the CLI's own setup command, so all the platform logic
# (setuid, Homebrew tools, Linux groups, OCR helper) lives in one place.
native:
	$(PANIOLO) setup

# Fast path for iterating on the Rust daemons without re-running the full setup
# (skips OCR and the macOS setuid step — re-run `make native` if you need those).
rust:
	@for crate in $(CRATES); do \
		echo "==> cargo install --path $$crate"; \
		cargo install --path $$crate --force || exit 1; \
	done

test:
	uv run pytest -q
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
