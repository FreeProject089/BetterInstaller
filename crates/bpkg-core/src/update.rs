//! Updates with atomic-ish rollback.
//!
//! Local apply: snapshot the install dir to a sibling `<name>.bak`, extract the
//! new package over it; on ANY error, wipe + restore from the snapshot; on
//! success, drop the snapshot. Remote: [`check_remote`] fetches an
//! [`UpdateManifest`], checks its signature and expiry ([`parse_manifest`]), and
//! [`download_and_apply`] downloads the new package — preferring a small binary
//! [delta](crate::delta) from the current version when offered — then applies it with
//! the same rollback safety net.
//!
//! # The signed manifest (card C-1)
//!
//! `update.json` decides which version is offered and where it is downloaded from. The
//! package it points at is signed; the file itself was not, so whoever controlled its host
//! (or a mirror in `manifest_urls`) could withhold updates for ever, or offer a genuine
//! release older than the latest. It now carries a signature by the SAME publisher key
//! that signs packages:
//!
//! ```json
//! {
//!   "version": "1.3.0", "url": "…", "urls": […], "notes": "…", "deltas": […],
//!   "signed": "{\"app_id\":\"com.example.app\",\"version\":\"1.3.0\",…,\"expires\":\"…\"}",
//!   "signature": "<128 hex characters>"
//! }
//! ```
//!
//! - `signed` is a JSON document carried as a STRING, and the signature is over its exact
//!   bytes: nothing has to be re-serialised the same way twice, so there is no
//!   canonicalisation to get wrong. The message is [`MANIFEST_SIG_CONTEXT`] followed by
//!   those bytes, so a manifest signature can never be replayed as a package signature
//!   (whose message starts with the `BPKG` magic) or the reverse.
//! - The signed body names the app (`app_id`), the version, the URLs, the SHA-256 of the
//!   full `.bpkg` (`sha256`), the SHA-256 of the `installer.toml` stamped into that
//!   release's setup (`config_sha256`), and `issued` / `expires` (RFC 3339, at most
//!   [`MANIFEST_MAX_VALIDITY_DAYS`] apart). A client refuses it once `expires` is past.
//! - The top-level `version` / `url` / … are a copy for engines that predate signing (they
//!   ignore the fields they do not know). An engine that verifies reads the signed body
//!   ONLY and never falls back to the copy.

use std::path::{Path, PathBuf};

use ed25519_dalek::VerifyingKey;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::package::Package;
// The one version ordering (card C-5); this module used to carry its own.
use crate::version::is_newer;

/// Prefix of every manifest signature's message. Changing the signed format means a new
/// context string, so an old signature cannot be read under new rules.
pub const MANIFEST_SIG_CONTEXT: &[u8] = b"BetterInstaller update manifest v1\n";

/// Longest `expires - issued` a client accepts. The publisher re-signs at least this
/// often; a host that stops receiving new manifests (a freeze attack, or a stalled release
/// job) stops being believed after this long.
pub const MANIFEST_MAX_VALIDITY_DAYS: i64 = 7;

/// The migration window, one engine release long. While `true`, an UNSIGNED manifest is
/// accepted — with a warning — for an install made by an engine that predates signed
/// manifests (see `accept_unsigned` in [`ManifestPolicy`]). Set it to `false` in the
/// release after the first one that signs.
pub const MIGRATION_ACCEPTS_UNSIGNED: bool = true;

/// A remote update manifest (JSON at a stable URL), or the signed body inside one.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateManifest {
    /// The app this manifest is for. Required in a signed body, so a manifest another app
    /// of the same publisher signed cannot be served in its place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    /// Latest available version, e.g. "1.2.0".
    pub version: String,
    /// URL of the full `.bpkg` for that version.
    pub url: String,
    /// Additional sources for the SAME package, tried in order after `url` when a download
    /// fails. Safe by construction: the package's signature (and, with a signed manifest,
    /// its SHA-256) is checked before the install directory is touched, so a mirror can
    /// serve a bad file but never get it applied. Without this, one unreachable host means
    /// nobody can update at all.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub urls: Vec<String>,
    /// Release notes shown to the user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Binary deltas from older versions (download a small patch instead of the full pkg).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deltas: Vec<DeltaEntry>,
    /// Lowercase hex SHA-256 of the full `.bpkg`. Required in a signed body; the download
    /// (or the package rebuilt from a delta) must match it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// Lowercase hex SHA-256 of the `installer.toml` bytes stamped into this release's
    /// setup. `bpkg verify-manifest --setup` checks a setup against it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_sha256: Option<String>,
    /// RFC 3339 time the body was signed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issued: Option<String>,
    /// RFC 3339 time after which the body is refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<String>,
    /// How this manifest was authenticated. Never read from the file.
    #[serde(skip)]
    pub trust: ManifestTrust,
}

/// How a manifest came to be believed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ManifestTrust {
    /// No key to check against (none configured): read as given, like an unsigned package.
    #[default]
    Unverified,
    /// Signature valid for the pinned key, not expired, for this app.
    Verified,
    /// Unsigned, accepted once under the migration rule. The caller must warn.
    UnsignedAccepted,
}

/// What [`parse_manifest`] requires.
#[derive(Debug, Clone)]
pub struct ManifestPolicy<'a> {
    /// The publisher key (`[security] public_key`). With a key, the manifest MUST be
    /// signed by it — except under `accept_unsigned`.
    pub key: Option<&'a VerifyingKey>,
    /// The app being updated (`[app] id`); a signed body for another app is refused.
    pub app_id: Option<&'a str>,
    /// The migration rule: accept an unsigned manifest (flagged
    /// [`ManifestTrust::UnsignedAccepted`]). Callers set it to
    /// `MIGRATION_ACCEPTS_UNSIGNED && <the install was made by a pre-signing engine>`.
    pub accept_unsigned: bool,
    /// The clock expiry is checked against.
    pub now: chrono::DateTime<chrono::Utc>,
}

