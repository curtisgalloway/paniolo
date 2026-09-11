# Release train profile: paniolo

Derived from commit b1b8654 on 2026-09-11. Executed by the `release-train`
skill (public-skills, `plugins/dev-tools/skills/release-train`); kept honest by
its `profile_check.py` against the `## Sources` table below. Read `AGENTS.md`
"Cutting a release" first: everything there still holds, this file only adds
what the train needs to build, install and run each channel before the tag.

The first dry run (2026-09-10) PASSed the homebrew, deb and source arms and
FAILed the windows arm (the zip shipped no skills; fixed on branch
`windows-zip-skills`), so every arm below except windows is verified once.

## Project

- cli: `paniolo`
- version source: the tag (`${GITHUB_REF_NAME#v}` in the release workflow); every `Cargo.toml` stays `0.1.0` and `paniolo --version` prints `0.1.0` on purpose
- tag format: `vX.Y.Z`; annotated; subject `vX.Y.Z: <lowercase one-line summary>`, optional body
- main branch: `main`; protected: PR required, required checks `cli`, `serialcap`, `netbootd`, `hdmicap`, `macos`; auto-merge off; tag ruleset "Protect release tags"
- release workflow: `.github/workflows/release.yml`
- ci workflow: `.github/workflows/ci.yml`; lint helpers `scripts/ci-actions-pinned.sh`, `scripts/ci-coverage-check.sh`
- bump rules: conventional prefixes where present; this repo mostly writes `<crate>: <summary>`, so apply AGENTS.md: minor for a new command, channel, helper, verb, wire-protocol or daemon-lifecycle change, or a fix that changes what a target sees; patch for small fixes and docs; on the line, take the minor
- releaser identity: tag with the author's GitHub noreply form already in the history (`git log --format=%ae | sort -u`), passed as `-c user.email` on the tag command, never edited into config
- helpers: `hdmicap serialcap netbootd cambrionix hidrig ch9329 shellyplug amt` (the `HELPERS` line in the release workflow and `CRATES` in `Makefile` must agree; `scripts/ci-coverage-check.sh` enforces it)
- bundled skills: `skills/paniolo`, `skills/kvm-puppeting`, `skills/control-host` (shipped to `share/paniolo/skills` in every channel)
- log dir: `logs/release-train/<run>/` (gitignored, persistent)

## Hosts

Roles only; reach recipes and addresses are in `RELEASE-TRAIN.local.md`
(gitignored, see "Never commit private infrastructure" in `AGENTS.md`).

| role | needed by | what it must have |
|---|---|---|
| local (macOS, Apple Silicon) | homebrew, source, archaeology | rustup toolchain with `aarch64-apple-darwin` and `x86_64-apple-darwin`, `swiftc`, `brew`, `uv`, `python3.12`, `gh` |
| linux-builder (aarch64 Linux VM) | deb | cargo, the apt build deps from `Makefile` check-deps, `dpkg-deb`, `gpg`, `apt-ftparchive`, `rsync`, passwordless sudo; `nfpm` is fetched per run |
| windows-bench (Windows 11, x86_64) | windows | cargo, PowerShell 7 as the ssh login shell, a checkout to sync into |

## Smoke contract

Run against the **installed** copy with `HOME` (or `USERPROFILE` and
`APPDATA`/`LOCALAPPDATA`) pointed at a fresh temp dir, from a working
directory outside the checkout so `paniolo skill` resolves the packaged copy
rather than the repo's `skills/`.

