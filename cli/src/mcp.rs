//! `paniolo mcp`: paniolo's runtime commands as MCP tools over stdio.
//!
//! A Model Context Protocol server speaking newline-delimited JSON-RPC 2.0 on
//! stdin/stdout, hand-written rather than built on an SDK (design and
//! dependency decision: `notes/mcp-server.md`). It implements the tools-only
//! subset an agent harness needs — `initialize`, `ping`, `tools/list`,
//! `tools/call` — and ignores notifications.
//!
//! **stdout is the protocol channel.** Nothing else may write to it, so a tool
//! never runs a command handler in-process (handlers print, and `video shot`
//! exits the process). A tool that drives a channel runs this same binary as
//! a child with stdin closed and stdout captured, which also gives it the
//! CLI's remote dispatch unchanged. Read-only lab queries (`target_list`) run
//! in-process because they print nothing.
//!
//! The server holds no state of its own: the daemons hold the real state, so
//! a restart loses nothing.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::Result;
use serde_json::{json, Value};

/// MCP revisions this server accepts. The tools-only subset is identical
/// across them; an `initialize` naming one is answered with that one, and
/// anything else with the newest.
const PROTOCOL_VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// Longest `timeout_ms` a `video_shot` call may ask for. The call blocks the
/// server (requests are handled one at a time), so it is bounded.
const MAX_SHOT_TIMEOUT_MS: u64 = 120_000;
const DEFAULT_SHOT_TIMEOUT_MS: u64 = 2_000;

// JSON-RPC 2.0 error codes.
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// Run the server on this process's stdin/stdout until stdin closes.
pub fn serve(lab_flag: Option<&str>) -> Result<()> {
    let server = Server {
        exe: std::env::current_exe()?,
        lab: lab_flag.map(str::to_string),
    };
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    server.run(stdin.lock(), stdout.lock())
}

struct Server {
    /// The paniolo binary tools run as a child.
    exe: PathBuf,
    /// The `--lab` the server was started with, passed on to every child.
    lab: Option<String>,
}

impl Server {
    fn run(&self, input: impl BufRead, mut output: impl Write) -> Result<()> {
        for line in input.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let reply = match serde_json::from_str::<Value>(&line) {
                Ok(msg) => self.handle(&msg),
                Err(e) => Some(error_reply(
                    Value::Null,
                    PARSE_ERROR,
                    &format!("parse error: {e}"),
                )),
            };
            if let Some(reply) = reply {
                // serde_json never emits a raw newline, so one message stays
                // one line, as the stdio transport requires.
                writeln!(output, "{reply}")?;
                output.flush()?;
            }
        }
        Ok(())
    }

    /// One incoming message → its reply, or `None` for a notification.
    fn handle(&self, msg: &Value) -> Option<Value> {
        let Some(obj) = msg.as_object() else {
            // Includes JSON-RPC batches, which MCP no longer allows.
            return Some(error_reply(
                Value::Null,
                INVALID_REQUEST,
                "expected one JSON-RPC object",
            ));
        };
        let method = obj.get("method").and_then(Value::as_str);
        // No id: a notification (`notifications/initialized`,
        // `notifications/cancelled`, …). Nothing to answer.
        let id = obj.get("id")?.clone();
        let Some(method) = method else {
            return Some(error_reply(id, INVALID_REQUEST, "missing method"));
        };
        let params = obj.get("params").cloned().unwrap_or(Value::Null);
        Some(match method {
            "initialize" => result_reply(id, initialize(&params)),
            "ping" => result_reply(id, json!({})),
            "tools/list" => result_reply(id, json!({ "tools": tool_list() })),
            "tools/call" => match self.call_tool(&params) {
                Ok(result) => result_reply(id, result),
                Err(message) => error_reply(id, INVALID_PARAMS, &message),
            },
            other => error_reply(id, METHOD_NOT_FOUND, &format!("unknown method: {other}")),
        })
    }

    /// `tools/call`. `Err` is a protocol error (unknown tool, bad arguments);
    /// a tool that ran and failed is `Ok` with `isError: true`, so the agent
    /// sees the failure as a result it can act on.
    fn call_tool(&self, params: &Value) -> Result<Value, String> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or("tools/call needs a tool name")?;
        let args = params.get("arguments").cloned().unwrap_or(json!({}));
        match name {
            "target_list" => Ok(self.target_list()),
            "video_shot" => {
                let req = ShotRequest::from_args(&args)?;
                Ok(self.video_shot(&req))
            }
            other => Err(format!("unknown tool: {other}")),
        }
    }

    fn target_list(&self) -> Value {
        let lab = match crate::load_for_read(self.lab.as_deref()) {
            Ok(lab) => lab,
            Err(e) => return tool_error(&format!("{e:#}")),
        };
        let targets: Vec<Value> = lab
            .targets
            .keys()
            .filter_map(|name| lab.resolved_target(name))
            .map(|rt| {
                let channels: Vec<Value> = rt
                    .channels
                    .iter()
                    .map(|c| json!({ "kind": c.kind.as_str(), "name": c.name, "host": c.host }))
                    .collect();
                json!({
                    "name": rt.name,
                    "description": rt.description,
                    "hosts": rt.hosts(),
                    "channels": channels,
                })
            })
            .collect();
        tool_text(&serde_json::to_string_pretty(&targets).unwrap_or_default())
    }

    fn video_shot(&self, req: &ShotRequest) -> Value {
        let dir = match tempfile::tempdir() {
            Ok(d) => d,
            Err(e) => return tool_error(&format!("creating a temp dir for the shot: {e}")),
        };
        let png = dir.path().join("shot.png");
        let out = match self.run_child(&req.argv(&png)) {
            Ok(out) => out,
            Err(e) => return tool_error(&format!("running paniolo video shot: {e}")),
        };
        let stderr = String::from_utf8_lossy(&out.stderr);
        if !out.status.success() {
            let code = crate::error::shell_code(out.status);
            return tool_error(&format!(
                "paniolo video shot exited {code}\n{}",
                stderr.trim_end()
            ));
        }
        let bytes = match std::fs::read(&png) {
            Ok(b) => b,
            Err(e) => return tool_error(&format!("reading the captured PNG: {e}")),
        };
        // hdmicap prints `signal=<s>  hash=<h>[  (timeout)]`; pass it through
        // so the agent has the hash for its next `changed_since`.
        let status = stderr
            .lines()
            .find(|l| l.contains("hash="))
            .unwrap_or("")
            .trim();
        json!({
            "content": [
                { "type": "image", "data": base64(&bytes), "mimeType": "image/png" },
                { "type": "text", "text": status },
            ],
            "isError": false,
        })
    }

    /// Run this binary with `args`, never letting it near the protocol
    /// stream: stdin closed (it must not read our requests), stdout and
    /// stderr captured.
    fn run_child(&self, args: &[String]) -> std::io::Result<std::process::Output> {
        let mut cmd = Command::new(&self.exe);
        if let Some(lab) = &self.lab {
            cmd.arg("--lab").arg(lab);
        }
        cmd.arg("--json-errors")
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
    }
}