impl<'a> ManifestPolicy<'a> {
    /// Strict: signed when a key is given, checked against the current time.
    pub fn new(key: Option<&'a VerifyingKey>, app_id: Option<&'a str>) -> Self {
        ManifestPolicy {
            key,
            app_id,
            accept_unsigned: false,
            now: chrono::Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeltaEntry {
    /// The version this patch upgrades *from*.
    pub from: String,
    /// URL of the bsdiff patch (old.bpkg → new.bpkg).
    pub url: String,
    /// Mirrors for that patch, same rule as [`UpdateManifest::urls`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub urls: Vec<String>,
}

/// Lowercase hex SHA-256.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    crate::sign::hex_encode(&Sha256::digest(bytes))
}

/// Read an `update.json`, applying `policy`. See the module docs for the format.
///
/// With a key: the manifest must carry `signed` + `signature`, the signature must verify,
/// and the signed body must name this app, carry the package hash, and not be expired.
/// Without a key there is nothing to verify against: the signed body is read if present
/// (its expiry still applies), else the top-level fields, flagged
/// [`ManifestTrust::Unverified`].
pub fn parse_manifest(text: &str, policy: &ManifestPolicy) -> Result<UpdateManifest> {
    let doc: serde_json::Value = serde_json::from_str(text)?;
    let signed = match doc.get("signed") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(s)) => Some(s.as_str()),
        Some(_) => {
            return Err(Error::Other(
                "update manifest: `signed` must be a string".into(),
            ))
        }
    };
    match (signed, policy.key) {
        (Some(body), Some(vk)) => {
            let mut m = verified_body(&doc, body, vk)?;
            check_signed_body(&m, policy)?;
            m.trust = ManifestTrust::Verified;
            Ok(m)
        }
        (Some(body), None) => {
            let mut m: UpdateManifest = serde_json::from_str(body)?;
            check_signed_body(&m, policy)?;
            m.trust = ManifestTrust::Unverified;
            Ok(m)
        }
        (None, Some(_)) if !policy.accept_unsigned => Err(Error::Other(
            "update manifest refused: it is not signed, and this installer pins a publisher \
             key (update.json must be produced with `bpkg update-manifest`)"
                .into(),
        )),
        (None, key) => {
            let mut m: UpdateManifest = serde_json::from_value(doc)?;
            // Only the signed body may say these; a copy at the top level proves nothing.
            m.sha256 = None;
            m.config_sha256 = None;
            m.trust = if key.is_some() {
                ManifestTrust::UnsignedAccepted
            } else {
                ManifestTrust::Unverified
            };
            Ok(m)
        }
    }
}

/// The signed body, once its signature checks out for `vk`. Nothing else is checked here.
fn verified_body(doc: &serde_json::Value, body: &str, vk: &VerifyingKey) -> Result<UpdateManifest> {
    let sig = doc
        .get("signature")
        .and_then(|s| s.as_str())
        .and_then(crate::sign::hex_decode)
        .and_then(|b| <[u8; 64]>::try_from(b).ok())
        .ok_or_else(|| {
            Error::Other("update manifest: the signature is missing or malformed".into())
        })?;
    let mut msg = MANIFEST_SIG_CONTEXT.to_vec();
    msg.extend_from_slice(body.as_bytes());
    if !crate::sign::verify_message(vk, &msg, &sig) {
        return Err(Error::Other(
            "update manifest refused: its signature does not match the publisher key".into(),
        ));
    }
    Ok(serde_json::from_str(body)?)
}

/// Re-sign an existing signed `update.json` with fresh `issued` / `expires` — the weekly job
/// that keeps a manifest from expiring between releases. The existing signature must verify
/// with `sk`'s own public key (so this never launders a manifest somebody else wrote), but
/// its expiry does not matter: a job that ran late must still be able to renew it.
pub fn resign_manifest(
    text: &str,
    sk: &ed25519_dalek::SigningKey,
    now: chrono::DateTime<chrono::Utc>,
    valid_days: i64,
) -> Result<String> {
    let doc: serde_json::Value = serde_json::from_str(text)?;
    let body = doc.get("signed").and_then(|s| s.as_str()).ok_or_else(|| {
        Error::Other("this update.json is not signed; create it with `bpkg update-manifest`".into())
    })?;
    let m = verified_body(&doc, body, &sk.verifying_key())?;
    sign_manifest(&m, sk, now, valid_days)
}

/// The rules a signed body obeys beyond its signature.
fn check_signed_body(m: &UpdateManifest, policy: &ManifestPolicy) -> Result<()> {
    let bad = |why: String| Err(Error::Other(format!("update manifest refused: {why}")));
    match (m.app_id.as_deref(), policy.app_id) {
        (None, _) => return bad("the signed part does not name the app".into()),
        (Some(got), Some(want)) if got != want => {
            return bad(format!("it is for {got:?}, not {want:?}"))
        }
        _ => {}
    }
    match m.sha256.as_deref() {
        Some(h) if h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()) => {}
        _ => return bad("the signed part carries no package SHA-256".into()),
    }
    let time = |field: &str, v: Option<&str>| {
        v.and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.with_timezone(&chrono::Utc))
            .ok_or_else(|| {
                Error::Other(format!(
                    "update manifest refused: `{field}` is missing or not an RFC 3339 time"
                ))
            })
    };
    let issued = time("issued", m.issued.as_deref())?;
    let expires = time("expires", m.expires.as_deref())?;
    if expires <= issued || expires - issued > chrono::Duration::days(MANIFEST_MAX_VALIDITY_DAYS) {
        return bad(format!(
            "it claims to be valid from {issued} to {expires}; the limit is \
             {MANIFEST_MAX_VALIDITY_DAYS} days"
        ));
    }
    if policy.now >= expires {
        return bad(format!(
            "it expired on {expires}. The publisher re-signs update.json at least every \
             {MANIFEST_MAX_VALIDITY_DAYS} days; an expired one means the host is not serving \
             a current copy (or this PC's clock is wrong)"
        ));
    }
    Ok(())
}

