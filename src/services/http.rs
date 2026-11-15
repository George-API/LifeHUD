//! HTTPS through Windows' own TLS stack (SChannel) and certificate store, shared by all services.

use std::sync::OnceLock;
use std::time::Duration;

use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

const LIMIT: u64 = 4 * 1024 * 1024;
const USER_AGENT: &str = concat!("LifeHUD/", env!("CARGO_PKG_VERSION"), " (+https://github.com/George-API/LifeHUD)");

/// The one TLS setup every agent uses: SChannel, verified against the Windows certificate store.
fn tls_config() -> TlsConfig {
    TlsConfig::builder().provider(TlsProvider::NativeTls).root_certs(RootCerts::PlatformVerifier).build()
}

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .tls_config(tls_config())
            .timeout_global(Some(Duration::from_secs(15)))
            .user_agent(USER_AGENT)
            .build()
            .into()
    })
}

/// For long streamed responses (AI chat): no overall deadline, generous per-phase timeouts, and
/// non-2xx statuses returned as responses so callers can read the error body.
pub fn streaming_agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .tls_config(tls_config())
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(15)))
            // Models may think for a while before the first byte; long answers stream for minutes.
            .timeout_recv_response(Some(Duration::from_secs(180)))
            .timeout_recv_body(Some(Duration::from_secs(600)))
            .user_agent(USER_AGENT)
            .build()
            .into()
    })
}

pub fn get(url: &str) -> Result<String, ureq::Error> {
    agent().get(url).call()?.body_mut().with_config().limit(LIMIT).read_to_string()
}

/// A GET with extra request headers (e.g. `Accept-Language` for pages that localise their markup).
pub fn get_with(url: &str, headers: &[(&str, &str)]) -> Result<String, ureq::Error> {
    let mut req = agent().get(url);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    req.call()?.body_mut().with_config().limit(LIMIT).read_to_string()
}

pub fn get_bytes(url: &str) -> Result<Vec<u8>, ureq::Error> {
    agent().get(url).call()?.body_mut().with_config().limit(LIMIT).read_to_vec()
}

/// A conditional GET: `Ok(None)` when the server says our copy (from `last_modified`) is current.
/// Returns the body and its new `Last-Modified` value.
pub fn get_if_modified(
    url: &str,
    last_modified: Option<&str>,
) -> Result<Option<(String, Option<String>)>, ureq::Error> {
    let mut req = agent().get(url);
    if let Some(lm) = last_modified {
        req = req.header("If-Modified-Since", lm);
    }
    let mut resp = req.call()?;
    if resp.status() == 304 {
        return Ok(None);
    }
    let lm = resp.headers().get("last-modified").and_then(|v| v.to_str().ok()).map(str::to_string);
    let body = resp.body_mut().with_config().limit(LIMIT).read_to_string()?;
    Ok(Some((body, lm)))
}

/// A GET answer that keeps the status and the rate-limit hint instead of turning 4xx/5xx into an
/// error (Reddit's anonymous feeds allow about one request a minute and say when the window resets).
pub struct Reply {
    pub status: u16,
    /// The body, for 200 responses only.
    pub body: String,
    /// Requests left in the current window, if the server says.
    pub remaining: Option<f64>,
    /// Seconds until the window resets (`x-ratelimit-reset`, else `Retry-After`).
    pub reset_secs: Option<u64>,
}

pub fn get_reply(url: &str) -> Result<Reply, ureq::Error> {
    let mut resp = agent().get(url).config().http_status_as_error(false).build().call()?;
    let status = resp.status().as_u16();
    let header = |name: &str| resp.headers().get(name).and_then(|v| v.to_str().ok()).map(|s| s.trim().to_string());
    let number = |name: &str| header(name).and_then(|s| s.parse::<f64>().ok()).filter(|v| v.is_finite() && *v >= 0.0);
    let remaining = number("x-ratelimit-remaining");
    let reset_secs = number("x-ratelimit-reset").or_else(|| number("retry-after")).map(|s| s.ceil() as u64);
    let body = if status == 200 { resp.body_mut().with_config().limit(LIMIT).read_to_string()? } else { String::new() };
    Ok(Reply { status, body, remaining, reset_secs })
}

/// The HTTP status behind an error, if the server answered at all.
pub fn status(e: &ureq::Error) -> Option<u16> {
    match e {
        ureq::Error::StatusCode(code) => Some(*code),
        _ => None,
    }
}

/// Percent-encodes a query-string value.
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn encodes_query_values() {
        assert_eq!(super::encode("Black Doves"), "Black%20Doves");
        assert_eq!(super::encode("^GSPC"), "%5EGSPC");
        assert_eq!(super::encode("Amélie & co"), "Am%C3%A9lie%20%26%20co");
    }
}
