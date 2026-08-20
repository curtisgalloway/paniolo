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

//! Intel AMT WS-Management client (SOAP over HTTP, HTTP Digest auth).
//!
//! Speaks just enough WS-Man for power control: a `transfer/Get` of
//! `CIM_AssociatedPowerManagementService` (power-state readback) and the
//! `CIM_PowerManagementService.RequestPowerStateChange` invoke, over plain
//! HTTP on port 16992. AMT 11+ advertises Digest-only authentication
//! (established on this bench: the supported-auth list is the single byte
//! 0x04, and plaintext is rejected), so the RFC 2617 handshake is implemented
//! here directly on top of ureq: POST unauthenticated, answer the 401
//! challenge, retry once. The password never appears on the command line —
//! it comes from the `AMT_PASSWORD` environment variable.

use std::cell::RefCell;
use std::fmt::Write as _;
use std::process;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use md5::{Digest as _, Md5};

/// Per-request timeout. The ME answers fast (it is firmware, not the OS);
/// this mostly bounds how long a wrong/dead address stalls a hook.
const CALL_TIMEOUT: Duration = Duration::from_secs(10);

const XMLNS_ENV: &str = "http://www.w3.org/2003/05/soap-envelope";
const XMLNS_ADDR: &str = "http://schemas.xmlsoap.org/ws/2004/08/addressing";
const XMLNS_WSMAN: &str = "http://schemas.dmtf.org/wbem/wsman/1/wsman.xsd";
const ANON: &str = "http://schemas.xmlsoap.org/ws/2004/08/addressing/role/anonymous";
const TRANSFER_GET: &str = "http://schemas.xmlsoap.org/ws/2004/09/transfer/Get";
const CIM: &str = "http://schemas.dmtf.org/wbem/wscim/1/cim-schema/2/";

/// `CIM_PowerManagementService.RequestPowerStateChange` PowerState values
/// (DMTF CIM PowerState value map). AMT implements 2/5/8/10; this helper
/// commands 2 and 8 and builds `cycle` as off → delay → on so the off-hold
/// duration is controllable (the fixed CIM power-cycle state 5 is not).
pub const PS_ON: u16 = 2;
pub const PS_OFF_SOFT: u16 = 8;

/// Human name for a `PowerState` value (DMTF CIM value map), for `status`
/// output and error messages.
pub fn power_state_name(ps: u16) -> &'static str {
    match ps {
        1 => "Other",
        2 => "On",
        3 => "Sleep - Light",
        4 => "Sleep - Deep",
        5 => "Power Cycle (Off - Soft)",
        6 => "Off - Hard",
        7 => "Hibernate (Off - Soft)",
        8 => "Off - Soft",
        9 => "Power Cycle (Off - Hard)",
        10 => "Master Bus Reset",
        11 => "Diagnostic Interrupt (NMI)",
        12 => "Off - Soft Graceful",
        13 => "Off - Hard Graceful",
        14 => "Master Bus Reset Graceful",
        15 => "Power Cycle (Off - Soft Graceful)",
        16 => "Power Cycle (Off - Hard Graceful)",
        _ => "unknown",
    }
}

/// Marker context on errors that may clear on their own. Observed on this
/// bench: the machine's NIC drops link for a few seconds around host power
/// transitions (PHY renegotiation as the host side comes up or down), which
/// surfaces as "no route to host" mid-command. Callers use [`is_transient`]
/// to keep polling through that window instead of failing the hook.
#[derive(Debug)]
pub struct Transient;

impl std::fmt::Display for Transient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("transient AMT transport error")
    }
}

/// Whether an error carries the [`Transient`] marker (a transport-level
/// failure worth retrying, as opposed to an auth failure or a CIM error).
pub fn is_transient(err: &anyhow::Error) -> bool {
    err.downcast_ref::<Transient>().is_some()
}