/// Sign `body` (the manifest to publish) for `now`, valid for `valid_days` (1 ..= 7).
/// Returns the whole `update.json`: the signed body plus the top-level copy that engines
/// predating signed manifests read.
pub fn sign_manifest(
    body: &UpdateManifest,
    sk: &ed25519_dalek::SigningKey,
    now: chrono::DateTime<chrono::Utc>,
    valid_days: i64,
) -> Result<String> {
    if !(1..=MANIFEST_MAX_VALIDITY_DAYS).contains(&valid_days) {
        return Err(Error::Other(format!(
            "a manifest is valid for 1 to {MANIFEST_MAX_VALIDITY_DAYS} days, not {valid_days}"
        )));
    }
    if body.app_id.as_deref().is_none_or(str::is_empty) || body.sha256.is_none() {
        return Err(Error::Other(
            "a signed manifest needs app_id and the package sha256".into(),
        ));
    }
    let mut body = body.clone();
    body.issued = Some(now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    body.expires = Some(
        (now + chrono::Duration::days(valid_days))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    );
    let signed = serde_json::to_string(&body)?;
    let mut msg = MANIFEST_SIG_CONTEXT.to_vec();
    msg.extend_from_slice(signed.as_bytes());
    let sig = crate::sign::sign_message(sk, &msg);

    // The copy for pre-signing engines: exactly the fields they know.
    let legacy = UpdateManifest {
        version: body.version.clone(),
        url: body.url.clone(),
        urls: body.urls.clone(),
        notes: body.notes.clone(),
        deltas: body.deltas.clone(),
        ..Default::default()
    };
    let mut doc = serde_json::to_value(&legacy)?;
    doc["signed"] = serde_json::Value::String(signed);
    doc["signature"] = serde_json::Value::String(crate::sign::hex_encode(&sig));
    Ok(serde_json::to_string_pretty(&doc)?)
}

/// Fetch the manifest at `manifest_url`; return it only if newer than `current_version`.
pub fn check_remote(
    manifest_url: &str,
    current_version: &str,
    policy: &ManifestPolicy,
) -> Result<Option<UpdateManifest>> {
    Ok(offer_if_newer(
        fetch_manifest(manifest_url, policy)?,
        current_version,
    ))
}

/// Rollback protection at the manifest: a (verified) manifest is offered only when its
/// version is newer than the installed one. An older one — a replayed manifest, or a host
/// serving a stale copy — is never offered, and [`check_offered`] refuses its package even
/// if a caller skips this.
pub fn offer_if_newer(m: UpdateManifest, current_version: &str) -> Option<UpdateManifest> {
    is_newer(&m.version, current_version).then_some(m)
}

/// Fetch + parse one manifest URL.
fn fetch_manifest(url: &str, policy: &ManifestPolicy) -> Result<UpdateManifest> {
    let text = crate::net::fetch_text(url)?;
    parse_manifest(&text, policy)
}

/// Check **several** manifest sources and return the single newest update across all of
/// them. Sources that fail to fetch/parse are skipped (one dead mirror never blocks the
/// others). Returns:
/// - `Ok(Some(m))` — the highest-version manifest found, when it's newer than `current`;
/// - `Ok(None)`    — at least one source was reachable but none is newer;
/// - `Err(_)`      — **every** source failed (so the caller can report it instead of
///   silently claiming "up to date").
///
/// Backward-compatible: pass a single URL and it behaves like [`check_remote`], so
/// multi-source is always opt-in (just list more URLs).
///
/// A source whose manifest fails `policy` (unsigned, wrong signature, expired, another
/// app) counts as a failed source.
pub fn check_remote_multi(
    urls: &[String],
    current_version: &str,
    policy: &ManifestPolicy,
) -> Result<Option<UpdateManifest>> {
    let mut best: Option<UpdateManifest> = None;
    let mut ok_count = 0u32;
    let mut last_err: Option<Error> = None;

    for url in urls.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        match fetch_manifest(url, policy) {
            Ok(m) => {
                ok_count += 1;
                // Keep the highest version seen so far across all reachable sources.
                let take = best
                    .as_ref()
                    .is_none_or(|b| is_newer(&m.version, &b.version));
                if take {
                    best = Some(m);
                }
            }
            Err(e) => last_err = Some(e),
        }
    }

    if ok_count == 0 {
        return Err(last_err.unwrap_or_else(|| Error::Other("no update source configured".into())));
    }
    // Only offer the best one if it's actually newer than what's installed.
    Ok(best.and_then(|m| offer_if_newer(m, current_version)))
}

/// Download the new package — using a binary delta from `current_version` when one
/// is offered and `current_bpkg` is available — then apply it with rollback.
///
/// `app_id`: when `Some`, the package must be THAT app (see [`check_offered`]).
pub fn download_and_apply(
    m: &UpdateManifest,
    current_version: &str,
    current_bpkg: Option<&Path>,
    install_dir: &Path,
    verify_key: Option<&VerifyingKey>,
    app_id: Option<&str>,
) -> Result<u64> {
    let new_bytes = download_update(m, current_version, current_bpkg)?;
    apply_downloaded(
        &new_bytes,
        m,
        current_version,
        install_dir,
        verify_key,
        app_id,
        |_| {},
    )
    .map(|a| a.written)
}

/// The download half of [`download_and_apply`]: a delta from `current_version` when one is
/// offered and `current_bpkg` is available, else the full package (mirrors in turn).
pub fn download_update(
    m: &UpdateManifest,
    current_version: &str,
    current_bpkg: Option<&Path>,
) -> Result<Vec<u8>> {
    Ok(
        match (
            current_bpkg,
            m.deltas.iter().find(|d| d.from == current_version),
        ) {
            (Some(old_path), Some(delta)) => {
                // Delta path: download a small patch and reconstruct the new package.
                let old = std::fs::read(old_path).map_err(|e| Error::io(old_path, e))?;
                let patch = download_any(&delta.url, &delta.urls)?;
                crate::delta::apply_delta(&old, &patch)?
            }
            _ => download_any(&m.url, &m.urls)?, // full download
        },
    )
}

