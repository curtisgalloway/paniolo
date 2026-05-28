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

//! Timestamped, line-oriented capture of serial output.
//!
//! The daemon owns the port and is the only process that sees every byte, so it
//! assembles incoming bytes into lines, stamps each with a UTC timestamp and a
//! monotonic sequence number, and appends them to a rolling on-disk log under the
//! runtime dir. History survives daemon restarts (the sequence counter resumes
//! from the last line on disk) and grows well past the live-view window; old
//! lines age out by segment rotation.
//!
//! The `serialcap log` client reads these files directly — no daemon round-trip,
//! and it still works after the daemon has stopped. The current unterminated line
//! (e.g. a `login:` prompt that hasn't emitted a newline yet) lives only in the
//! daemon's memory, so it is mirrored to a small sidecar file the reader folds in
//! as the most recent (partial) line.
//!
//! Lines are stored *raw* (ANSI escapes and control bytes preserved); the reader
//! cleans them for display unless `--raw` is given, so no information is lost.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Default number of lines retained across all rotated segments.
pub const DEFAULT_BUFFER_LINES: u64 = 50_000;
/// Rotated segments kept (active + this many `.N` files).
const MAX_SEGMENTS: usize = 5;
/// Hard cap on a single unterminated line; bytes past this are force-flushed so a
/// newline-less stream can't grow the pending buffer without bound.
const MAX_PENDING: usize = 64 * 1024;

const ACTIVE: &str = "serial.jsonl";
const PENDING: &str = "pending.json";

/// One line of captured serial output.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Line {
    pub seq: u64,
    /// Wall-clock capture time, milliseconds since the Unix epoch (UTC).
    pub ts_ms: u64,
    /// Raw line text: ANSI escapes / control bytes preserved, trailing CR removed.
    pub text: String,
    /// True only for the in-flight line that has not seen its newline yet.
    #[serde(default, skip_serializing_if = "is_false")]
    pub partial: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Base directory holding per-interface capture sub-directories. Shares the
/// daemon's runtime dir so the writer (daemon) and reader (`serialcap log`)
/// always agree on the path.
pub fn capture_dir() -> Result<PathBuf> {
    let dir = crate::daemon::runtime_dir()?.join("capture");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// The capture sub-directory for one named interface.
pub fn interface_dir(base: &Path, name: &str) -> PathBuf {
    base.join(sanitize(name))
}

/// Make an interface name safe as a single path component (interface names are
/// user-chosen). Keeps alphanumerics, `-`, `_`, `.`; collapses everything else
/// to `_`. An empty/degenerate result falls back to `_`.
fn sanitize(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() || s == "." || s == ".." {
        "_".to_string()
    } else {
        s
    }
}

/// Names of interfaces that have a capture sub-directory under `base`.
fn list_interface_dirs(base: &Path) -> Vec<String> {
    let mut names = Vec::new();
    if let Ok(entries) = fs::read_dir(base) {
        for e in entries.flatten() {
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                if let Some(n) = e.file_name().to_str() {
                    names.push(n.to_string());
                }
            }
        }
    }
    names.sort();
    names
}

// ── writer (owned by the capture thread) ────────────────────────────────────

/// Append-only writer that turns a byte stream into timestamped lines on disk.
/// Lives on a dedicated thread; never shared, so it needs no locking.
pub struct LineLog {
    dir: PathBuf,
    active_path: PathBuf,
    writer: Option<File>,
    next_seq: u64,
    seg_lines: u64,
    active_lines: u64,
    pending: Vec<u8>,
    pending_ts: Option<u64>,
}