/// An AMT machine addressed over WS-Man on the plain (non-TLS) port.
pub struct Client {
    url: String,
    /// The request path, as it appears in the Digest `uri` parameter.
    path: &'static str,
    user: String,
    password: String,
    /// The `Server:` header seen on the digest challenge (e.g. "Intel(R)
    /// Active Management Technology 12.0.24.1314"), captured for `status`.
    server: RefCell<Option<String>>,
}

impl Client {
    /// Build a client from a user-supplied address: a bare IPv4 address or
    /// hostname, optionally with a port (default 16992). An `https://`
    /// address is rejected — TLS-provisioned AMT (port 16993) is not
    /// supported by this helper.
    pub fn new(device: &str, user: &str, password: &str) -> Result<Self> {
        let d = device.trim().trim_end_matches('/');
        if d.starts_with("https://") {
            bail!(
                "TLS (https / port 16993) is not supported — \
                 this helper speaks plain WS-Man on port 16992"
            );
        }
        let d = d.strip_prefix("http://").unwrap_or(d);
        if d.is_empty() {
            bail!("empty device address");
        }
        let hostport = if d.contains(':') {
            d.to_string()
        } else {
            format!("{d}:16992")
        };
        Ok(Client {
            url: format!("http://{hostport}/wsman"),
            path: "/wsman",
            user: user.to_string(),
            password: password.to_string(),
            server: RefCell::new(None),
        })
    }

    /// The AMT firmware identity from the HTTP `Server:` header, if a
    /// request has been made.
    pub fn server_ident(&self) -> Option<String> {
        self.server.borrow().clone()
    }

    /// Current system power state: `transfer/Get` on
    /// `CIM_AssociatedPowerManagementService`, whose `PowerState` property is
    /// the ME's live view of the host (readable in any host state — the ME
    /// runs even when the host is off).
    pub fn power_state(&self) -> Result<u16> {
        let uri = format!("{CIM}CIM_AssociatedPowerManagementService");
        let body = self.post(&envelope(&self.url, TRANSFER_GET, &uri, "", ""))?;
        let text = xml_text(&body, "PowerState")
            .ok_or_else(|| anyhow!("no PowerState in WS-Man response"))?;
        text.parse()
            .with_context(|| format!("unparseable PowerState {text:?}"))
    }

    /// Invoke `CIM_PowerManagementService.RequestPowerStateChange` with the
    /// given `PowerState`, addressed at the managed host system. Errors on a
    /// non-zero CIM ReturnValue.
    pub fn request_power_state(&self, state: u16) -> Result<()> {
        let uri = format!("{CIM}CIM_PowerManagementService");
        let action = format!("{uri}/RequestPowerStateChange");
        // The AMT instance keys for the power service and the managed system
        // (fixed values, per the Intel AMT SDK).
        let selectors = "\n  <w:SelectorSet>\
             \n   <w:Selector Name=\"CreationClassName\">CIM_PowerManagementService</w:Selector>\
             \n   <w:Selector Name=\"Name\">Intel(r) AMT Power Management Service</w:Selector>\
             \n   <w:Selector Name=\"SystemCreationClassName\">CIM_ComputerSystem</w:Selector>\
             \n   <w:Selector Name=\"SystemName\">Intel(r) AMT</w:Selector>\
             \n  </w:SelectorSet>";
        let body = format!(
            "<r:RequestPowerStateChange_INPUT xmlns:r=\"{uri}\">\
             <r:PowerState>{state}</r:PowerState>\
             <r:ManagedElement>\
             <a:Address>{ANON}</a:Address>\
             <a:ReferenceParameters>\
             <w:ResourceURI>{CIM}CIM_ComputerSystem</w:ResourceURI>\
             <w:SelectorSet>\
             <w:Selector Name=\"CreationClassName\">CIM_ComputerSystem</w:Selector>\
             <w:Selector Name=\"Name\">ManagedSystem</w:Selector>\
             </w:SelectorSet>\
             </a:ReferenceParameters>\
             </r:ManagedElement>\
             </r:RequestPowerStateChange_INPUT>"
        );
        let resp = self.post(&envelope(&self.url, &action, &uri, selectors, &body))?;
        let rv = xml_text(&resp, "ReturnValue")
            .ok_or_else(|| anyhow!("no ReturnValue in RequestPowerStateChange response"))?;
        match rv {
            "0" => Ok(()),
            code => bail!(
                "RequestPowerStateChange({state}) failed: ReturnValue {code} ({})",
                return_value_name(code)
            ),
        }
    }

