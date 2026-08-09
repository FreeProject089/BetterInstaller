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
        Err(Error::Other(format!(
            "refusing non-HTTPS update URL: {url}"
        )))
    }
}

fn client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(concat!("BetterInstaller/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(60))
        // `require_https` only checks the URL we were GIVEN. Without this, reqwest happily
        // follows a 302 from https to plaintext http — its default redirect policy is
        // `limited(10)` with `https_only: false` (redirect.rs), and the scheme check on the
        // next hop is gated on exactly this flag. A hostile or compromised host could
        // therefore serve the update MANIFEST in the clear, and the manifest is what dictates
        // the version and the download URL. The package stays Ed25519-signed, so the worst
        // case is a downgrade rather than code execution — but a downgrade to a known-bad
        // release is not a defence worth relying on.
        .https_only(true)
        .redirect(reqwest::redirect::Policy::limited(5))
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
        return Err(Error::Other(format!(
            "response body exceeds size limit: {url}"
        )));
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

#[cfg(test)]
mod tests {
    use super::require_https;

    #[test]
    fn https_is_required_for_update_transport() {
        // Accepted: https, any case, with surrounding whitespace (we trim).
        for ok in [
            "https://example.com/update.json",
            "HTTPS://EXAMPLE.COM/x",
            "  https://example.com/x  ",
        ] {
            assert!(require_https(ok).is_ok(), "should accept: {ok:?}");
        }
        // Refused: plaintext http, other schemes, scheme-relative and bare paths — any of
        // which would let a network attacker dictate the version + download URL.
        for bad in [
            "http://example.com/update.json",
            "ftp://example.com/x",
            "file:///etc/passwd",
            "//example.com/x",
            "example.com/x",
            "",
            "httpsx://example.com", // must be exactly the https:// prefix
        ] {
            assert!(require_https(bad).is_err(), "should refuse: {bad:?}");
        }
    }

    // Defence in depth, and the reason it is needed: `require_https` inspects the URL we were
    // handed, and nothing else. A 302 to plaintext http happens AFTER that check, so the
    // guarantee has to be restated on the client itself.
    //
    // Asserting the client refuses a plain-http request is the closest thing to that redirect
    // reachable without standing up a TLS server: both refusals come from the same
    // `https_only` flag, so if this passes, a downgrade redirect is refused too — and if
    // someone removes the flag, this test goes red rather than the failure staying invisible
    // until a hostile host exercises it.
    #[test]
    fn the_client_itself_refuses_plaintext_http() {
        let c = super::client().expect("client builds");
        let err = c
            .get("http://example.com/update.json")
            .send()
            .expect_err("https_only must refuse a plaintext request");
        assert!(
            err.is_builder() || err.is_request(),
            "expected a scheme refusal, got: {err}"
        );
    }
}