/// Validated `video_shot` arguments.
#[derive(Debug, PartialEq)]
struct ShotRequest {
    target: String,
    changed_since: Option<String>,
    stable: bool,
    timeout_ms: u64,
}

impl ShotRequest {
    fn from_args(args: &Value) -> Result<Self, String> {
        let target = args
            .get("target")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
            .ok_or("video_shot needs a target")?
            .to_string();
        let changed_since = match args.get("changed_since") {
            None | Some(Value::Null) => None,
            Some(Value::String(h)) => Some(h.clone()),
            Some(_) => return Err("changed_since must be a string".into()),
        };
        let stable = match args.get("stable") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(b)) => *b,
            Some(_) => return Err("stable must be a boolean".into()),
        };
        let timeout_ms = match args.get("timeout_ms") {
            None | Some(Value::Null) => DEFAULT_SHOT_TIMEOUT_MS,
            Some(v) => v
                .as_u64()
                .filter(|t| *t <= MAX_SHOT_TIMEOUT_MS)
                .ok_or(format!(
                    "timeout_ms must be an integer from 0 to {MAX_SHOT_TIMEOUT_MS}"
                ))?,
        };
        Ok(Self {
            target,
            changed_since,
            stable,
            timeout_ms,
        })
    }

    /// The `paniolo` argv (after the global flags) for this shot.
    fn argv(&self, out: &Path) -> Vec<String> {
        // `-t` rather than a positional, so a target named like an option
        // can never be read as one.
        let mut a = vec![
            "video".to_string(),
            "shot".to_string(),
            "-t".to_string(),
            self.target.clone(),
            "--timeout".to_string(),
            self.timeout_ms.to_string(),
            "--out".to_string(),
            out.to_string_lossy().into_owned(),
        ];
        if self.stable {
            a.push("--stable".to_string());
        }
        if let Some(h) = &self.changed_since {
            a.push("--changed-since".to_string());
            a.push(h.clone());
        }
        a
    }
}

fn initialize(params: &Value) -> Value {
    let requested = params.get("protocolVersion").and_then(Value::as_str);
    let version = requested
        .filter(|v| PROTOCOL_VERSIONS.contains(v))
        .unwrap_or(PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "paniolo", "version": crate::VERSION },
        "instructions": "Drive embedded target machines managed by paniolo. \
            Call target_list first to see the targets and their channels. \
            video_shot returns the target's screen as an image plus a hash; \
            pass that hash back as changed_since to wait for the screen to change.",
    })
}

