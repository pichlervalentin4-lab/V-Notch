//! Blocking HTTP used by the background threads. Always called off the UI thread.

use serde_json::Value;
use std::io::Read;
use std::time::Duration;
use ureq::Agent;

const USER_AGENT: &str = concat!("V-Notch-Rust/", env!("CARGO_PKG_VERSION"));
const MAX_BODY: u64 = 16 * 1024 * 1024;

pub fn agent(timeout: Duration) -> Agent {
    let builder = Agent::config_builder().timeout_global(Some(timeout)).http_status_as_error(false);
    #[cfg(feature = "tls")]
    let builder = builder.tls_config(
        ureq::tls::TlsConfig::builder().provider(ureq::tls::TlsProvider::NativeTls).build(),
    );
    builder.build().into()
}

fn describe(status: u16, url: &str) -> String {
    let host = url.split('/').nth(2).unwrap_or("?");
    format!("HTTP {status} from {host}")
}

pub fn get_json(url: &str, timeout: Duration) -> Result<Value, String> {
    let bytes = get_bytes(url, timeout)?;
    serde_json::from_slice(&bytes).map_err(|e| format!("bad JSON: {e}"))
}

pub fn get_bytes(url: &str, timeout: Duration) -> Result<Vec<u8>, String> {
    let mut response = agent(timeout)
        .get(url)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(describe(status, url));
    }
    response.body_mut().with_config().limit(MAX_BODY).read_to_vec().map_err(|e| e.to_string())
}

/// Like [`get_json`] with extra headers; used where a token or key has to go along.
pub fn get_json_with(url: &str, headers: &[(&str, String)], timeout: Duration) -> Result<Value, String> {
    let mut request = agent(timeout).get(url).header("User-Agent", USER_AGENT);
    for (name, value) in headers {
        request = request.header(*name, value);
    }
    let mut response = request.call().map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(describe(status, url));
    }
    response.body_mut().read_json().map_err(|e| e.to_string())
}

/// POSTs JSON and hands back the response as a stream, for server-sent events.
pub fn post_stream(
    url: &str,
    headers: &[(&str, String)],
    body: &Value,
    timeout: Duration,
) -> Result<Box<dyn Read + Send + 'static>, (u16, String)> {
    let mut request = agent(timeout)
        .post(url)
        .header("User-Agent", USER_AGENT)
        .header("Content-Type", "application/json");
    for (name, value) in headers {
        request = request.header(*name, value);
    }
    let response = request.send_json(body).map_err(|e| (0, e.to_string()))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        let mut text = String::new();
        let mut reader = response.into_body().into_reader();
        let _ = (&mut reader).take(4096).read_to_string(&mut text);
        return Err((status, text));
    }
    Ok(Box::new(response.into_body().into_reader()))
}

pub fn post_json(url: &str, headers: &[(&str, String)], body: &Value, timeout: Duration) -> Result<Value, String> {
    let mut reader = post_stream(url, headers, body, timeout).map_err(|(status, _)| describe(status, url))?;
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    serde_json::from_slice(&bytes).map_err(|e| format!("bad JSON: {e}"))
}

/// Percent-encodes a query value.
pub fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_escapes_reserved_and_utf8() {
        assert_eq!(encode("a b&c"), "a%20b%26c");
        assert_eq!(encode("Müller"), "M%C3%BCller");
        assert_eq!(encode("safe-_.~"), "safe-_.~");
    }
}