/// What an update wrote.
#[derive(Debug, Clone, Default)]
pub struct Applied {
    /// Number of files written.
    pub written: u64,
    /// Their paths, relative to the install directory (package entry paths).
    pub files: Vec<String>,
}

/// The half of [`download_and_apply`] after the bytes have arrived: check the bytes are the
/// package the manifest names, stage, verify, check the package is the update that was
/// offered, run `before_apply`, apply with rollback.
///
/// `before_apply` gets the new package's (verified) manifest once every check has passed
/// and before the install directory is snapshotted or written: the GUI closes the running
/// app there (card C-2), so a download that is then refused never closes anything.
pub fn apply_downloaded(
    bytes: &[u8],
    m: &UpdateManifest,
    current_version: &str,
    install_dir: &Path,
    verify_key: Option<&VerifyingKey>,
    app_id: Option<&str>,
    before_apply: impl FnOnce(&crate::manifest::Manifest),
) -> Result<Applied> {
    // A signed manifest names the exact package: its signature covers this hash, so a
    // mirror (or a delta that rebuilds the wrong thing) cannot substitute another package,
    // not even another one the same key signed.
    if let Some(want) = m.sha256.as_deref() {
        let got = sha256_hex(bytes);
        if !got.eq_ignore_ascii_case(want) {
            return Err(Error::Other(format!(
                "update rejected: the download does not match the package the manifest \
                 names (SHA-256 {got}, expected {want})"
            )));
        }
    }
    // Private scratch dir: `apply_package_update` verifies the signature of this path and
    // then reads it again to extract, so a path a local attacker can write is a package
    // that is verified and a package that is installed (crate::tmp).
    let tmp = crate::tmp::stage("update.bpkg", bytes)?;
    let res = apply_checked(
        &tmp,
        install_dir,
        None,
        verify_key,
        |app| check_offered(app, &m.version, current_version, app_id),
        before_apply,
    );
    crate::tmp::discard(&tmp);
    res
}

/// A signature says who MADE a package, not which package it is. Everything else that
/// picks the file — the manifest's version, `url`, every mirror in `urls` — is unsigned, so
/// "the signature verifies" alone let a mirror (or whoever controls the manifest's host)
/// hand over ANY package that key ever signed:
///
/// - an older release, with the bugs that release was replaced for — a rollback, which
///   SECURITY.md lists as in scope;
/// - the release already installed, replayed forever so nothing newer is ever applied;
/// - a different app by the same publisher, installed over this one.
///
/// So the package's own manifest — the part the signature covers — must name the app being
/// updated and exactly the version that was offered, and that version must be newer than
/// what is installed.
pub fn check_offered(
    app: &crate::manifest::AppMeta,
    offered: &str,
    current_version: &str,
    app_id: Option<&str>,
) -> Result<()> {
    if let Some(id) = app_id {
        if app.id != id {
            return Err(Error::Other(format!(
                "update rejected: the package is for {:?}, not {id:?}",
                app.id
            )));
        }
    }
    if !crate::version::same_release(&app.version, offered) {
        return Err(Error::Other(format!(
            "update rejected: version {} was offered but the package is {}",
            offered, app.version
        )));
    }
    if !is_newer(&app.version, current_version) {
        return Err(Error::Other(format!(
            "update rejected: the package ({}) is not newer than the installed version ({})",
            app.version, current_version
        )));
    }
    Ok(())
}

/// Download from the primary URL, falling back to each mirror in turn.
///
/// Only the LAST error is surfaced: a caller shown "mirror 3 failed" learns nothing useful when
/// mirrors 1 and 2 also failed for the same reason (usually: no network).
fn download_any(primary: &str, mirrors: &[String]) -> Result<Vec<u8>> {
    let mut last = match crate::net::download(primary) {
        Ok(b) => return Ok(b),
        Err(e) => e,
    };
    for url in mirrors.iter().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        match crate::net::download(url) {
            Ok(b) => return Ok(b),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Apply a newer `.bpkg` over `install_dir`, rolling back on failure.
/// Returns the number of files written on success.
///
/// `verify_key`: when `Some`, the package MUST carry a valid Ed25519 signature for
/// that key or the update is refused *before* the install dir is touched (fail
/// closed). Pass the publisher key from `installer.toml`'s `security.public_key` so a
/// tampered/unsigned package from a hostile mirror can never be applied. `None`
/// preserves the legacy unverified behaviour for callers that don't pin a key.
pub fn apply_package_update(
    new_bpkg: &Path,
    install_dir: &Path,
    components: Option<&[String]>,
    verify_key: Option<&VerifyingKey>,
) -> Result<u64> {
    apply_checked(
        new_bpkg,
        install_dir,
        components,
        verify_key,
        |_| Ok(()),
        |_| {},
    )
    .map(|a| a.written)
}

/// [`apply_package_update`] with one more gate, `accept`, run on the package's own app
/// metadata AFTER the signature check (so it reads signed bytes when a key is pinned) and
/// before anything is snapshotted or written; then `before_apply`, once every check passed.
fn apply_checked(
    new_bpkg: &Path,
    install_dir: &Path,
    components: Option<&[String]>,
    verify_key: Option<&VerifyingKey>,
    accept: impl FnOnce(&crate::manifest::AppMeta) -> Result<()>,
    before_apply: impl FnOnce(&crate::manifest::Manifest),
) -> Result<Applied> {
    // Authenticity gate FIRST — refuse an unsigned/invalid package before we snapshot
    // or write anything.
    let mut pkg = Package::open(new_bpkg)?;
    if let Some(vk) = verify_key {
        if !pkg.verify_signature(vk)? {
            return Err(Error::Other(
                "update rejected: package signature is missing or invalid".into(),
            ));
        }
    }
    accept(&pkg.manifest.app)?;
    before_apply(&pkg.manifest);

    // A snapshot already there is what an interrupted update leaves behind (power cut,
    // killed process, or a rollback that could not finish — see below), and it may be the
    // only intact copy of the install. This used to delete it and snapshot the half-written
    // directory in its place.
    let backup = backup_path(install_dir);
    if backup.exists() {
        return Err(Error::Other(format!(
            "an earlier update did not finish: {} holds the install as it was before it. \
             Restore it or delete it, then update again",
            backup.display()
        )));
    }
    if let Err(e) = copy_dir(install_dir, &backup) {
        // Our own partial snapshot, from this call: nothing else is in it.
        let _ = remove_path(&backup);
        return Err(e);
    }

    let mut files = Vec::new();
    let outcome = pkg.install_with_progress(install_dir, components, |_, _, file| {
        files.push(file.to_string());
    });

    match outcome {
        Ok(n) => {
            let _ = remove_path(&backup);
            Ok(Applied { written: n, files })
        }
        Err(e) => {
            // Roll back: discard the half-applied dir, restore the snapshot.
            //
            // The snapshot is deleted ONLY when every file came back. The restore used to
            // stop at its first failure and the snapshot was deleted regardless — and the
            // commonest reason an update fails is the same thing that makes a restore fail:
            // a file held open (the running app, an antivirus scan). The files after it
            // were then gone from both places.
            let _ = wipe_dir_contents(install_dir);
            let failed = restore_dir(&backup, install_dir);
            if failed == 0 {
                let _ = remove_path(&backup);
                Err(e)
            } else {
                Err(Error::Other(format!(
                    "{e}; the rollback could not restore {failed} file(s) — the previous \
                     install is kept intact at {}",
                    backup.display()
                )))
            }
        }
    }
}

fn backup_path(dir: &Path) -> PathBuf {
    let name = dir
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "install".into());
    dir.parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("{name}.bak"))
}