fn tool_list() -> Value {
    json!([
        {
            "name": "target_list",
            "description": "List the lab's targets: each target's name, description, \
                the control hosts it lives on, and its channels (video, serial, hid, \
                power, …) with the host each runs on.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true },
        },
        {
            "name": "video_shot",
            "description": "Capture the target's current screen from its HDMI-capture \
                (video) channel. Returns the PNG and a text line `signal=<s>  hash=<h>`. \
                The video daemon must already be running (`paniolo video watch <target>`).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target": { "type": "string", "description": "Target name, as listed by target_list." },
                    "changed_since": {
                        "type": "string",
                        "description": "Wait until the frame's hash differs from this one (the hash a previous video_shot returned)."
                    },
                    "stable": { "type": "boolean", "description": "Wait until the signal is stable before capturing." },
                    "timeout_ms": {
                        "type": "integer",
                        "minimum": 0,
                        "maximum": MAX_SHOT_TIMEOUT_MS,
                        "description": "How long to wait for changed_since/stable, in ms (default 2000)."
                    }
                },
                "required": ["target"],
                "additionalProperties": false,
            },
            "annotations": { "readOnlyHint": true },
        },
    ])
}

fn result_reply(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error_reply(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn tool_text(text: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": false })
}

fn tool_error(text: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": true })
}

/// Standard base64 (RFC 4648, padded) — the one encoding MCP image content
/// needs, and too small to justify a dependency.
fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * i)) & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(exe: PathBuf, lab: Option<String>) -> Server {
        Server { exe, lab }
    }

    /// Drive the server over in-memory stdin/stdout, one request per line.
    fn exchange(s: &Server, requests: &[Value]) -> Vec<Value> {
        let input: String = requests.iter().map(|r| format!("{r}\n")).collect();
        let mut output = Vec::new();
        s.run(input.as_bytes(), &mut output).unwrap();
        String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn req(id: i64, method: &str, params: Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
    }

    #[test]
    fn base64_matches_rfc4648_vectors() {
        for (input, expected) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(input.as_bytes()), expected, "{input:?}");
        }
        assert_eq!(base64(&[0xff, 0xfe, 0xfd]), "//79");
    }

    #[test]
    fn initialize_echoes_a_known_version_and_offers_tools() {
        let s = server(PathBuf::from("/nonexistent"), None);
        let r = exchange(
            &s,
            &[req(
                1,
                "initialize",
                json!({ "protocolVersion": "2025-06-18" }),
            )],
        );
        assert_eq!(r[0]["id"], 1);
        assert_eq!(r[0]["result"]["protocolVersion"], "2025-06-18");
        assert!(r[0]["result"]["capabilities"]["tools"].is_object());
        assert_eq!(r[0]["result"]["serverInfo"]["name"], "paniolo");
    }

    #[test]
    fn initialize_answers_an_unknown_version_with_the_newest() {
        let s = server(PathBuf::from("/nonexistent"), None);
        let r = exchange(
            &s,
            &[req(
                1,
                "initialize",
                json!({ "protocolVersion": "1999-01-01" }),
            )],
        );
        assert_eq!(r[0]["result"]["protocolVersion"], PROTOCOL_VERSIONS[0]);
    }

    /// Notifications get no reply, and blank lines are skipped, so the reply
    /// stream lines up one-to-one with the requests that carry an id.
    #[test]
    fn notifications_and_blank_lines_get_no_reply() {
        let s = server(PathBuf::from("/nonexistent"), None);
        let input = format!(
            "{}\n\n{}\n",
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            req(7, "ping", json!({}))
        );
        let mut output = Vec::new();
        s.run(input.as_bytes(), &mut output).unwrap();
        let replies: Vec<Value> = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0]["id"], 7);
        assert_eq!(replies[0]["result"], json!({}));
    }

    #[test]
    fn protocol_errors_carry_json_rpc_codes() {
        let s = server(PathBuf::from("/nonexistent"), None);
        let mut output = Vec::new();
        s.run("not json\n[1,2]\n".as_bytes(), &mut output).unwrap();
        let replies: Vec<Value> = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(replies[0]["error"]["code"], PARSE_ERROR);
        assert_eq!(replies[1]["error"]["code"], INVALID_REQUEST);

        let r = exchange(
            &s,
            &[
                req(1, "resources/list", json!({})),
                req(2, "tools/call", json!({ "name": "no_such_tool" })),
                req(
                    3,
                    "tools/call",
                    json!({ "name": "video_shot", "arguments": {} }),
                ),
            ],
        );
        assert_eq!(r[0]["error"]["code"], METHOD_NOT_FOUND);
        assert_eq!(r[1]["error"]["code"], INVALID_PARAMS);
        assert_eq!(r[2]["error"]["code"], INVALID_PARAMS);
    }

    #[test]
    fn tools_list_names_both_tools_with_object_schemas() {
        let s = server(PathBuf::from("/nonexistent"), None);
        let r = exchange(&s, &[req(1, "tools/list", json!({}))]);
        let tools = r[0]["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["target_list", "video_shot"]);
        for t in tools {
            assert_eq!(t["inputSchema"]["type"], "object", "{}", t["name"]);
        }
    }

    #[test]
    fn shot_request_validates_and_builds_argv() {
        let r = ShotRequest::from_args(&json!({
            "target": "pi5", "changed_since": "abc", "stable": true, "timeout_ms": 5000
        }))
        .unwrap();
        assert_eq!(
            r.argv(Path::new("/tmp/x.png")),
            [
                "video",
                "shot",
                "-t",
                "pi5",
                "--timeout",
                "5000",
                "--out",
                "/tmp/x.png",
                "--stable",
                "--changed-since",
                "abc"
            ]
        );
        let d = ShotRequest::from_args(&json!({ "target": "pi5" })).unwrap();
        assert_eq!(d.timeout_ms, DEFAULT_SHOT_TIMEOUT_MS);
        assert!(!d.stable && d.changed_since.is_none());

        for bad in [
            json!({}),
            json!({ "target": "" }),
            json!({ "target": "pi5", "timeout_ms": MAX_SHOT_TIMEOUT_MS + 1 }),
            json!({ "target": "pi5", "timeout_ms": -1 }),
            json!({ "target": "pi5", "stable": "yes" }),
            json!({ "target": "pi5", "changed_since": 7 }),
        ] {
            assert!(ShotRequest::from_args(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn target_list_reads_the_lab() {
        let dir = tempfile::tempdir().unwrap();
        let lab = dir.path().join("lab.toml");
        std::fs::write(
            &lab,
            "[targets.pi5]\ndescription = \"bench pi\"\n[targets.pi5.video]\ndevice = \"cap0\"\n",
        )
        .unwrap();
        let s = server(
            PathBuf::from("/nonexistent"),
            Some(lab.to_string_lossy().into_owned()),
        );
        let r = exchange(
            &s,
            &[req(1, "tools/call", json!({ "name": "target_list" }))],
        );
        let result = &r[0]["result"];
        assert_eq!(result["isError"], false, "{result}");
        let targets: Value =
            serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(targets[0]["name"], "pi5");
        assert_eq!(targets[0]["description"], "bench pi");
        assert_eq!(targets[0]["channels"][0]["kind"], "video");
    }

    /// A fake `paniolo` that behaves like `video shot`: writes PNG bytes to
    /// the `--out` path and the status line to stderr. Proves the child is
    /// really run with the global flags first, its file read back, and its
    /// stdin closed (the script would block on `read` otherwise).
    #[cfg(unix)]
    fn fake_paniolo(dir: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join("paniolo");
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[cfg(unix)]
    #[test]
    fn video_shot_runs_the_cli_and_returns_the_image() {
        let dir = tempfile::tempdir().unwrap();
        let args_log = dir.path().join("args");
        let exe = fake_paniolo(
            dir.path(),
            &format!(
                r#"echo "$@" > {log}
read -r _ && exit 9
while [ $# -gt 0 ]; do [ "$1" = --out ] && printf 'PNGDATA' > "$2"; shift; done
echo 'signal=ok  hash=00ff' >&2"#,
                log = args_log.display()
            ),
        );
        let s = server(exe, Some("/some/lab.toml".into()));
        let r = exchange(
            &s,
            &[req(
                1,
                "tools/call",
                json!({ "name": "video_shot", "arguments": { "target": "pi5" } }),
            )],
        );
        let result = &r[0]["result"];
        assert_eq!(result["isError"], false, "{result}");
        assert_eq!(result["content"][0]["type"], "image");
        assert_eq!(result["content"][0]["data"], base64(b"PNGDATA"));
        assert_eq!(result["content"][0]["mimeType"], "image/png");
        assert_eq!(result["content"][1]["text"], "signal=ok  hash=00ff");
        let args = std::fs::read_to_string(&args_log).unwrap();
        assert!(
            args.starts_with("--lab /some/lab.toml --json-errors video shot -t pi5"),
            "{args}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn video_shot_failure_is_a_tool_error_with_the_exit_code() {
        let dir = tempfile::tempdir().unwrap();
        let exe = fake_paniolo(dir.path(), "echo 'no video daemon running' >&2; exit 6");
        let s = server(exe, None);
        let r = exchange(
            &s,
            &[req(
                1,
                "tools/call",
                json!({ "name": "video_shot", "arguments": { "target": "pi5" } }),
            )],
        );
        let result = &r[0]["result"];
        assert_eq!(result["isError"], true);
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("exited 6"), "{text}");
        assert!(text.contains("no video daemon running"), "{text}");
    }
}