    /// The one place every WS-Man request is issued, including the HTTP
    /// Digest handshake: POST unauthenticated, answer the 401 challenge,
    /// retry once. Returns the response body.
    fn post(&self, xml: &str) -> Result<String> {
        let req = || {
            ureq::post(&self.url)
                .set("Content-Type", "application/soap+xml;charset=UTF-8")
                .timeout(CALL_TIMEOUT)
        };
        match req().send_string(xml) {
            Ok(resp) => resp.into_string().context("reading WS-Man response"),
            Err(ureq::Error::Status(401, resp)) => {
                *self.server.borrow_mut() = resp.header("Server").map(str::to_string);
                let header = resp
                    .header("WWW-Authenticate")
                    .ok_or_else(|| anyhow!("HTTP 401 without a WWW-Authenticate challenge"))?;
                let ch = parse_challenge(header)?;
                let authz = self.authorization(&ch, "POST");
                match req().set("Authorization", &authz).send_string(xml) {
                    Ok(resp) => resp.into_string().context("reading WS-Man response"),
                    Err(ureq::Error::Status(401, _)) => bail!(
                        "authentication failed (HTTP 401 after digest response) — \
                         check AMT_PASSWORD and the username"
                    ),
                    Err(ureq::Error::Status(code, resp)) => Err(http_error(code, resp)),
                    Err(ureq::Error::Transport(t)) => {
                        Err(anyhow!("cannot reach AMT at {}: {t}", self.url).context(Transient))
                    }
                }
            }
            Err(ureq::Error::Status(code, resp)) => Err(http_error(code, resp)),
            Err(ureq::Error::Transport(t)) => Err(anyhow!(
                "cannot reach AMT at {}: {t} \
                 (a TLS-provisioned machine serves WS-Man only on port 16993, \
                 which this helper does not support)",
                self.url
            )
            .context(Transient)),
        }
    }

    /// Build the `Authorization:` header value answering a digest challenge.
    fn authorization(&self, ch: &Challenge, method: &str) -> String {
        let nc = "00000001";
        let cnonce = cnonce();
        let response = digest_response(
            &self.user,
            &ch.realm,
            &self.password,
            method,
            self.path,
            &ch.nonce,
            ch.qop_auth.then_some((nc, cnonce.as_str())),
        );
        let mut h = format!(
            "Digest username=\"{}\", realm=\"{}\", nonce=\"{}\", uri=\"{}\"",
            self.user, ch.realm, ch.nonce, self.path
        );
        if ch.qop_auth {
            let _ = write!(h, ", qop=auth, nc={nc}, cnonce=\"{cnonce}\"");
        }
        let _ = write!(h, ", response=\"{response}\"");
        if let Some(o) = &ch.opaque {
            let _ = write!(h, ", opaque=\"{o}\"");
        }
        h
    }
}

/// CIM `RequestPowerStateChange` ReturnValue names (DMTF value map subset).
fn return_value_name(code: &str) -> &'static str {
    match code {
        "1" => "Not Supported",
        "2" => "Unknown or Unspecified Error",
        "3" => "Cannot complete within Timeout Period",
        "4" => "Failed",
        "5" => "Invalid Parameter",
        "6" => "In Use",
        "4096" => "Job Started",
        "4097" => "Invalid State Transition",
        "4098" => "Use of Timeout Parameter Not Supported",
        "4099" => "Busy",
        _ => "unrecognized code",
    }
}