| id | check | pass condition |
|---|---|---|
| S1 | `paniolo --help` | exit 0; lists `target`, `serial`, `video`, `hid`, `doctor`, `daemons`, `skill`, `helper` |
| S2 | `paniolo skill` and `paniolo skill paniolo` | exit 0; lists `paniolo`, `kvm-puppeting`, `control-host`; the second prints a SKILL.md body |
| S3 | `paniolo helper` then `paniolo helper <h> -- --help` for each helper (the `--` matters: without it clap answers for the `helper` verb and the binary never runs) | each exit 0 (a `--help` that returns 0 proves the shared libraries loaded) |
| S4 | `paniolo --lab $T/lab.toml init`, `target add smoke`, `target list`, `config show`, `target rm smoke`, `target list` | exit 0 each; `smoke` listed before `rm`, absent after |
| S5 | `paniolo --lab $T/lab.toml doctor` on that hardware-free lab, then `paniolo daemons list` | exit 0 or a clean "nothing configured" exit; never a panic or traceback |
| S6 | the platform's OCR helper answers: `visionocr` (macOS) and `winocr.exe` (Windows) decode the 8x8 PNG the release workflow's macOS smoke step uses and print the v1 envelope; `linuxocr --help` and `rapidocr --help` on Linux | exit 0, `engine` field matches |
| S7 | `python3.12 evals/run.py --check` with the installed `paniolo` first on `PATH` | exit 0 (the drift guard proves the shipped `--help` surface matches every documented scenario); n/a on Windows |

## Channels

Only the channels this repo ships. There is no crates.io or PyPI publish; the
from-source path is the `source` arm. winget is a no-op until its token
exists and is not an arm.

### homebrew

- kind: homebrew
- artifact: `paniolo-X.Y.Z-macos-universal.tar.gz` plus `.sha256`
- workflow job: package-macos
- host: local
- build: in the worktree, `cargo build --release --target <triple>` for `cli` and every helper, both Apple targets, `CARGO_PROFILE_RELEASE_STRIP=symbols`; `swiftc -O -target <arch>-apple-macos12.0 ocr/visionocr.swift` per slice; `lipo` everything including `netbootd-bpf-helper`; stage the keg layout (`bin/paniolo`, `libexec/bin/*`, `share/paniolo/skills/*`); ad-hoc `codesign`; tarball and sidecar, exactly as the `package-macos` job does. Use a persistent `CARGO_TARGET_DIR` outside the worktree so trains are incremental
- install like a user: the tap's stable formula pours this tarball, so write a throwaway formula `paniolo-rt.rb` (`keg_only "release-train dry run"`, `url "file://<tarball>"`, the real `sha256`, `bin.install "bin/paniolo"`, `(libexec/"bin").install Dir["libexec/bin/*"]`, and skills to the literal `prefix/"share/paniolo/skills"` because the CLI hardcodes `share/paniolo` and `pkgshare` for `paniolo-rt` would be `share/paniolo-rt`); `HOMEBREW_DEVELOPER=1 brew install --formula ./paniolo-rt.rb` (Homebrew 6 refuses a bare `.rb` outside a tap otherwise). keg-only means nothing links into the developer's `bin`
- smoke: S1..S7 against `$(brew --prefix paniolo-rt)/bin/paniolo`
- cleanup: `brew uninstall paniolo-rt`, always
- caveats: the real tap formula (`curtisgalloway/homebrew-tap`) is re-pinned by the release workflow's `bump-tap` job, not exercised here; re-verify covers it

### deb

- kind: deb
- artifact: `paniolo_X.Y.Z_<arch>.deb` plus `.sha256`, via `packaging/nfpm.yaml`
- workflow job: package
- host: linux-builder
- build: rsync the worktree to a VM-local dir (never build on a shared mount; exclude `target`, `.venv`, `.git`); build `cli` and every helper `--release`; stage the gitignored staging tree under `packaging` (`stage/bin` with the CLI, `stage/libexec` with the helpers plus `ocr/linuxocr` and `ocr/rapidocr`) as the `package` job does; fetch `nfpm` at the workflow's `NFPM_VERSION` and verify its `checksums.txt`; `VERSION=X.Y.Z ARCH=<arch> nfpm package -f packaging/nfpm.yaml -p deb`
- install like a user: assemble a real repo with `packaging/scripts/build-apt-repo.sh <debs> <out> <fpr>` under a throwaway GPG key, install the public key under `/etc/apt/keyrings/paniolo-rt.asc`, write a deb822 `.sources` (`URIs: file:///<out>`, `Suites: stable`, `Components: main`, `Signed-By` that key), `apt-get update`, `apt-cache policy paniolo` shows X.Y.Z, `apt-get install paniolo=X.Y.Z`; record whether an older paniolo was present (upgrade path) or not (fresh path)
- smoke: S1..S7 against `/usr/bin/paniolo`; S3 also `/usr/libexec/paniolo/bin/linuxocr --help` and `rapidocr --help`; also `/usr/share/paniolo/skills/paniolo/SKILL.md` and `/usr/lib/tmpfiles.d/paniolo.conf` exist
- cleanup: `apt-get remove paniolo` unless it was installed before; remove the `.sources` file, the keyring, the throwaway key
- caveats: the builder is Ubuntu 24.04 (glibc 2.39), not the `debian:bookworm` (glibc 2.36) container CI builds in, so glibc-floor regressions are CI's to catch; an amd64 `.deb` is built only by CI

