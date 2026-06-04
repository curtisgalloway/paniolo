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
# (hdmicap, serialcap, netbootd, cambrionix, hidrig) and the OCR helper.
# `make install` from a fresh clone is the only command you need; re-run it
# after editing anything to rebuild and reinstall.

CRATES = cli hdmicap serialcap netbootd cambrionix hidrig

# The installed CLI, by absolute path: immune to a stale `paniolo` shadowing
# ~/.cargo/bin earlier in PATH (e.g. the retired Python CLI's uv-tools shim).
PANIOLO ?= $(HOME)/.cargo/bin/paniolo

.PHONY: help install reinstall rust test fmt clean check-shadow

help:
	@echo "paniolo build targets:"
	@echo "  make install    Build + install everything: the paniolo CLI, the daemons,"
	@echo "                   and platform steps via 'paniolo setup' (OCR helper,"
	@echo "                   bpf-helper setuid on macOS, group setup on Linux)."
	@echo "  make reinstall   Alias for 'make install' (install is already a full rebuild)."
	@echo "  make rust        Fast path: cargo install the crates ($(CRATES)) only,"
	@echo "                   skipping the OCR/setuid/group steps."
	@echo "  make test        cargo test every crate."
	@echo "  make fmt         rustfmt every crate."
	@echo "  make clean       cargo clean every crate."

# Full build + install. Bootstrap the CLI with cargo, then let `paniolo setup`
# do the rest — it rebuilds all crates and applies the platform-specific steps,
# so all that logic lives in one place (cli/src/setup.rs).
install: check-shadow
	cargo install --path cli
	$(PANIOLO) setup

reinstall: install

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
# (skips OCR and the macOS setuid step — re-run `make install` if you need those).
rust:
	@for crate in $(CRATES); do \
		echo "==> cargo install --path $$crate"; \
		cargo install --path $$crate --force || exit 1; \
	done

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