impl LineLog {
    /// Open (creating it if needed) the capture log in `dir`, resuming the
    /// sequence counter after the highest line already on disk. A stale pending
    /// sidecar from a previous run is discarded (its line was never completed).
    pub fn open(dir: PathBuf, buffer_lines: u64) -> Self {
        let _ = fs::create_dir_all(&dir);
        let active_path = dir.join(ACTIVE);
        let seg_lines = (buffer_lines / MAX_SEGMENTS as u64).max(1);
        let next_seq = recover_next_seq(&dir);
        let active_lines = count_lines(&active_path);
        let _ = fs::remove_file(dir.join(PENDING));
        let writer = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&active_path)
            .ok();
        LineLog {
            dir,
            active_path,
            writer,
            next_seq,
            seg_lines,
            active_lines,
            pending: Vec::new(),
            pending_ts: None,
        }
    }

    /// Feed a chunk of received bytes. Completed lines are appended to the log;
    /// any trailing partial line is mirrored to the sidecar.
    pub fn ingest(&mut self, bytes: &[u8]) {
        for &b in bytes {
            if b == b'\n' {
                if self.pending.last() == Some(&b'\r') {
                    self.pending.pop();
                }
                let text = String::from_utf8_lossy(&self.pending).into_owned();
                let ts = self.pending_ts.take().unwrap_or_else(now_ms);
                self.commit(ts, text);
                self.pending.clear();
            } else {
                if self.pending.is_empty() {
                    self.pending_ts = Some(now_ms());
                }
                self.pending.push(b);
                if self.pending.len() >= MAX_PENDING {
                    let text = String::from_utf8_lossy(&self.pending).into_owned();
                    let ts = self.pending_ts.take().unwrap_or_else(now_ms);
                    self.commit(ts, text);
                    self.pending.clear();
                }
            }
        }
        self.write_pending_sidecar();
    }

    fn commit(&mut self, ts_ms: u64, text: String) {
        let line = Line {
            seq: self.next_seq,
            ts_ms,
            text,
            partial: false,
        };
        self.next_seq += 1;
        if let Some(w) = self.writer.as_mut() {
            if let Ok(mut s) = serde_json::to_string(&line) {
                s.push('\n');
                if w.write_all(s.as_bytes()).is_ok() {
                    self.active_lines += 1;
                }
            }
        }
        if self.active_lines >= self.seg_lines {
            self.rotate();
        }
    }

    /// Shift `serial.jsonl(.k)` → `.k+1`, dropping the oldest, and start a fresh
    /// active segment. Keeps at most `MAX_SEGMENTS` files.
    fn rotate(&mut self) {
        self.writer = None; // close before renaming
        let _ = fs::remove_file(self.dir.join(format!("{ACTIVE}.{}", MAX_SEGMENTS - 1)));
        for k in (1..MAX_SEGMENTS - 1).rev() {
            let from = self.dir.join(format!("{ACTIVE}.{k}"));
            let to = self.dir.join(format!("{ACTIVE}.{}", k + 1));
            let _ = fs::rename(&from, &to);
        }
        let _ = fs::rename(&self.active_path, self.dir.join(format!("{ACTIVE}.1")));
        self.writer = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.active_path)
            .ok();
        self.active_lines = 0;
    }

    /// Mirror the current unterminated line to the sidecar (or remove it when the
    /// pending buffer is empty). Written via a temp file + rename so a concurrent
    /// reader never sees a half-written sidecar.
    fn write_pending_sidecar(&self) {
        let path = self.dir.join(PENDING);
        if self.pending.is_empty() {
            let _ = fs::remove_file(&path);
            return;
        }
        let line = Line {
            seq: self.next_seq,
            ts_ms: self.pending_ts.unwrap_or_else(now_ms),
            text: String::from_utf8_lossy(&self.pending).into_owned(),
            partial: true,
        };
        if let Ok(s) = serde_json::to_string(&line) {
            let tmp = self.dir.join("pending.tmp");
            if fs::write(&tmp, s.as_bytes()).is_ok() {
                let _ = fs::rename(&tmp, &path);
            }
        }
    }
}

// ── reader (used by `serialcap log`) ─────────────────────────────────────────

/// A line selection for [`read_lines`]. An unset field imposes no constraint.
#[derive(Default)]
pub struct Query {
    /// Keep only the most recent N lines (applied after the seq filters).
    pub tail: Option<u64>,
    /// Lowest sequence number to include (inclusive).
    pub from: Option<u64>,
    /// Highest sequence number to include (inclusive).
    pub to: Option<u64>,
    /// Only lines with `seq` strictly greater than this.
    pub since: Option<u64>,
    /// Fold in the current unterminated line as the last (partial) entry.
    pub include_pending: bool,
}

/// Read captured lines from `dir`, oldest first, applying `q`.
pub fn read_lines(dir: &Path, q: &Query) -> Vec<Line> {
    let mut all: Vec<Line> = Vec::new();
    for k in (1..MAX_SEGMENTS).rev() {
        for_each_line(&dir.join(format!("{ACTIVE}.{k}")), |l| all.push(l));
    }
    for_each_line(&dir.join(ACTIVE), |l| all.push(l));

    if q.include_pending {
        if let Some(p) = read_pending(dir) {
            all.push(p);
        }
    }

    let mut out: Vec<Line> = all
        .into_iter()
        .filter(|l| {
            if let Some(s) = q.since {
                if l.seq <= s {
                    return false;
                }
            }
            if let Some(f) = q.from {
                if l.seq < f {
                    return false;
                }
            }
            if let Some(t) = q.to {
                if l.seq > t {
                    return false;
                }
            }
            true
        })
        .collect();

    if let Some(n) = q.tail {
        let n = n as usize;
        if out.len() > n {
            out.drain(0..out.len() - n);
        }
    }
    out
}

fn read_pending(dir: &Path) -> Option<Line> {
    let s = fs::read_to_string(dir.join(PENDING)).ok()?;
    serde_json::from_str(&s).ok()
}

