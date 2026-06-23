//! Minimal blocking HTTP for update checks/downloads (rustls, no native-tls).

use crate::error::{Error, Result};

fn client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(concat!("BetterInstaller/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| Error::Other(format!("http client: {e}")))
}

/// GET a URL as text (e.g. an update manifest JSON).
pub fn fetch_text(url: &str) -> Result<String> {
    let resp = client()?
        .get(url)
        .send()
        .map_err(|e| Error::Other(format!("GET {url}: {e}")))?;
    if !resp.status().is_success() {
        return Err(Error::Other(format!("HTTP {} for {url}", resp.status())));
    }
    resp.text()
        .map_err(|e| Error::Other(format!("read body: {e}")))
}

/// GET a URL as bytes (e.g. a `.bpkg` or a delta patch).
pub fn download(url: &str) -> Result<Vec<u8>> {
    let resp = client()?
        .get(url)
        .send()
        .map_err(|e| Error::Other(format!("GET {url}: {e}")))?;
    if !resp.status().is_success() {
        return Err(Error::Other(format!("HTTP {} for {url}", resp.status())));
    }
    Ok(resp
        .bytes()
        .map_err(|e| Error::Other(format!("read body: {e}")))?
        .to_vec())
}