/// Map a non-2xx WS-Man response to an error, surfacing the SOAP fault
/// reason text when the body carries one.
fn http_error(code: u16, resp: ureq::Response) -> anyhow::Error {
    let body = resp.into_string().unwrap_or_default();
    match xml_text(&body, "Text").filter(|t| !t.is_empty()) {
        Some(reason) => anyhow!("AMT returned HTTP {code}: {reason}"),
        None => anyhow!("AMT returned HTTP {code}"),
    }
}

/// A WS-Man SOAP envelope. `to` is the endpoint URL; `selectors` is header
/// XML (the `SelectorSet` naming the service instance, or empty); `body` is
/// the `s:Body` content.
fn envelope(to: &str, action: &str, resource_uri: &str, selectors: &str, body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
         \n<s:Envelope xmlns:s=\"{XMLNS_ENV}\" xmlns:a=\"{XMLNS_ADDR}\" xmlns:w=\"{XMLNS_WSMAN}\">\
         \n <s:Header>\
         \n  <a:Action s:mustUnderstand=\"true\">{action}</a:Action>\
         \n  <a:To s:mustUnderstand=\"true\">{to}</a:To>\
         \n  <w:ResourceURI s:mustUnderstand=\"true\">{resource_uri}</w:ResourceURI>\
         \n  <a:MessageID s:mustUnderstand=\"true\">{}</a:MessageID>\
         \n  <a:ReplyTo><a:Address>{ANON}</a:Address></a:ReplyTo>\
         \n  <w:OperationTimeout>PT60.000S</w:OperationTimeout>{selectors}\
         \n </s:Header>\
         \n <s:Body>{body}</s:Body>\
         \n</s:Envelope>",
        message_id()
    )
}

/// A unique-enough WS-Addressing MessageID (uuid-shaped, from the clock and
/// pid — no RNG dependency; uniqueness only needs to hold per conversation).
fn message_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let hi = (nanos >> 64) as u64;
    let lo = (nanos as u64) ^ ((process::id() as u64) << 48);
    format!(
        "uuid:{:08x}-{:04x}-4{:03x}-8{:03x}-{:012x}",
        lo as u32,
        (lo >> 32) as u16,
        (lo >> 48) as u16 & 0xfff,
        hi as u16 & 0xfff,
        (hi >> 12) & 0xffff_ffff_ffff
    )
}

/// The fields of a `WWW-Authenticate: Digest` challenge this client uses.
struct Challenge {
    realm: String,
    nonce: String,
    qop_auth: bool,
    opaque: Option<String>,
}

/// Parse a `WWW-Authenticate: Digest` header value: comma-separated
/// `key=value` pairs, values optionally quoted (AMT emits e.g.
/// `Digest realm="Digest:4BB9...", nonce="...",stale="false",qop="auth"`).
fn parse_challenge(header: &str) -> Result<Challenge> {
    let rest = header
        .strip_prefix("Digest")
        .ok_or_else(|| anyhow!("not a Digest challenge: {header:?}"))?;
    let mut realm = None;
    let mut nonce = None;
    let mut qop = None;
    let mut opaque = None;
    for (key, value) in parse_kv_list(rest) {
        match key.as_str() {
            "realm" => realm = Some(value),
            "nonce" => nonce = Some(value),
            "qop" => qop = Some(value),
            "opaque" => opaque = Some(value),
            _ => {}
        }
    }
    Ok(Challenge {
        realm: realm.ok_or_else(|| anyhow!("digest challenge without realm: {header:?}"))?,
        nonce: nonce.ok_or_else(|| anyhow!("digest challenge without nonce: {header:?}"))?,
        qop_auth: qop
            .as_deref()
            .is_some_and(|q| q.split(',').any(|v| v.trim() == "auth")),
        opaque,
    })
}

