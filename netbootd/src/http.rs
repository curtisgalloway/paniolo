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

//! Minimal read-only HTTP/1.1 server for UEFI HTTP Boot.
//!
//! Serves the same rooted directory as the TFTP server, but over TCP. The EDK2
//! `HttpBootDxe` client (and any follow-on fetches its NBP makes — GRUB reading
//! `grub.cfg`, an iPXE script chainloading, …) needs only:
//!   * **GET** and **HEAD** (it HEADs to size its buffer, then GETs);
//!   * an explicit **`Content-Length`** — we always know the file size, so we
//!     always send it and never use chunked transfer;
//!   * a sane **`Content-Type`** (default `application/octet-stream`, accepted as
//!     an EFI application).
//!
//! Unlike the silent Pi bootloader the TFTP path serves, a UEFI client owns a
//! full IP/TCP/ARP stack and answers ARP, so the host kernel delivers normally:
//! **no `/dev/bpf` raw-frame path, no setuid helper, no static ARP pin.** Path
//! resolution reuses [`crate::served::resolve`], so traversal safety is shared
//! with TFTP.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tracing::{info, warn};

use crate::served::resolve;

const DEFAULT_CONTENT_TYPE: &str = "application/octet-stream";
/// Streaming copy buffer for GET bodies (boot payloads can be tens of MB, so we
/// never load the whole file into memory).
const COPY_CHUNK: usize = 64 * 1024;
/// Upper bound on the request head (request line + headers). A boot client's
/// head is a few hundred bytes; this just stops a peer from streaming forever
/// without a terminator.
const MAX_HEAD_BYTES: usize = 16 * 1024;

/// Run the HTTP server until the task is cancelled.
pub async fn serve(root: PathBuf, port: u16, content_type: Option<String>) -> Result<()> {
    let root = root
        .canonicalize()
        .with_context(|| format!("HTTP root {} does not exist", root.display()))?;
    let listener = TcpListener::bind(("0.0.0.0", port))
        .await
        .with_context(|| {
            format!("bind HTTP port {port} (need root/CAP_NET_BIND_SERVICE on Linux)")
        })?;
    let content_type = content_type.unwrap_or_else(|| DEFAULT_CONTENT_TYPE.to_string());
    info!(root = %root.display(), content_type, "HTTP listening on 0.0.0.0:{port}");

    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(v) => v,
            Err(e) => {
                warn!("HTTP accept: {e}");
                continue;
            }
        };
        let root = root.clone();
        let ctype = content_type.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_conn(stream, root, ctype).await {
                warn!("HTTP {peer}: {e:#}");
            }
        });
    }
}

/// A parsed request head: method, request-target, and the two headers we act on.
struct Head {
    method: String,
    target: String,
    keep_alive: bool,
    content_length: usize,
}

/// Read and parse one request head (up to the blank-line terminator). Returns
/// `None` on a clean EOF before any bytes (the client closed an idle keep-alive
/// connection).
async fn read_head<R: AsyncRead + Unpin>(r: &mut R) -> Result<Option<Head>> {
    let mut raw = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let n = r.read(&mut byte).await?;
        if n == 0 {
            // Clean EOF (idle keep-alive close) or a head cut short at EOF —
            // either way there is no complete request to serve.
            return Ok(None);
        }
        raw.push(byte[0]);
        if raw.len() > MAX_HEAD_BYTES {
            anyhow::bail!("request head exceeds {MAX_HEAD_BYTES} bytes without terminator");
        }
        if raw.ends_with(b"\r\n\r\n") || raw.ends_with(b"\n\n") {
            break;
        }
    }

    let text = String::from_utf8_lossy(&raw);
    let mut lines = text.lines();
    let req_line = lines.next().unwrap_or("");
    let mut it = req_line.split_whitespace();
    let method = it.next().unwrap_or("").to_string();
    let target = it.next().unwrap_or("").to_string();

    let mut keep_alive = true; // HTTP/1.1 default
    let mut content_length = 0usize;
    for h in lines {
        let Some((name, value)) = h.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim();
        match name.as_str() {
            "connection" => keep_alive = !value.eq_ignore_ascii_case("close"),
            "content-length" => content_length = value.parse().unwrap_or(0),
            _ => {}
        }
    }
    Ok(Some(Head {
        method,
        target,
        keep_alive,
        content_length,
    }))
}