### windows

- kind: zip
- artifact: `paniolo-X.Y.Z-x86_64-pc-windows-msvc.zip` plus `.sha256`
- workflow job: package-windows
- host: windows-bench
- build: sync sources to the bench host the way `scripts/sync-brik.sh` does (`tar` over ssh, excluding `target` and `.git`); on the host mirror the `package-windows` job's "Build every crate" step (cli, every helper, `ocr/winocr`), stage `paniolo\paniolo.exe`, `paniolo\libexec\*.exe`, and `paniolo\share\paniolo\skills\<name>\SKILL.md`; `Compress-Archive`; write the sidecar as the job does. Anything beyond a one-liner goes in a `.ps1` copied over and run with `pwsh -NoProfile -File`; PowerShell quoting over ssh is not worth fighting
- install like a user: `Expand-Archive` into a fresh temp dir; a new `pwsh` with that dir's `paniolo\` prepended to `PATH` and `USERPROFILE`, `APPDATA`, `LOCALAPPDATA` pointed at a fresh temp dir
- smoke: S1..S6 (S7 n/a); S6 is `libexec\winocr.exe --json <tiny png>`
- cleanup: nothing to undo; the zip and install dir stay under the host's build-logs dir
- caveats: signing is skipped locally (no Azure vars) so the exes are unsigned; netboot and OCR-in-paniolo are known-unsupported on Windows per AGENTS.md "Platform support"; the bench host sleeps when idle, so the local file's liveness probe and wake step come first. Fallback when the host is unreachable: `gh workflow run release.yml --ref <release branch>`, download `packages-windows-x86_64`, inspect the layout, report PARTIAL (never dispatch in a dry run)

### source