/// Parse each JSON-lines record in `path`, invoking `f` per valid line. Missing
/// files and unparseable lines (e.g. a torn final append) are skipped silently.
fn for_each_line(path: &Path, mut f: impl FnMut(Line)) {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return,
    };
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        if let Ok(parsed) = serde_json::from_str::<Line>(&line) {
            f(parsed);
        }
    }
}

fn recover_next_seq(dir: &Path) -> u64 {
    let mut max_seq: Option<u64> = None;
    for k in (1..MAX_SEGMENTS).rev() {
        for_each_line(&dir.join(format!("{ACTIVE}.{k}")), |l| {
            max_seq = Some(max_seq.map_or(l.seq, |m| m.max(l.seq)));
        });
    }
    for_each_line(&dir.join(ACTIVE), |l| {
        max_seq = Some(max_seq.map_or(l.seq, |m| m.max(l.seq)));
    });
    max_seq.map_or(0, |m| m + 1)
}

fn count_lines(path: &Path) -> u64 {
    let mut n = 0;
    for_each_line(path, |_| n += 1);
    n
}

// ── the `log` subcommand ─────────────────────────────────────────────────────

/// Options for [`cmd_log`], mirroring the `serialcap log` CLI flags.
pub struct LogArgs {
    pub interface: Option<String>,
    pub tail: Option<u64>,
    pub from: Option<u64>,
    pub to: Option<u64>,
    pub since: Option<u64>,
    pub raw: bool,
    pub json: bool,
    pub no_pending: bool,
}

/// Print captured lines to stdout per `args`.
pub fn cmd_log(args: LogArgs) -> Result<()> {
    let base = capture_dir().context("locating capture dir")?;
    let dir = match &args.interface {
        Some(name) => interface_dir(&base, name),
        None => {
            let names = list_interface_dirs(&base);
            match names.len() {
                // Exactly one interface (or none yet): no need to disambiguate.
                0 | 1 => names.first().map_or(base.clone(), |n| base.join(n)),
                // Expected user-choice condition, not a fault: a clean message,
                // no error chain / backtrace.
                _ => {
                    eprintln!(
                        "serialcap: multiple interfaces captured ({}); pass --interface NAME",
                        names.join(", ")
                    );
                    std::process::exit(2);
                }
            }
        }
    };

    // With no selector, default to a recent window rather than the whole history.
    let no_selector =
        args.tail.is_none() && args.from.is_none() && args.to.is_none() && args.since.is_none();
    let q = Query {
        tail: if no_selector { Some(200) } else { args.tail },
        from: args.from,
        to: args.to,
        since: args.since,
        include_pending: !args.no_pending,
    };

    let lines = read_lines(&dir, &q);
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    for l in lines {
        if args.json {
            if let Ok(s) = serde_json::to_string(&l) {
                let _ = writeln!(out, "{s}");
            }
        } else {
            let text = if args.raw {
                l.text
            } else {
                strip_ansi(&l.text)
            };
            let seq = if l.partial {
                format!("{}*", l.seq)
            } else {
                l.seq.to_string()
            };
            let _ = writeln!(out, "[{}] #{:<7} {}", format_utc(l.ts_ms), seq, text);
        }
    }
    Ok(())
}

// ── formatting helpers ───────────────────────────────────────────────────────

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Format epoch milliseconds as `YYYY-MM-DDTHH:MM:SS.mmmZ` (UTC). Done by hand so
/// the crate needs no calendar dependency.
fn format_utc(ms: u64) -> String {
    let secs = (ms / 1000) as i64;
    let millis = ms % 1000;
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        tod / 3600,
        (tod % 3600) / 60,
        tod % 60,
    )
}

/// Howard Hinnant's `civil_from_days`: days since 1970-01-01 → (year, month, day).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (y + if m <= 2 { 1 } else { 0 }, m as u32, d)
}