/// Snapshot `src` into `dst`.
///
/// Symbolic links and junctions are neither followed nor copied: `wipe_dir_contents`
/// leaves them in place, so they need no restoring. Following one (`is_dir()` does) copied
/// whatever it pointed at — a mods library, a whole drive — into the snapshot, and a link
/// back to an ancestor never finished.
fn copy_dir(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst).map_err(|e| Error::io(dst, e))?;
    for entry in std::fs::read_dir(src).map_err(|e| Error::io(src, e))? {
        let entry = entry.map_err(|e| Error::io(src, e))?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        // DirEntry::file_type does not traverse links; a junction reports as one on Windows.
        let kind = entry.file_type().map_err(|e| Error::io(&from, e))?;
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            std::fs::copy(&from, &to).map_err(|e| Error::io(&from, e))?;
        }
    }
    Ok(())
}

/// Copy the snapshot back, CONTINUING past a file that cannot be written, and return how
/// many could not be. A restore that stops at its first failure leaves every later file
/// missing from the install directory, and they are then only in the snapshot.
fn restore_dir(src: &Path, dst: &Path) -> usize {
    let entries = match std::fs::create_dir_all(dst).and_then(|()| std::fs::read_dir(src)) {
        Ok(rd) => rd,
        Err(_) => return 1,
    };
    let mut failed = 0;
    for entry in entries {
        let Ok(entry) = entry else {
            failed += 1;
            continue;
        };
        let (from, to) = (entry.path(), dst.join(entry.file_name()));
        match entry.file_type() {
            Ok(t) if t.is_symlink() => {}
            Ok(t) if t.is_dir() => failed += restore_dir(&from, &to),
            Ok(_) => {
                if std::fs::copy(&from, &to).is_err() {
                    failed += 1;
                }
            }
            Err(_) => failed += 1,
        }
    }
    failed
}

/// Empty `dir` before a restore. Links and junctions stay: the snapshot did not copy them,
/// and removing one would lose the link itself for good.
fn wipe_dir_contents(dir: &Path) -> Result<()> {
    for entry in std::fs::read_dir(dir).map_err(|e| Error::io(dir, e))? {
        let entry = entry.map_err(|e| Error::io(dir, e))?;
        let p = entry.path();
        match entry.file_type() {
            Ok(t) if t.is_symlink() => {}
            Ok(t) if t.is_dir() => {
                let _ = std::fs::remove_dir_all(&p);
            }
            _ => {
                let _ = std::fs::remove_file(&p);
            }
        }
    }
    Ok(())
}