- kind: source
- artifact: none (the README's from-source path: `cargo install --path cli` then `paniolo setup`; also what `brew install --HEAD paniolo` does)
- workflow job: none
- host: local
- build: `cargo install --path cli --root $S/.cargo` from the worktree with `HOME=$S`, `CARGO_HOME` left at the real one (registry cache), a persistent `CARGO_TARGET_DIR`, and **`CARGO_INSTALL_ROOT=$S/.cargo`**: `paniolo setup` reinstalls the CLI itself with `cargo install --path cli --force` and no `--root` (`cli/src/setup.rs`), which without that variable overwrites the developer's real CLI (it did, 2026-09-10)
- install like a user: `HOME=$S paniolo setup --rust-only` from the worktree root (helpers land in `$S/.local/libexec/paniolo/bin`); then by hand the two non-`--rust-only` steps that need no sudo, because `--rust-only` exists to skip the setuid BPF helper but also skips these: `swiftc -O -o $S/.local/libexec/paniolo/bin/visionocr ocr/visionocr.swift`, and copy `skills/<name>/SKILL.md` to `$S/.local/share/paniolo/skills/<name>/SKILL.md` (what `skills::install_bundled` does); then `UV_TOOL_DIR=$S/uv UV_TOOL_BIN_DIR=$S/.local/libexec/paniolo/bin uv tool install ./zigplug` and `zigplug --help`
- smoke: S1..S7 against `$S/.cargo/bin/paniolo` with `HOME=$S`
- cleanup: nothing outside `$S`
- caveats: `paniolo --version` prints `0.1.0` here by design; `--rust-only`'s own message names only OCR/setuid/zigplug as skipped, not skills (minor doc gap in `cli/src/setup.rs`)

## Archaeology

- issue source: `gh issue list --state closed --limit 200 --json number,title,closedAt,body`
- fix commits: `git log --format='%h%x09%s%x09%b'` filtered for `Fixes #N` / `Closes #N` trailers first; loose `#N` matching mis-maps prose mentions (#144, #168, #140/#147 did on 2026-09-10); a fix commit with no issue still counts
- test locations: Rust `#[cfg(test)]` modules beside the code and `<crate>/tests/*.rs`; pytest in `ocr/tests` and `zigplug/tests`; `evals/tests`
- how to run: `cargo test --manifest-path <crate>/Cargo.toml` (own `CARGO_TARGET_DIR`, do not share with the arms); Python the way `.github/workflows/ci.yml` does; `cargo fmt` and `cargo clippy --all-targets -- -D warnings` before committing; `uvx pyink` for Python
- prove-it-bites: `git checkout <fix>^ -- <fixed source files only>`, run, must fail; `git checkout HEAD -- <those files>`, must pass
- conventions: tests must execute the fixed path (spawn, parse, call), never assert on the text of source or scripts; American spelling; commit subject `test: pin #<n> <short bug>`
- state on 2026-09-10: all 27 closed issues already pinned in their fix commits; backlog is fix commits with no issue number

## Publish

- gate: `date`; the repo is public on GitHub, so the user-level business-hours push rule applies; ask before any push, tag, or PR
- steps: push the release branch; `gh pr create --base main` with only the archaeology commits; `gh pr checks --watch`; `gh pr merge --squash`; `git fetch origin && git checkout main && git pull --ff-only`; annotated tag on the merged head with the releaser identity above; `git push origin vX.Y.Z`; watch `release.yml` (`gh run watch`), then the dispatched `docs.yml` run (it rebuilds the apt pool from the newest 5 Releases; a red docs run means apt clients keep the previous version)
- re-verify github: `gh release view vX.Y.Z --json assets` lists 12 assets (2 `.deb`, 2 Linux `.tar.gz`, macOS `.tar.gz`, Windows `.zip`, each with `.sha256`); download all, `shasum -a 256 -c` each against its sidecar
- re-verify homebrew: the tap's `Formula/paniolo.rb` shows `version "X.Y.Z"` and the new sha256s; `brew update && brew fetch paniolo` succeeds; install the fetched tarball `paniolo-rt`-style and run S1..S3
- re-verify apt: `https://curtisgalloway.github.io/paniolo/apt/dists/stable/InRelease` is signed and its `Packages` lists X.Y.Z; on linux-builder configure the `.sources` from `README.md`, `apt-get update`, `apt-cache policy paniolo` shows X.Y.Z, `apt-get install paniolo`, run S1..S3
- re-verify windows: download the zip, verify the sidecar, expand and run S1..S3 on windows-bench if reachable, else inspect the layout and report PARTIAL
- re-verify source: `cargo install --git https://github.com/curtisgalloway/paniolo --tag vX.Y.Z paniolo` into a temp root, run S1
- a re-verify failure never rolls back the tag; open an issue and report `PUBLISHED, unverified on <channel>`

## Sources

`profile_check.py` recomputes each blob id; a CHANGED row means the section it
feeds needs re-reading before `--update` re-pins it.

| path | blob | feeds |
|---|---|---|
| `.github/workflows/release.yml` | 2269f9768b1d | Channels (every arm's build and staging), Publish |
| `.github/workflows/docs.yml` | 2d69ff629252 | Publish: re-verify apt |
| `packaging/nfpm.yaml` | b348d64432f4 | Channels: deb |
| `packaging/scripts/build-apt-repo.sh` | fb2205eeab8e | Channels: deb, install like a user |
| `Makefile` | 449aa1fe4b37 | Project: helpers; Channels: source |
| `cli/src/setup.rs` | 67a6d1e41824 | Channels: source |
| `cli/src/skills.rs` | 299ad01f27bc | Smoke contract S2; Channels: homebrew, windows |
| `cli/src/daemons.rs` | 2f8f0ec1f955 | Smoke contract S3 |
| `scripts/ci-coverage-check.sh` | 8d0d03ddf496 | Project: helpers |
| `scripts/sync-brik.sh` | b8b5a15775a9 | Channels: windows |
| `README.md` | 00ee1992abb7 | Channels: source; Publish: re-verify apt |
| `AGENTS.md` | fadf34fbea9c | Project: bump rules, tag format; Publish |