/// Split `key=value, key="quoted value", ...` into pairs, honoring quotes.
fn parse_kv_list(s: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let mut chars = s.chars().peekable();
    loop {
        while matches!(chars.peek(), Some(' ' | '\t' | ',')) {
            chars.next();
        }
        let mut key = String::new();
        while let Some(&c) = chars.peek() {
            if c == '=' {
                break;
            }
            key.push(c);
            chars.next();
        }
        if chars.next().is_none() {
            break; // no '=': end of input
        }
        let mut value = String::new();
        if chars.peek() == Some(&'"') {
            chars.next();
            for c in chars.by_ref() {
                if c == '"' {
                    break;
                }
                value.push(c);
            }
        } else {
            while let Some(&c) = chars.peek() {
                if c == ',' {
                    break;
                }
                value.push(c);
                chars.next();
            }
        }
        pairs.push((key.trim().to_ascii_lowercase(), value));
    }
    pairs
}

/// RFC 2617 digest response. `qop` carries `(nc, cnonce)` for `qop=auth`;
/// `None` selects the legacy (RFC 2069) form.
fn digest_response(
    user: &str,
    realm: &str,
    password: &str,
    method: &str,
    uri: &str,
    nonce: &str,
    qop: Option<(&str, &str)>,
) -> String {
    let ha1 = md5_hex(&format!("{user}:{realm}:{password}"));
    let ha2 = md5_hex(&format!("{method}:{uri}"));
    match qop {
        Some((nc, cnonce)) => md5_hex(&format!("{ha1}:{nonce}:{nc}:{cnonce}:auth:{ha2}")),
        None => md5_hex(&format!("{ha1}:{nonce}:{ha2}")),
    }
}