async fn handle_conn(stream: TcpStream, root: PathBuf, ctype: String) -> Result<()> {
    let mut reader = BufReader::new(stream);
    loop {
        let Some(head) = read_head(&mut reader).await? else {
            return Ok(());
        };

        // GET/HEAD carry no body, but drain a declared one so the next request
        // on a kept-alive connection stays framed.
        let mut remaining = head.content_length;
        let mut sink = [0u8; 4096];
        while remaining > 0 {
            let want = remaining.min(sink.len());
            let got = reader.read(&mut sink[..want]).await?;
            if got == 0 {
                return Ok(());
            }
            remaining -= got;
        }

        let head_only = head.method.eq_ignore_ascii_case("HEAD");
        let is_get = head.method.eq_ignore_ascii_case("GET");
        let stream = reader.get_mut();

        if !is_get && !head_only {
            info!("{} {} -> 405", head.method, head.target);
            write_status(
                stream,
                405,
                "Method Not Allowed",
                head.keep_alive,
                head_only,
            )
            .await?;
        } else if let Some(path) = resolve_target(&root, &head.target) {
            serve_file(
                stream,
                &path,
                &ctype,
                head.keep_alive,
                head_only,
                &head.target,
            )
            .await?;
        } else {
            info!("{} {} -> 404", head.method, head.target);
            write_status(stream, 404, "Not Found", head.keep_alive, head_only).await?;
        }

        if !head.keep_alive {
            return Ok(());
        }
    }
}

/// Map an HTTP request-target to a file inside `root`, or `None` (→ 404).
///
/// Accepts origin-form (`/grubaa64.efi`) and absolute-form
/// (`http://host/grubaa64.efi`); strips any query/fragment; percent-decodes; and
/// rejects anything that is not a regular file inside `root` (directories,
/// traversal, missing files).
fn resolve_target(root: &Path, target: &str) -> Option<PathBuf> {
    // absolute-form: drop scheme + authority, keep from the first '/'.
    let path = if let Some(rest) = target
        .strip_prefix("http://")
        .or_else(|| target.strip_prefix("https://"))
    {
        match rest.find('/') {
            Some(i) => &rest[i..],
            None => "/",
        }
    } else {
        target
    };
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let decoded = percent_decode(path);
    resolve(root, &decoded).filter(|p| p.is_file())
}

/// Decode `%XX` escapes; pass everything else through unchanged.
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hi = (b[i + 1] as char).to_digit(16);
            let lo = (b[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

async fn serve_file<W: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut W,
    path: &Path,
    ctype: &str,
    keep_alive: bool,
    head_only: bool,
    target: &str,
) -> Result<()> {
    let file = match File::open(path).await {
        Ok(f) => f,
        Err(e) => {
            warn!("open {}: {e}", path.display());
            return write_status(stream, 404, "Not Found", keep_alive, head_only).await;
        }
    };
    let len = file.metadata().await?.len();
    let conn = if keep_alive { "keep-alive" } else { "close" };
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {len}\r\nContent-Type: {ctype}\r\n\
         Connection: {conn}\r\n\r\n"
    );
    stream.write_all(header.as_bytes()).await?;
    info!(
        "{target} -> 200 ({len} bytes{})",
        if head_only { ", HEAD" } else { "" }
    );

    if !head_only {
        let mut file = file;
        let mut buf = vec![0u8; COPY_CHUNK];
        loop {
            let n = file.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            stream.write_all(&buf[..n]).await?;
        }
    }
    stream.flush().await?;
    Ok(())
}