/// Strip ANSI escape sequences and control noise for readable, agent-friendly
/// text. Removes CSI/OSC and other escape sequences, applies bare-`\r` carriage
/// returns as overwrites (keeps text after the last `\r`), and drops remaining
/// control characters except tab. Operates on raw bytes so it is robust to
/// partial UTF-8.
pub fn strip_ansi(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b {
            i += 1;
            if i >= bytes.len() {
                break;
            }
            match bytes[i] {
                b'[' => {
                    // CSI: parameters/intermediates until a final byte 0x40..=0x7e.
                    i += 1;
                    while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                        i += 1;
                    }
                    if i < bytes.len() {
                        i += 1;
                    }
                }
                b']' => {
                    // OSC: until BEL or ST (ESC \).
                    i += 1;
                    while i < bytes.len() {
                        if bytes[i] == 0x07 {
                            i += 1;
                            break;
                        }
                        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                            i += 2;
                            break;
                        }
                        i += 1;
                    }
                }
                _ => i += 1, // two-byte escape: drop the following byte too
            }
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }

    let cleaned = String::from_utf8_lossy(&out);
    // Bare carriage returns redraw the line; keep only the final overwrite.
    let cleaned = match cleaned.rfind('\r') {
        Some(idx) => &cleaned[idx + 1..],
        None => &cleaned,
    };
    cleaned
        .chars()
        .filter(|&c| c == '\t' || !c.is_control())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "serialcap-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn splits_lines_and_strips_cr() {
        let dir = tmp();
        let mut log = LineLog::open(dir.clone(), DEFAULT_BUFFER_LINES);
        log.ingest(b"hello\r\nworld\n");
        let lines = read_lines(&dir, &Query::default());
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "hello");
        assert_eq!(lines[0].seq, 0);
        assert_eq!(lines[1].text, "world");
        assert_eq!(lines[1].seq, 1);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unterminated_line_is_pending() {
        let dir = tmp();
        let mut log = LineLog::open(dir.clone(), DEFAULT_BUFFER_LINES);
        log.ingest(b"done\nlogin: ");
        let with = read_lines(
            &dir,
            &Query {
                include_pending: true,
                ..Default::default()
            },
        );
        assert_eq!(with.len(), 2);
        assert!(with[1].partial);
        assert_eq!(with[1].text, "login: ");
        assert_eq!(with[1].seq, 1); // seq it will take once completed

        let without = read_lines(&dir, &Query::default());
        assert_eq!(without.len(), 1);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn tail_and_range_and_since() {
        let dir = tmp();
        let mut log = LineLog::open(dir.clone(), DEFAULT_BUFFER_LINES);
        for i in 0..10 {
            log.ingest(format!("line{i}\n").as_bytes());
        }
        let tail = read_lines(
            &dir,
            &Query {
                tail: Some(3),
                ..Default::default()
            },
        );
        assert_eq!(tail.len(), 3);
        assert_eq!(tail[0].text, "line7");
        assert_eq!(tail[2].text, "line9");

        let range = read_lines(
            &dir,
            &Query {
                from: Some(2),
                to: Some(4),
                ..Default::default()
            },
        );
        assert_eq!(
            range.iter().map(|l| l.seq).collect::<Vec<_>>(),
            vec![2, 3, 4]
        );

        let since = read_lines(
            &dir,
            &Query {
                since: Some(7),
                ..Default::default()
            },
        );
        assert_eq!(since.iter().map(|l| l.seq).collect::<Vec<_>>(), vec![8, 9]);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn seq_resumes_after_reopen() {
        let dir = tmp();
        {
            let mut log = LineLog::open(dir.clone(), DEFAULT_BUFFER_LINES);
            log.ingest(b"a\nb\n");
        }
        let mut log = LineLog::open(dir.clone(), DEFAULT_BUFFER_LINES);
        log.ingest(b"c\n");
        let lines = read_lines(&dir, &Query::default());
        assert_eq!(
            lines.iter().map(|l| l.seq).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(lines[2].text, "c");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rotation_drops_oldest_keeps_recent() {
        let dir = tmp();
        // buffer_lines / MAX_SEGMENTS = seg_lines; 50 / 5 = 10 lines per segment.
        let mut log = LineLog::open(dir.clone(), 50);
        for i in 0..120 {
            log.ingest(format!("line{i}\n").as_bytes());
        }
        let lines = read_lines(&dir, &Query::default());
        // At most MAX_SEGMENTS * seg_lines retained, oldest aged out.
        assert!(lines.len() <= 50, "retained {} lines", lines.len());
        assert_eq!(lines.last().unwrap().text, "line119");
        assert_eq!(lines.last().unwrap().seq, 119);
        // Sequence numbers stay monotonic and contiguous over what survives.
        for w in lines.windows(2) {
            assert_eq!(w[1].seq, w[0].seq + 1);
        }
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn strip_ansi_removes_color_and_handles_cr() {
        assert_eq!(strip_ansi("\x1b[1;32mgreen\x1b[0m text"), "green text");
        assert_eq!(strip_ansi("progress 10%\rprogress 90%"), "progress 90%");
        assert_eq!(strip_ansi("keep\ttab"), "keep\ttab");
        assert_eq!(strip_ansi("bell\x07gone"), "bellgone");
        // OSC title sequence terminated by BEL.
        assert_eq!(strip_ansi("\x1b]0;my title\x07shell"), "shell");
    }

    #[test]
    fn format_utc_known_values() {
        assert_eq!(format_utc(0), "1970-01-01T00:00:00.000Z");
        // 2021-01-01T00:00:00Z = 1609459200 s.
        assert_eq!(format_utc(1_609_459_200_000), "2021-01-01T00:00:00.000Z");
        assert_eq!(format_utc(1_609_459_200_123), "2021-01-01T00:00:00.123Z");
    }
}
