//! Ed25519 package signing (v3 plan §2.2, Phase 5).
//!
//! Keys are stored as hex text: `private.key` (32-byte seed) and `public.key`
//! (32-byte verifying key). A signature covers the package's header, manifest and
//! payload — bytes `0 .. 24+N+M` (see `package::sign_package`). Never commit
//! `private.key`.

use std::path::Path;

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

use crate::error::{Error, Result};

/// Generate a fresh keypair from the OS CSPRNG.
pub fn generate() -> SigningKey {
    use rand::rngs::OsRng;
    SigningKey::generate(&mut OsRng)
}

/// Write a NEW private key. Refuses to replace an existing file.
///
/// `bpkg keygen` used to `fs::write` here, and running it a second time on the same
/// `--out` (its default is `keys`) silently replaced the publisher key. Every installed
/// copy pins the old public key and accepts updates signed by it alone, so the old key is
/// the only thing that can ever update them — overwriting it strands every install. On
/// Unix the file is also created owner-only (0600) rather than world-readable.
pub fn save_private(sk: &SigningKey, path: &Path) -> Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            Error::Other(format!(
                "{} already exists — refusing to replace a signing key (installed apps \
                 trust the key it holds). Move it away first if you really mean to rotate.",
                path.display()
            ))
        } else {
            Error::io(path, e)
        }
    })?;
    f.write_all(hex_encode(&sk.to_bytes()).as_bytes())
        .map_err(|e| Error::io(path, e))
}

pub fn save_public(vk: &VerifyingKey, path: &Path) -> Result<()> {
    std::fs::write(path, hex_encode(&vk.to_bytes())).map_err(|e| Error::io(path, e))
}

pub fn load_private(path: &Path) -> Result<SigningKey> {
    let s = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
    let bytes = hex_decode(s.trim()).ok_or_else(|| Error::Other("private.key: bad hex".into()))?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| Error::Other("private.key must be 32 bytes".into()))?;
    Ok(SigningKey::from_bytes(&arr))
}

pub fn load_public(path: &Path) -> Result<VerifyingKey> {
    let s = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
    parse_public(s.trim())
}

/// Parse a hex-encoded 32-byte public key (e.g. from `installer.toml`).
pub fn parse_public(hex: &str) -> Result<VerifyingKey> {
    let bytes = hex_decode(hex).ok_or_else(|| Error::Other("public key: bad hex".into()))?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| Error::Other("public key must be 32 bytes".into()))?;
    VerifyingKey::from_bytes(&arr).map_err(|e| Error::Other(format!("invalid public key: {e}")))
}

pub fn sign_message(sk: &SigningKey, msg: &[u8]) -> [u8; 64] {
    sk.sign(msg).to_bytes()
}

pub fn verify_message(vk: &VerifyingKey, msg: &[u8], sig: &[u8; 64]) -> bool {
    vk.verify(msg, &Signature::from_bytes(sig)).is_ok()
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    // Byte-indexed slicing below: a multi-byte character would split mid-codepoint and
    // panic instead of reporting "bad hex".
    if !s.is_ascii() || !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_verify_roundtrip_and_tamper() {
        let sk = generate();
        let vk = sk.verifying_key();
        let msg = b"the package manifest + payload bytes";
        let sig = sign_message(&sk, msg);

        assert!(verify_message(&vk, msg, &sig));
        // tampered message must fail
        assert!(!verify_message(
            &vk,
            b"the package manifest + payload bytez",
            &sig
        ));

        // hex round-trip of the public key
        let hex = hex_encode(&vk.to_bytes());
        let vk2 = parse_public(&hex).unwrap();
        assert!(verify_message(&vk2, msg, &sig));
    }

    #[test]
    fn a_second_keygen_never_replaces_the_publisher_key() {
        let dir = std::env::temp_dir().join(format!("bpkg-keys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("private.key");

        let first = generate();
        save_private(&first, &path).unwrap();
        let err = save_private(&generate(), &path).expect_err("must not overwrite");
        assert!(err.to_string().contains("already exists"), "{err}");
        assert_eq!(
            load_private(&path).unwrap().to_bytes(),
            first.to_bytes(),
            "the key installed apps trust was replaced"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0, "private.key readable by group/other");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Non-ASCII in a key file must be an error, not a panic on a char boundary.
    #[test]
    fn a_key_with_non_ascii_text_is_an_error() {
        // Three-byte characters: slicing two bytes at a time lands inside one.
        assert!(parse_public("\u{20ac}\u{20ac}").is_err());
        assert!(parse_public(&"ab\u{20ac}\u{20ac}".repeat(4)).is_err());
    }
}