async fn write_status<W: AsyncWrite + Unpin>(
    stream: &mut W,
    code: u16,
    reason: &str,
    keep_alive: bool,
    head_only: bool,
) -> Result<()> {
    let body = format!("{code} {reason}\n");
    let conn = if keep_alive { "keep-alive" } else { "close" };
    let header = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Length: {}\r\nContent-Type: text/plain\r\n\
         Connection: {conn}\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes()).await?;
    if !head_only {
        stream.write_all(body.as_bytes()).await?;
    }
    stream.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn tmp() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "netbootd-http-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn split_response(resp: &[u8]) -> (String, Vec<u8>) {
        let sep = b"\r\n\r\n";
        let pos = resp
            .windows(sep.len())
            .position(|w| w == sep)
            .expect("response has a header terminator");
        let head = String::from_utf8_lossy(&resp[..pos]).into_owned();
        let body = resp[pos + sep.len()..].to_vec();
        (head, body)
    }

    fn header_value<'a>(head: &'a str, name: &str) -> Option<&'a str> {
        head.lines()
            .skip(1)
            .filter_map(|l| l.split_once(':'))
            .find(|(k, _)| k.trim().eq_ignore_ascii_case(name))
            .map(|(_, v)| v.trim())
    }

    /// Drive one request through `handle_conn` over a real loopback TCP pair and
    /// return the raw response bytes. The request must close the connection
    /// (`Connection: close`) so the client's read-to-EOF completes.
    async fn roundtrip(root: PathBuf, ctype: &str, request: &[u8]) -> Vec<u8> {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let ctype = ctype.to_string();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            handle_conn(stream, root, ctype).await.unwrap();
        });

        let mut client = TcpStream::connect(addr).await.unwrap();
        client.write_all(request).await.unwrap();
        let mut resp = Vec::new();
        client.read_to_end(&mut resp).await.unwrap();
        server.await.unwrap();
        resp
    }

    #[tokio::test]
    async fn get_serves_file_with_content_length() {
        let root = tmp();
        let body = vec![0xABu8; 4096];
        fs::write(root.join("boot.efi"), &body).unwrap();

        let resp = roundtrip(
            root.clone(),
            "application/octet-stream",
            b"GET /boot.efi HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        )
        .await;
        let (head, got) = split_response(&resp);

        assert!(head.starts_with("HTTP/1.1 200 OK"), "head: {head}");
        assert_eq!(header_value(&head, "Content-Length"), Some("4096"));
        assert_eq!(
            header_value(&head, "Content-Type"),
            Some("application/octet-stream")
        );
        assert_eq!(got, body, "body bytes match the file");
        fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn head_returns_length_without_body() {
        let root = tmp();
        fs::write(root.join("boot.efi"), vec![0u8; 1234]).unwrap();

        let resp = roundtrip(
            root.clone(),
            "application/octet-stream",
            b"HEAD /boot.efi HTTP/1.1\r\nConnection: close\r\n\r\n",
        )
        .await;
        let (head, body) = split_response(&resp);

        assert!(head.starts_with("HTTP/1.1 200 OK"));
        assert_eq!(header_value(&head, "Content-Length"), Some("1234"));
        assert!(body.is_empty(), "HEAD must not send a body");
        fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn missing_file_is_404() {
        let root = tmp();
        let resp = roundtrip(
            root.clone(),
            "application/octet-stream",
            b"GET /nope.efi HTTP/1.1\r\nConnection: close\r\n\r\n",
        )
        .await;
        let (head, _) = split_response(&resp);
        assert!(head.starts_with("HTTP/1.1 404"), "head: {head}");
        fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn unsupported_method_is_405() {
        let root = tmp();
        fs::write(root.join("boot.efi"), b"x").unwrap();
        let resp = roundtrip(
            root.clone(),
            "application/octet-stream",
            b"POST /boot.efi HTTP/1.1\r\nConnection: close\r\n\r\n",
        )
        .await;
        let (head, _) = split_response(&resp);
        assert!(head.starts_with("HTTP/1.1 405"), "head: {head}");
        fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn traversal_is_rejected() {
        let base = tmp();
        let served = base.join("served");
        fs::create_dir_all(&served).unwrap();
        fs::write(base.join("secret"), b"top secret").unwrap();

        let resp = roundtrip(
            served.clone(),
            "application/octet-stream",
            b"GET /../secret HTTP/1.1\r\nConnection: close\r\n\r\n",
        )
        .await;
        let (head, _) = split_response(&resp);
        assert!(
            head.starts_with("HTTP/1.1 404"),
            "traversal must 404: {head}"
        );
        fs::remove_dir_all(&base).ok();
    }

    #[tokio::test]
    async fn serves_file_in_subdirectory() {
        let root = tmp();
        let sub = root.join("grub");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("grub.cfg"), b"set timeout=0\n").unwrap();

        let resp = roundtrip(
            root.clone(),
            "application/octet-stream",
            b"GET /grub/grub.cfg?v=1 HTTP/1.1\r\nConnection: close\r\n\r\n",
        )
        .await;
        let (head, body) = split_response(&resp);
        assert!(head.starts_with("HTTP/1.1 200 OK"), "head: {head}");
        assert_eq!(
            body, b"set timeout=0\n",
            "query string stripped, file served"
        );
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn percent_decode_handles_escapes_and_literals() {
        assert_eq!(percent_decode("a%20b"), "a b");
        assert_eq!(percent_decode("plain"), "plain");
        // A trailing, incomplete escape is passed through literally.
        assert_eq!(percent_decode("end%2"), "end%2");
    }
}
