//! Minimal blocking HTTP for update checks/downloads (rustls, no native-tls).

use std::io::Read;

use crate::error::{Error, Result};

/// Hard ceiling on a downloaded package/patch/manifest body, to bound memory use on a
/// hostile or misbehaving server (the payload is also signature-verified elsewhere).
const MAX_BODY: u64 = 1024 * 1024 * 1024; // 1 GiB

/// Require HTTPS for every update URL. The package payload is Ed25519-signed, but the
/// update *manifest* (which dictates the version + download URL) is otherwise
/// unauthenticated — over plaintext HTTP it could be tampered with or downgraded, so
/// we refuse non-TLS transport outright.
fn require_https(url: &str) -> Result<()> {
    let u = url.trim();
    if u.len() >= 8 && u[..8].eq_ignore_ascii_case("https://") {
        Ok(())
    } else {
        Err(Error::Other(format!("refusing non-HTTPS update URL: {url}")))
    }
}

fn client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(concat!("BetterInstaller/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| Error::Other(format!("http client: {e}")))
}

/// Read a response body with a hard size cap (streams, never buffers past the limit).
fn read_capped(resp: reqwest::blocking::Response, url: &str) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    resp.take(MAX_BODY + 1)
        .read_to_end(&mut buf)
        .map_err(|e| Error::Other(format!("read body: {e}")))?;
    if buf.len() as u64 > MAX_BODY {
        return Err(Error::Other(format!("response body exceeds size limit: {url}")));
    }
    Ok(buf)
}

/// GET a URL as text (e.g. an update manifest JSON). HTTPS-only, size-capped.
pub fn fetch_text(url: &str) -> Result<String> {
    require_https(url)?;
    let resp = client()?
        .get(url)
        .send()
        .map_err(|e| Error::Other(format!("GET {url}: {e}")))?;
    if !resp.status().is_success() {
        return Err(Error::Other(format!("HTTP {} for {url}", resp.status())));
    }
    let bytes = read_capped(resp, url)?;
    String::from_utf8(bytes).map_err(|e| Error::Other(format!("non-UTF8 body from {url}: {e}")))
}

/// GET a URL as bytes (e.g. a `.bpkg` or a delta patch). HTTPS-only, size-capped.
pub fn download(url: &str) -> Result<Vec<u8>> {
    require_https(url)?;
    let resp = client()?
        .get(url)
        .send()
        .map_err(|e| Error::Other(format!("GET {url}: {e}")))?;
    if !resp.status().is_success() {
        return Err(Error::Other(format!("HTTP {} for {url}", resp.status())));
    }
    read_capped(resp, url)
}