fn md5_hex(input: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(input.as_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(32);
    for b in digest {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// A per-invocation client nonce: clock + pid, hex. Uniqueness, not
/// unpredictability, is what qop=auth needs from the client side here.
fn cnonce() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{:016x}", (nanos as u64) ^ ((process::id() as u64) << 32))
}

/// Text content of the first element whose *local* name matches exactly,
/// namespace prefix ignored (`<g:PowerState>2</g:PowerState>` → `"2"`).
/// Exact-match on the local name so `RequestedPowerState` or
/// `AvailableRequestedPowerStates` never satisfy a `PowerState` lookup.
/// Good enough for AMT's scalar response fields; not a general XML parser
/// (attribute values containing `>` would confuse it).
fn xml_text<'a>(xml: &'a str, local: &str) -> Option<&'a str> {
    let bytes = xml.as_bytes();
    let mut i = 0;
    while let Some(off) = xml[i..].find('<') {
        let start = i + off + 1;
        if start >= xml.len() {
            return None;
        }
        if matches!(bytes[start], b'/' | b'?' | b'!') {
            i = start;
            continue;
        }
        let mut j = start;
        while j < xml.len() && !matches!(bytes[j], b' ' | b'\t' | b'\r' | b'\n' | b'>' | b'/') {
            j += 1;
        }
        let name = &xml[start..j];
        let localname = name.rsplit(':').next().unwrap_or(name);
        let gt = xml[j..].find('>')? + j;
        if localname == local {
            if bytes[gt - 1] == b'/' {
                return Some(""); // self-closed element
            }
            let end = xml[gt + 1..].find('<')? + gt + 1;
            return Some(xml[gt + 1..end].trim());
        }
        i = gt + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_normalization() {
        assert_eq!(
            Client::new("10.0.0.5", "admin", "pw").unwrap().url,
            "http://10.0.0.5:16992/wsman"
        );
        assert_eq!(
            Client::new("amt-host:16993", "admin", "pw").unwrap().url,
            "http://amt-host:16993/wsman"
        );
        assert_eq!(
            Client::new("http://10.0.0.5/", "admin", "pw").unwrap().url,
            "http://10.0.0.5:16992/wsman"
        );
        assert!(Client::new("https://10.0.0.5", "admin", "pw").is_err());
        assert!(Client::new("", "admin", "pw").is_err());
    }

    #[test]
    fn parses_amt_challenge() {
        // Verbatim from an OptiPlex 7060 (AMT 12.0.24.1314) on this bench —
        // note the missing spaces after some commas.
        let h = "Digest realm=\"Digest:4BB90000000000000000000000000000\", \
                 nonce=\"zicaAAAAAAAAAAAAa8tMnB1xbyLK5UFG\",stale=\"false\",qop=\"auth\"";
        let ch = parse_challenge(h).unwrap();
        assert_eq!(ch.realm, "Digest:4BB90000000000000000000000000000");
        assert_eq!(ch.nonce, "zicaAAAAAAAAAAAAa8tMnB1xbyLK5UFG");
        assert!(ch.qop_auth);
        assert!(ch.opaque.is_none());
    }

    #[test]
    fn challenge_requires_digest_scheme() {
        assert!(parse_challenge("Basic realm=\"x\"").is_err());
    }

    #[test]
    fn digest_rfc2617_vector() {
        // The worked example from RFC 2617 §3.5.
        let resp = digest_response(
            "Mufasa",
            "testrealm@host.com",
            "Circle Of Life",
            "GET",
            "/dir/index.html",
            "dcd98b7102dd2f0e8b11d0f600bfb0c093",
            Some(("00000001", "0a4f113b")),
        );
        assert_eq!(resp, "6629fae49393a05397450978507c4ef1");
    }

    #[test]
    fn xml_text_exact_local_name() {
        let body = "<g:CIM_AssociatedPowerManagementService \
                    xmlns:g=\"urn:x\"><g:AvailableRequestedPowerStates>2\
                    </g:AvailableRequestedPowerStates>\
                    <g:AvailableRequestedPowerStates>8\
                    </g:AvailableRequestedPowerStates>\
                    <g:PowerState>2</g:PowerState>\
                    <g:RequestedPowerState>8</g:RequestedPowerState>\
                    </g:CIM_AssociatedPowerManagementService>";
        assert_eq!(xml_text(body, "PowerState"), Some("2"));
        assert_eq!(xml_text(body, "RequestedPowerState"), Some("8"));
        assert_eq!(xml_text(body, "Missing"), None);
    }

    #[test]
    fn xml_text_return_value_and_fault() {
        let invoke = "<a:Body><g:RequestPowerStateChange_OUTPUT xmlns:g=\"urn:x\">\
                      <g:ReturnValue>0</g:ReturnValue>\
                      </g:RequestPowerStateChange_OUTPUT></a:Body>";
        assert_eq!(xml_text(invoke, "ReturnValue"), Some("0"));
        let fault = "<s:Fault><s:Reason>\
                     <s:Text xml:lang=\"en-US\">The sender was not authorized</s:Text>\
                     </s:Reason></s:Fault>";
        assert_eq!(
            xml_text(fault, "Text"),
            Some("The sender was not authorized")
        );
        let selfclosed = "<g:PowerState/>";
        assert_eq!(xml_text(selfclosed, "PowerState"), Some(""));
    }

    #[test]
    fn envelope_shape() {
        let e = envelope(
            "http://10.0.0.5:16992/wsman",
            "http://schemas.xmlsoap.org/ws/2004/09/transfer/Get",
            "urn:resource",
            "",
            "<x/>",
        );
        assert!(e.contains("transfer/Get</a:Action>"));
        assert!(e.contains("<a:To s:mustUnderstand=\"true\">http://10.0.0.5:16992/wsman"));
        assert!(e.contains("<w:ResourceURI s:mustUnderstand=\"true\">urn:resource"));
        assert!(e.contains("<a:MessageID s:mustUnderstand=\"true\">uuid:"));
        assert!(e.contains("<s:Body><x/></s:Body>"));
    }

    #[test]
    fn message_ids_are_unique() {
        assert_ne!(message_id(), message_id());
    }
}
