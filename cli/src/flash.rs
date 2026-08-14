// Copyright 2026 Curtis Galloway
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! bao1x-uf2 flash protocol client (the `flash` channel's only method today).
//!
//! Streams UF2 blocks to the Baochip-1x `boot1` bootloader REPL over the
//! target's serial console, *through* the serialcap daemon's generic
//! send/expect endpoint — capture keeps running, so the device's `Wrote …`
//! acks land in the serial log. Vendor knowledge (the block protocol, retry
//! policy, ack validation) lives here in the CLI, like adb.rs; the daemon
//! stays vendor-free. Protocol reference: `bao1x-boot/uf2send.py` in
//! betrusted-io/xous-core, with two deliberate differences: any failed block
//! fails the transfer (uf2send exits 0 after 1–4 failed blocks), and protocol
//! errors are matched as alternations so they fail fast with a diagnosis
//! instead of eating the ack timeout.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Result};

use crate::serial;

// ── protocol constants (see docs/flash.md) ──────────────────────────────────

/// UF2 container magics (little-endian words at offsets 0, 4, 508).
const UF2_MAGIC_START0: u32 = 0x0A32_4655;
const UF2_MAGIC_START1: u32 = 0x9E5D_5157;
const UF2_MAGIC_END: u32 = 0x0AB1_6F30;
const UF2_BLOCK_SIZE: usize = 512;
/// A UF2 block's payload lives in a 476-byte field.
const UF2_MAX_PAYLOAD: u32 = 476;

/// How long to wait for a block's ack after the command is sent (uf2send: 0.5 s).
const ACK_TIMEOUT_MS: u64 = 500;
/// Attempts per block, total (uf2send's "3 retries" is 3 total attempts).
const BLOCK_ATTEMPTS: u32 = 3;
/// Abort the transfer once this many blocks have failed all their attempts.
const MAX_FAILED_BLOCKS: usize = 5;
/// Per-byte pacing for `localecho` commands: boot1's echo processing lags, so
/// they're dripped (uf2send writes them one char at a time with flush).
const LOCALECHO_PACE_MS: u32 = 5;
/// Settle after switching localecho (uf2send: 100 ms).
const SETTLE_MS: u64 = 100;

/// Everything a block exchange can answer. `Wrote …` is the happy path — but
/// it means "address accepted", not "write verified" (ReRAM write errors go
/// only to the debug UART), so post-flash verification is "does it boot".
/// The `\s` anchor keeps a partially-received hex address from matching early.
const ACK_PATTERN: &str = r"Wrote (\d+) to (0x[0-9a-fA-F]+)\s|Invalid write address|Corrupt base64|CRC error|Command not recognized";

// ── base64 (standard alphabet, padded — what boot1's decoder expects) ───────

const B64_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard padded base64. Hand-rolled to keep the CLI dependency-light; the
/// vectors in the tests below pin it to RFC 4648.
pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        let idx = |shift: u32| B64_ALPHABET[(n >> shift & 0x3f) as usize] as char;
        out.push(idx(18));
        out.push(idx(12));
        out.push(if chunk.len() > 1 { idx(6) } else { '=' });
        out.push(if chunk.len() > 2 { idx(0) } else { '=' });
    }
    out
}

// ── UF2 container ───────────────────────────────────────────────────────────

/// One 512-byte UF2 block, with the fields the ack is validated against.
#[derive(Debug)]
pub struct Uf2Block {
    pub raw: Vec<u8>,
    pub target_addr: u32,
    pub payload_size: u32,
}

fn le32(chunk: &[u8], off: usize) -> u32 {
    u32::from_le_bytes(chunk[off..off + 4].try_into().unwrap())
}

/// Validate a UF2 image (size multiple of 512, per-block magics, sane payload
/// size) and split it into blocks. Family/address range are the bootloader's
/// checks; the ack alternation surfaces its verdict per block.
pub fn parse_uf2(name: &str, bytes: &[u8]) -> Result<Vec<Uf2Block>> {
    if bytes.is_empty() {
        bail!("{name}: empty file");
    }
    if !bytes.len().is_multiple_of(UF2_BLOCK_SIZE) {
        bail!(
            "{name}: {} bytes is not a multiple of {UF2_BLOCK_SIZE} — not a UF2 image",
            bytes.len()
        );
    }
    let mut blocks = Vec::with_capacity(bytes.len() / UF2_BLOCK_SIZE);
    for (i, chunk) in bytes.chunks(UF2_BLOCK_SIZE).enumerate() {
        if le32(chunk, 0) != UF2_MAGIC_START0
            || le32(chunk, 4) != UF2_MAGIC_START1
            || le32(chunk, 508) != UF2_MAGIC_END
        {
            bail!("{name}: block {i} has bad UF2 magics — not a UF2 image");
        }
        let payload_size = le32(chunk, 16);
        if payload_size == 0 || payload_size > UF2_MAX_PAYLOAD {
            bail!("{name}: block {i} has implausible payload size {payload_size}");
        }
        blocks.push(Uf2Block {
            raw: chunk.to_vec(),
            target_addr: le32(chunk, 12),
            payload_size,
        });
    }
    Ok(blocks)
}