fn remove_path(p: &Path) -> Result<()> {
    if p.is_dir() {
        std::fs::remove_dir_all(p).map_err(|e| Error::io(p, e))
    } else if p.exists() {
        std::fs::remove_file(p).map_err(|e| Error::io(p, e))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::AppMeta;

    fn app() -> AppMeta {
        AppMeta {
            id: "test".into(),
            name: "Test".into(),
            version: "1".into(),
            publisher: "p".into(),
            homepage: None,
            platforms: vec!["windows".into()],
        }
    }

    #[test]
    fn version_compare() {
        assert!(is_newer("1.2.0", "1.1.9"));
        assert!(is_newer("2.0.0", "1.9.9"));
        assert!(is_newer("1.0.1", "1.0")); // missing components = 0
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("1.0.0", "1.0.1"));
        // Numeric, not lexical: "1.10.0" > "1.9.0" (string compare would get this wrong).
        assert!(is_newer("1.10.0", "1.9.0"));
        assert!(!is_newer("1.9.0", "1.10.0"));
        assert!(is_newer("1.0.10", "1.0.9"));
        // Trailing-zero components are equal, not newer ("1.2" == "1.2.0").
        assert!(!is_newer("1.2", "1.2.0"));
        assert!(!is_newer("1.2.0", "1.2"));
    }

    #[test]
    fn update_succeeds_then_rolls_back_on_corrupt() {
        let base = std::env::temp_dir().join(format!("bpkg-upd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let v1 = base.join("v1");
        let v2 = base.join("v2");
        std::fs::create_dir_all(&v1).unwrap();
        std::fs::create_dir_all(&v2).unwrap();
        std::fs::write(v1.join("f.txt"), b"VERSION ONE").unwrap();
        std::fs::write(v2.join("f.txt"), b"VERSION TWO!!").unwrap();

        let p1 = base.join("v1.bpkg");
        let p2 = base.join("v2.bpkg");
        crate::package::create_from_dir(&v1, app(), vec![], |_| None, &p1).unwrap();
        crate::package::create_from_dir(&v2, app(), vec![], |_| None, &p2).unwrap();

        // Install v1.
        let install = base.join("install");
        {
            let mut pkg = Package::open(&p1).unwrap();
            pkg.install_with_progress(&install, None, |_, _, _| {})
                .unwrap();
        }
        assert_eq!(
            std::fs::read(install.join("f.txt")).unwrap(),
            b"VERSION ONE"
        );

        // Update to v2 → success (no key pinned).
        apply_package_update(&p2, &install, None, None).unwrap();
        assert_eq!(
            std::fs::read(install.join("f.txt")).unwrap(),
            b"VERSION TWO!!"
        );

        // Corrupt v2 and update again → must fail AND leave v2 intact (rollback).
        let mut bytes = std::fs::read(&p2).unwrap();
        let n = bytes.len();
        bytes[n - 40] ^= 0xFF; // flip a byte inside the compressed payload
        let bad = base.join("bad.bpkg");
        std::fs::write(&bad, &bytes).unwrap();
        let err = apply_package_update(&bad, &install, None, None);
        assert!(err.is_err(), "corrupt update must fail");
        assert_eq!(
            std::fs::read(install.join("f.txt")).unwrap(),
            b"VERSION TWO!!",
            "rollback should restore the pre-update state"
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn update_refuses_unsigned_package_when_key_pinned() {
        let base = std::env::temp_dir().join(format!("bpkg-sig-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let src = base.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("f.txt"), b"payload").unwrap();

        // An UNSIGNED package (the signer closure returns None).
        let pkg = base.join("v.bpkg");
        crate::package::create_from_dir(&src, app(), vec![], |_| None, &pkg).unwrap();

        let install = base.join("install");
        std::fs::create_dir_all(&install).unwrap();

        // With a pinned key, an unsigned package must be REFUSED (fail closed) and the
        // install dir left untouched.
        let vk = crate::sign::generate().verifying_key();
        let res = apply_package_update(&pkg, &install, None, Some(&vk));
        assert!(
            res.is_err(),
            "unsigned package must be rejected when a key is pinned"
        );
        assert!(
            !install.join("f.txt").exists(),
            "nothing should be written when the signature gate fails"
        );

        // Without a pinned key, the same package still applies (legacy behaviour).
        apply_package_update(&pkg, &install, None, None).unwrap();
        assert!(install.join("f.txt").exists());

        let _ = std::fs::remove_dir_all(&base);
    }
}

#[cfg(test)]
mod offered_tests {
    use super::*;
    use crate::manifest::AppMeta;

    fn meta(id: &str, version: &str) -> AppMeta {
        AppMeta {
            id: id.into(),
            name: "App".into(),
            version: version.into(),
            publisher: "p".into(),
            homepage: None,
            platforms: vec![],
        }
    }

    fn offer(version: &str) -> UpdateManifest {
        UpdateManifest {
            version: version.into(),
            url: "https://example.invalid/app.bpkg".into(),
            ..Default::default()
        }
    }

    /// A signed package built for `app`, and the key that signed it.
    fn signed(base: &Path, app: AppMeta) -> (Vec<u8>, ed25519_dalek::SigningKey) {
        let src = base.join(format!("src-{}", app.version));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("app.exe"), format!("build {}", app.version)).unwrap();
        let out = base.join(format!("{}-{}.bpkg", app.id, app.version));
        crate::package::create_from_dir(&src, app, vec![], |_| None, &out).unwrap();
        let sk = crate::sign::generate();
        crate::package::sign_package(&out, &sk).unwrap();
        (std::fs::read(&out).unwrap(), sk)
    }

    /// The attack: the manifest offers 1.3.0, a mirror serves an OLD release the same
    /// publisher genuinely signed. The signature verifies. It must still not install.
    #[test]
    fn a_genuinely_signed_older_release_is_not_an_update() {
        let base = std::env::temp_dir().join(format!("bpkg-rollback-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let install = base.join("install");
        std::fs::create_dir_all(&install).unwrap();
        std::fs::write(install.join("app.exe"), b"build 1.2.0").unwrap();

        let (old, sk) = signed(&base, meta("app", "1.0.0"));
        let vk = sk.verifying_key();
        let err = apply_downloaded(
            &old,
            &offer("1.3.0"),
            "1.2.0",
            &install,
            Some(&vk),
            Some("app"),
            |_| {},
        )
        .expect_err("a downgrade to a signed 1.0.0 must be refused");
        assert!(err.to_string().contains("1.3.0"), "{err}");
        assert_eq!(
            std::fs::read(install.join("app.exe")).unwrap(),
            b"build 1.2.0"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_offered_version_of_this_app_and_only_that_is_accepted() {
        assert!(check_offered(&meta("app", "1.3.0"), "1.3.0", "1.2.0", Some("app")).is_ok());
        assert!(check_offered(&meta("app", "1.3"), "1.3.0", "1.2.0", Some("app")).is_ok());
        // Offered 1.3.0, got 1.2.5: newer than installed, still not what was offered.
        assert!(check_offered(&meta("app", "1.2.5"), "1.3.0", "1.2.0", Some("app")).is_err());
        // The installed release replayed (manifest lying about its version too).
        assert!(check_offered(&meta("app", "1.2.0"), "1.2.0", "1.2.0", Some("app")).is_err());
        // Another app by the same publisher.
        assert!(check_offered(&meta("other", "1.3.0"), "1.3.0", "1.2.0", Some("app")).is_err());
        // No expected id (the CLI): the version rules still hold.
        assert!(check_offered(&meta("other", "1.3.0"), "1.3.0", "1.2.0", None).is_ok());
        assert!(check_offered(&meta("app", "1.0.0"), "1.0.0", "1.2.0", None).is_err());
    }
}

#[cfg(test)]
mod mirror_tests {
    use super::download_any;

    // The HTTPS gate rejects a plain-http URL before any socket is opened, which makes it a
    // deterministic, offline stand-in for "this source failed" — enough to prove the loop
    // actually walks the mirrors instead of stopping at the primary.
    #[test]
    fn tries_every_mirror_and_reports_the_last_failure() {
        let err = download_any(
            "http://primary.invalid/a.bpkg",
            &[
                "http://one.invalid/a.bpkg".into(),
                "http://two.invalid/a.bpkg".into(),
            ],
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("two.invalid"),
            "expected the LAST mirror's error, got: {err}"
        );
    }

    #[test]
    fn blank_mirrors_are_skipped() {
        let err = download_any("http://primary.invalid/a.bpkg", &["".into(), "   ".into()])
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("primary.invalid"),
            "expected the primary's error, got: {err}"
        );
    }

    #[test]
    fn no_mirrors_behaves_exactly_as_before() {
        let err = download_any("http://primary.invalid/a.bpkg", &[])
            .unwrap_err()
            .to_string();
        assert!(err.contains("primary.invalid"));
    }
}

/// Card C-1: the signed `update.json`.
#[cfg(test)]
mod signed_manifest_tests {
    use super::*;
    use crate::manifest::AppMeta;
    use chrono::{Duration, Utc};

    fn body(version: &str, sha: &str) -> UpdateManifest {
        UpdateManifest {
            app_id: Some("app".into()),
            version: version.into(),
            url: "https://example.invalid/app.bpkg".into(),
            sha256: Some(sha.into()),
            config_sha256: Some(sha256_hex(b"[app]\nid = \"app\"\n")),
            ..Default::default()
        }
    }

    fn zero_sha() -> String {
        "0".repeat(64)
    }

    fn strict(vk: &VerifyingKey) -> ManifestPolicy<'_> {
        ManifestPolicy::new(Some(vk), Some("app"))
    }

    const LEGACY: &str =
        r#"{ "version": "1.3.0", "url": "https://example.invalid/app.bpkg", "notes": "n" }"#;

    #[test]
    fn a_signed_manifest_round_trips_and_keeps_a_copy_for_older_engines() {
        let sk = crate::sign::generate();
        let text = sign_manifest(&body("1.3.0", &zero_sha()), &sk, Utc::now(), 7).unwrap();
        let m = parse_manifest(&text, &strict(&sk.verifying_key())).unwrap();
        assert_eq!(m.trust, ManifestTrust::Verified);
        assert_eq!(m.version, "1.3.0");
        assert_eq!(m.sha256.as_deref(), Some(zero_sha().as_str()));
        assert!(m.config_sha256.is_some());
        // An engine that predates signing reads the top level, and finds what it knows.
        let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(doc["version"], "1.3.0");
        assert_eq!(doc["url"], "https://example.invalid/app.bpkg");
    }

    /// Refusal 1: an unsigned manifest, when the installer pins a key.
    #[test]
    fn an_unsigned_manifest_is_refused_when_a_key_is_pinned() {
        let vk = crate::sign::generate().verifying_key();
        let err = parse_manifest(LEGACY, &strict(&vk)).expect_err("unsigned must be refused");
        assert!(err.to_string().contains("not signed"), "{err}");

        // The migration rule, and only when the caller says it applies: accepted, flagged.
        let mut grace = strict(&vk);
        grace.accept_unsigned = true;
        let m = parse_manifest(LEGACY, &grace).unwrap();
        assert_eq!(m.trust, ManifestTrust::UnsignedAccepted);
        // No key configured at all: nothing to verify with, read as given.
        let m = parse_manifest(LEGACY, &ManifestPolicy::new(None, Some("app"))).unwrap();
        assert_eq!(m.trust, ManifestTrust::Unverified);
    }

    #[test]
    fn a_manifest_signed_by_another_key_or_edited_after_signing_is_refused() {
        let sk = crate::sign::generate();
        let text = sign_manifest(&body("1.3.0", &zero_sha()), &sk, Utc::now(), 7).unwrap();
        let other = crate::sign::generate().verifying_key();
        assert!(parse_manifest(&text, &strict(&other)).is_err());

        // The version inside the signed string, changed after signing.
        let edited = text.replace(r#"\"version\":\"1.3.0\""#, r#"\"version\":\"1.4.0\""#);
        assert_ne!(edited, text, "the edit did not apply");
        let err = parse_manifest(&edited, &strict(&sk.verifying_key())).unwrap_err();
        assert!(err.to_string().contains("signature"), "{err}");
    }

    /// Refusal 2: an expired manifest. Also a body signed for longer than the limit.
    #[test]
    fn an_expired_manifest_is_refused() {
        let sk = crate::sign::generate();
        let vk = sk.verifying_key();
        let old = Utc::now() - Duration::days(8);
        let text = sign_manifest(&body("1.3.0", &zero_sha()), &sk, old, 7).unwrap();
        let err = parse_manifest(&text, &strict(&vk)).expect_err("expired must be refused");
        assert!(err.to_string().contains("expired"), "{err}");
        // Still inside its week: accepted.
        let recent = Utc::now() - Duration::days(6);
        let text = sign_manifest(&body("1.3.0", &zero_sha()), &sk, recent, 7).unwrap();
        assert!(parse_manifest(&text, &strict(&vk)).is_ok());

        // Signed by hand for a year: the publisher's tool refuses to make it, and a client
        // refuses to read it.
        assert!(sign_manifest(&body("1.3.0", &zero_sha()), &sk, Utc::now(), 365).is_err());
        let mut long = body("1.3.0", &zero_sha());
        long.issued = Some(Utc::now().to_rfc3339());
        long.expires = Some((Utc::now() + Duration::days(365)).to_rfc3339());
        let signed = serde_json::to_string(&long).unwrap();
        let mut msg = MANIFEST_SIG_CONTEXT.to_vec();
        msg.extend_from_slice(signed.as_bytes());
        let sig = crate::sign::hex_encode(&crate::sign::sign_message(&sk, &msg));
        let doc = serde_json::json!({
            "version": "1.3.0", "url": "x", "signed": signed, "signature": sig
        });
        let err = parse_manifest(&doc.to_string(), &strict(&vk)).unwrap_err();
        assert!(err.to_string().contains("limit"), "{err}");
    }

    /// The signed body is the only thing a verifying engine reads.
    #[test]
    fn the_unsigned_copy_is_never_read_when_a_signature_is_there() {
        let sk = crate::sign::generate();
        let text = sign_manifest(&body("1.3.0", &zero_sha()), &sk, Utc::now(), 7).unwrap();
        let mut doc: serde_json::Value = serde_json::from_str(&text).unwrap();
        doc["version"] = "9.9.9".into();
        doc["url"] = "https://evil.invalid/x.bpkg".into();
        let m = parse_manifest(&doc.to_string(), &strict(&sk.verifying_key())).unwrap();
        assert_eq!(m.version, "1.3.0");
        assert_eq!(m.url, "https://example.invalid/app.bpkg");
    }

    #[test]
    fn a_manifest_signed_for_another_app_is_refused() {
        let sk = crate::sign::generate();
        let mut b = body("1.3.0", &zero_sha());
        b.app_id = Some("other".into());
        let text = sign_manifest(&b, &sk, Utc::now(), 7).unwrap();
        let err = parse_manifest(&text, &strict(&sk.verifying_key())).unwrap_err();
        assert!(err.to_string().contains("other"), "{err}");
    }

    /// The weekly job: an expired manifest is renewed, by its own key only.
    #[test]
    fn resigning_renews_an_expired_manifest_but_only_with_the_key_that_signed_it() {
        let sk = crate::sign::generate();
        let vk = sk.verifying_key();
        let old = sign_manifest(
            &body("1.3.0", &zero_sha()),
            &sk,
            Utc::now() - Duration::days(30),
            7,
        )
        .unwrap();
        assert!(parse_manifest(&old, &strict(&vk)).is_err());
        let renewed = resign_manifest(&old, &sk, Utc::now(), 7).unwrap();
        let m = parse_manifest(&renewed, &strict(&vk)).unwrap();
        assert_eq!(
            (m.version.as_str(), m.trust),
            ("1.3.0", ManifestTrust::Verified)
        );

        assert!(resign_manifest(&old, &crate::sign::generate(), Utc::now(), 7).is_err());
        assert!(resign_manifest(LEGACY, &sk, Utc::now(), 7).is_err());
    }

    fn package(base: &Path, tag: &str, version: &str, body: &str, sk: &SigningKey) -> Vec<u8> {
        let src = base.join(format!("src-{tag}"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("app.exe"), body).unwrap();
        let out = base.join(format!("{tag}.bpkg"));
        let app = AppMeta {
            id: "app".into(),
            name: "App".into(),
            version: version.into(),
            publisher: "p".into(),
            homepage: None,
            platforms: vec![],
        };
        crate::package::create_from_dir(&src, app, vec![], |_| None, &out).unwrap();
        crate::package::sign_package(&out, sk).unwrap();
        std::fs::read(&out).unwrap()
    }

    use ed25519_dalek::SigningKey;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bpkg-c1-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("install")).unwrap();
        std::fs::write(d.join("install/app.exe"), b"build 1.2.0").unwrap();
        d
    }

    /// Refusal 3: a genuine, signed, current manifest for a version OLDER than the installed
    /// one (a stale copy on a host, a replay inside its week) is neither offered nor applied.
    #[test]
    fn a_manifest_older_than_the_installed_version_is_not_an_update() {
        let base = scratch("older");
        let sk = crate::sign::generate();
        let vk = sk.verifying_key();
        let pkg = package(&base, "old", "1.0.0", "build 1.0.0", &sk);
        let text = sign_manifest(&body("1.0.0", &sha256_hex(&pkg)), &sk, Utc::now(), 7).unwrap();
        let m = parse_manifest(&text, &strict(&vk)).unwrap();
        assert_eq!(m.trust, ManifestTrust::Verified);
        assert!(offer_if_newer(m.clone(), "1.2.0").is_none());
        assert!(offer_if_newer(m.clone(), "1.0.0").is_none());
        // And a caller that applies it anyway is refused before anything is written.
        let install = base.join("install");
        let err = apply_downloaded(&pkg, &m, "1.2.0", &install, Some(&vk), Some("app"), |_| {
            panic!("the app must not be closed for an update that is refused")
        })
        .unwrap_err();
        assert!(err.to_string().contains("not newer"), "{err}");
        assert_eq!(
            std::fs::read(install.join("app.exe")).unwrap(),
            b"build 1.2.0"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The signed hash pins the exact package: another package the same key signed, even at
    /// the offered version, is not it.
    #[test]
    fn the_download_must_be_the_package_the_manifest_names() {
        let base = scratch("sha");
        let sk = crate::sign::generate();
        let vk = sk.verifying_key();
        let good = package(&base, "good", "1.3.0", "build 1.3.0", &sk);
        let other = package(&base, "other", "1.3.0", "a different 1.3.0", &sk);
        let text = sign_manifest(&body("1.3.0", &sha256_hex(&good)), &sk, Utc::now(), 7).unwrap();
        let m = parse_manifest(&text, &strict(&vk)).unwrap();

        let install = base.join("install");
        let err = apply_downloaded(
            &other,
            &m,
            "1.2.0",
            &install,
            Some(&vk),
            Some("app"),
            |_| {},
        )
        .unwrap_err();
        assert!(err.to_string().contains("SHA-256"), "{err}");
        assert_eq!(
            std::fs::read(install.join("app.exe")).unwrap(),
            b"build 1.2.0"
        );

        let done =
            apply_downloaded(&good, &m, "1.2.0", &install, Some(&vk), Some("app"), |_| {}).unwrap();
        assert_eq!(done.files, vec!["app.exe".to_string()]);
        assert_eq!(
            std::fs::read(install.join("app.exe")).unwrap(),
            b"build 1.3.0"
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}