// ── REPL exchanges ──────────────────────────────────────────────────────────

/// A short token unique per call, for `echo` probes: queued writes can burst
/// out when the port reopens after a power cycle, so every attempt must be
/// distinguishable from stale ones.
fn nonce() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!(
        "p{:x}n{:x}s{:x}",
        std::process::id(),
        nanos,
        SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

/// Probe until the boot1 REPL answers an `echo` (any echo of the nonce proves
/// the REPL loop is pumping — never wait for boot banners: they print before
/// USB CDC is up and only reach the hardware UART).
pub fn probe_repl(url: &str, iface: &str, wait: Duration) -> Result<()> {
    let start = Instant::now();
    loop {
        let n = nonce();
        if let Ok(resp) =
            serial::send_expect(url, iface, Some(&format!("echo {n}\r")), &n, 1_000, 0)
        {
            if resp.matched {
                return Ok(());
            }
        }
        if start.elapsed() >= wait {
            bail!(
                "boot1 REPL did not answer within {} s — if the board booted its OS \
                 instead of stopping at the bootloader, enable boot-to-REPL once with \
                 `bootwait enable` on the boot1 console (see docs/flash.md)",
                wait.as_secs()
            );
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

/// Switch boot1's local echo. Sent twice, paced per byte, because the first
/// command after connecting is sometimes garbled and echo processing lags
/// (uf2send.py does the same); a settle delay follows.
fn set_localecho(url: &str, iface: &str, on: bool) -> Result<()> {
    let cmd = if on {
        "localecho on\r"
    } else {
        "localecho off\r"
    };
    for _ in 0..2 {
        serial::send_input(url, iface, cmd.as_bytes(), LOCALECHO_PACE_MS)?;
        std::thread::sleep(Duration::from_millis(SETTLE_MS));
    }
    Ok(())
}

/// Determine which `uf2` protocol variant this boot1 runs. The plain variant
/// takes one arg (the block); the `uf2-spim` variant (external-memory boards,
/// e.g. baosec) takes block + CRC-32 and needs `uf2_flush`. `has-crc` prints
/// `true` only on the spim variant; a variant mismatch must abort here with a
/// precise error, not surface as thousands of silent per-block retry timeouts.
fn check_uf2_variant(url: &str, iface: &str) -> Result<()> {
    let resp = serial::send_expect(
        url,
        iface,
        Some("has-crc\r"),
        r"\b(true|false)\b|not recognized",
        1_500,
        0,
    )?;
    if !resp.matched {
        bail!(
            "couldn't determine which uf2 protocol variant boot1 runs \
             (`has-crc` gave no answer); refusing to stream blocks blind"
        );
    }
    if resp.match_text.as_deref() == Some("true") {
        bail!(
            "this boot1 runs the uf2-spim CRC variant (2-arg `uf2` + `uf2_flush`), \
             which paniolo doesn't speak yet — flash with xous-core's uf2send.py \
             after `paniolo serial stop` frees the port"
        );
    }
    Ok(())
}

/// Ask boot1 to boot the OS. `boot` — NOT `reset`: with bootwait enabled a
/// plain chip reset lands back in the REPL (the warm-boot flag is OS-managed
/// and still clear during flashing).
pub fn send_boot(url: &str, iface: &str) -> Result<()> {
    serial::send_marker(url, iface, "flash: boot");
    serial::send_input(url, iface, b"boot\r", 0)
}

// ── the transfer ────────────────────────────────────────────────────────────

/// Is this ack the right one for `block`? The regex alone isn't enough: a late
/// ack for block N can land in block N+1's window, and matching the size and
/// address fields against the block is what makes that self-correcting (the
/// mismatched attempt fails, the retry re-syncs).
fn ack_matches(resp: &serial::ExpectResponse, block: &Uf2Block) -> bool {
    let size = resp
        .captures
        .first()
        .and_then(|c| c.as_deref())
        .and_then(|s| s.parse::<u32>().ok());
    let addr = resp
        .captures
        .get(1)
        .and_then(|c| c.as_deref())
        .and_then(|s| s.strip_prefix("0x"))
        .and_then(|s| u32::from_str_radix(s, 16).ok());
    size == Some(block.payload_size) && addr == Some(block.target_addr)
}

pub struct TransferReport {
    pub blocks: usize,
    pub retries: u64,
    pub elapsed: Duration,
}

/// Stream one parsed UF2 image to the REPL. Assumes the REPL is answering and
/// localecho is already off; returns per-file stats. Fails (with the failed
/// block numbers) if any block exhausts its attempts — never "mostly worked".
fn stream_file(url: &str, iface: &str, name: &str, blocks: &[Uf2Block]) -> Result<TransferReport> {
    let total = blocks.len();
    let started = Instant::now();
    let mut retries: u64 = 0;
    let mut failed: Vec<usize> = Vec::new();
    // ~10% progress steps: enough to see life without drowning the terminal.
    let step = (total / 10).max(1);

    for (i, block) in blocks.iter().enumerate() {
        let cmd = format!("uf2 {}\r", base64_encode(&block.raw));
        let mut wrote = false;
        let mut last_reply = String::new();
        for attempt in 1..=BLOCK_ATTEMPTS {
            if attempt > 1 {
                retries += 1;
            }
            let resp = serial::send_expect(url, iface, Some(&cmd), ACK_PATTERN, ACK_TIMEOUT_MS, 0)?;
            if resp.matched && ack_matches(&resp, block) {
                wrote = true;
                break;
            }
            last_reply = match (&resp.match_text, resp.matched) {
                (Some(m), true) => m.trim().to_string(),
                _ => format!("no ack within {ACK_TIMEOUT_MS} ms"),
            };
        }
        if !wrote {
            eprintln!("  block {i}/{total} failed {BLOCK_ATTEMPTS} attempts (last: {last_reply})");
            failed.push(i);
            if failed.len() >= MAX_FAILED_BLOCKS {
                bail!(
                    "{name}: aborting after {MAX_FAILED_BLOCKS} failed blocks \
                     (first failures: {:?})",
                    failed
                );
            }
        }
        if (i + 1) % step == 0 || i + 1 == total {
            eprintln!(
                "  {name}: {}/{total} blocks ({}%)",
                i + 1,
                (i + 1) * 100 / total
            );
        }
    }
    if !failed.is_empty() {
        bail!(
            "{name}: {} block(s) failed all {BLOCK_ATTEMPTS} attempts: {:?}",
            failed.len(),
            failed
        );
    }
    Ok(TransferReport {
        blocks: total,
        retries,
        elapsed: started.elapsed(),
    })
}

/// Flash one or more parsed UF2 images through the daemon-held port, in
/// argument order (loader → xous → apps when flashing all three). The REPL
/// must already be reachable (see [`probe_repl`] / `--cycle`). Local echo is
/// switched off for the transfer and restored afterwards even on failure.
pub fn flash_files(url: &str, iface: &str, files: &[(String, Vec<Uf2Block>)]) -> Result<()> {
    set_localecho(url, iface, false)?;
    let result = (|| -> Result<()> {
        // With echo off, a fresh echo probe proves command processing (not
        // just character echo) before the first 512-byte block goes out.
        let n = nonce();
        let probe = serial::send_expect(
            url,
            iface,
            Some(&format!("echo {n}\r")),
            &format!("{n} "),
            2_000,
            0,
        )?;
        if !probe.matched {
            bail!(
                "boot1 REPL stopped answering after localecho off (echo probe timed out); \
                 tail: {:?}",
                probe.tail.unwrap_or_default()
            );
        }
        check_uf2_variant(url, iface)?;

        for (name, blocks) in files {
            let kib = blocks.len() * UF2_BLOCK_SIZE / 1024;
            serial::send_marker(url, iface, &format!("flash {name} start ({kib} KiB)"));
            eprintln!("Flashing {name}: {} blocks ({kib} KiB)…", blocks.len());
            match stream_file(url, iface, name, blocks) {
                Ok(r) => {
                    serial::send_marker(
                        url,
                        iface,
                        &format!(
                            "flash {name} done ({} blocks, {} retries, {:.1}s)",
                            r.blocks,
                            r.retries,
                            r.elapsed.as_secs_f32()
                        ),
                    );
                    eprintln!(
                        "{name}: {} blocks in {:.1} s ({:.0} KiB/s), {} retries",
                        r.blocks,
                        r.elapsed.as_secs_f32(),
                        kib as f32 / r.elapsed.as_secs_f32().max(0.001),
                        r.retries
                    );
                }
                Err(e) => {
                    serial::send_marker(url, iface, &format!("flash {name} FAILED"));
                    return Err(e);
                }
            }
        }
        Ok(())
    })();
    // Restore interactive echo no matter how the transfer ended (uf2send's
    // `finally`); a failure here shouldn't mask the transfer result.
    if let Err(e) = set_localecho(url, iface, true) {
        eprintln!("note: failed to restore localecho on: {e:#}");
    }
    result
}

/// Read + validate the UF2 files named on the command line, before any power
/// or REPL action is taken.
pub fn load_uf2_files(paths: &[String]) -> Result<Vec<(String, Vec<Uf2Block>)>> {
    let mut files = Vec::new();
    for p in paths {
        let bytes = std::fs::read(p).map_err(|e| anyhow!("{p}: {e}"))?;
        let blocks = parse_uf2(p, &bytes)?;
        files.push((p.clone(), blocks));
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── base64: pinned to RFC 4648 test vectors ─────────────────────────────

    #[test]
    fn base64_rfc4648_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_full_block_length() {
        // A 512-byte block must encode to ceil(512/3)*4 = 684 chars, padded.
        let block = vec![0xA5u8; 512];
        let e = base64_encode(&block);
        assert_eq!(e.len(), 684);
        // 512 % 3 == 2: a two-byte final chunk pads with exactly one '='.
        assert!(e.ends_with('=') && !e.ends_with("=="), "{}", &e[680..]);
    }

    // ── UF2 parsing ─────────────────────────────────────────────────────────

    fn uf2_block(addr: u32, payload: u32) -> Vec<u8> {
        let mut b = vec![0u8; 512];
        b[0..4].copy_from_slice(&UF2_MAGIC_START0.to_le_bytes());
        b[4..8].copy_from_slice(&UF2_MAGIC_START1.to_le_bytes());
        b[12..16].copy_from_slice(&addr.to_le_bytes());
        b[16..20].copy_from_slice(&payload.to_le_bytes());
        b[508..512].copy_from_slice(&UF2_MAGIC_END.to_le_bytes());
        b
    }

    #[test]
    fn parse_uf2_extracts_addr_and_size() {
        let mut img = uf2_block(0x6010_0000, 256);
        img.extend(uf2_block(0x6010_0100, 476));
        let blocks = parse_uf2("t.uf2", &img).unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].target_addr, 0x6010_0000);
        assert_eq!(blocks[0].payload_size, 256);
        assert_eq!(blocks[1].target_addr, 0x6010_0100);
        assert_eq!(blocks[1].raw.len(), 512);
    }

    #[test]
    fn parse_uf2_rejects_bad_size_and_magic() {
        let e = parse_uf2("t.uf2", &[0u8; 100]).unwrap_err();
        assert!(e.to_string().contains("not a multiple"), "{e}");
        let e = parse_uf2("t.uf2", &[0u8; 512]).unwrap_err();
        assert!(e.to_string().contains("bad UF2 magics"), "{e}");
        let e = parse_uf2("t.uf2", &[]).unwrap_err();
        assert!(e.to_string().contains("empty"), "{e}");
    }

    #[test]
    fn parse_uf2_rejects_implausible_payload() {
        let img = uf2_block(0x6000_0000, 500); // > 476-byte payload field
        let e = parse_uf2("t.uf2", &img).unwrap_err();
        assert!(e.to_string().contains("implausible payload"), "{e}");
    }

    // ── ack validation (the late-ack self-correction) ───────────────────────

    fn resp(captures: Vec<Option<String>>) -> serial::ExpectResponse {
        serial::ExpectResponse {
            matched: true,
            match_text: Some("Wrote…".into()),
            captures,
            elapsed_ms: Some(1),
            lagged: false,
            tail: None,
        }
    }

    fn block(addr: u32, size: u32) -> Uf2Block {
        Uf2Block {
            raw: vec![],
            target_addr: addr,
            payload_size: size,
        }
    }

    #[test]
    fn ack_matches_validates_both_fields() {
        let b = block(0x6010_0000, 476);
        assert!(ack_matches(
            &resp(vec![Some("476".into()), Some("0x60100000".into())]),
            &b
        ));
        // A late ack from the previous block (different address) must NOT count.
        assert!(!ack_matches(
            &resp(vec![Some("476".into()), Some("0x600FFE24".into())]),
            &b
        ));
        // Size mismatch (partial final block ack) must NOT count.
        assert!(!ack_matches(
            &resp(vec![Some("256".into()), Some("0x60100000".into())]),
            &b
        ));
        // An error alternation match carries no captures.
        assert!(!ack_matches(&resp(vec![None, None]), &b));
    }
}
