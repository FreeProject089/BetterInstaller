//! Open, verify and extract a `.bpkg`.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use sha2::{Digest, Sha256};

use ed25519_dalek::VerifyingKey;

use super::format::{Header, FLAG_SIGNED, HEADER_LEN, SIGNATURE_LEN};
use super::writer::hex;
use crate::error::{Error, Result};
use crate::manifest::Manifest;

/// A parsed package. The header + manifest are read eagerly; the (compressed)
/// payload is read on demand by [`Package::extract`] / [`Package::verify`].
///
/// Once [`Package::verify_signature`] succeeds, the bytes it verified are kept and every
/// later read is served from them, not from the file (see `verified`).
pub struct Package {
    file: std::fs::File,
    header: Header,
    pub manifest: Manifest,
    payload_offset: u64,
    /// The header and manifest exactly as read by `open` — what `manifest` was parsed from.
    head: [u8; HEADER_LEN],
    manifest_raw: Vec<u8>,
    /// Bytes `0 .. 24+N+M` as they were when the signature verified.
    ///
    /// The path used to be read three separate times: the manifest at `open`, the signed
    /// range at `verify_signature`, the payload at extraction. Swap the file between those
    /// reads — hostile manifest at open, the genuine package while it is verified, a
    /// payload matching the hostile manifest for extraction — and a package nobody signed
    /// installs as "verified" (CWE-367). Keeping the verified bytes, and refusing a
    /// verification whose header and manifest are not the ones `open` parsed, makes the
    /// signed bytes the only bytes the package can ever yield.
    verified: Option<Vec<u8>>,
}

impl Package {
    pub fn open(path: impl AsRef<Path>) -> Result<Package> {
        let path = path.as_ref();
        let mut file = std::fs::File::open(path).map_err(|e| Error::io(path, e))?;

        let mut head = [0u8; HEADER_LEN];
        file.read_exact(&mut head).map_err(|e| Error::io(path, e))?;
        let header = Header::from_bytes(&head)?;

        // The two lengths are read from the file and every later allocation is sized by
        // them: bound them by what the file actually holds before trusting either (a
        // 40-byte file claiming a 16 EiB payload is an allocation failure = abort, not
        // an error).
        let file_len = file.metadata().map_err(|e| Error::io(path, e))?.len();
        let claimed = (HEADER_LEN as u64)
            .checked_add(header.manifest_len as u64)
            .and_then(|n| n.checked_add(header.payload_len));
        if claimed.is_none_or(|n| n > file_len) {
            return Err(Error::Corrupt("header lengths exceed the file size".into()));
        }

        let mut manifest_buf = vec![0u8; header.manifest_len as usize];
        file.read_exact(&mut manifest_buf)
            .map_err(|e| Error::io(path, e))?;
        let manifest: Manifest = serde_json::from_slice(&manifest_buf)?;

        let payload_offset = HEADER_LEN as u64 + header.manifest_len as u64;
        Ok(Package {
            file,
            header,
            manifest,
            payload_offset,
            head,
            manifest_raw: manifest_buf,
            verified: None,
        })
    }

    /// Read and decompress the full inner archive into memory.
    fn read_archive(&mut self) -> Result<Vec<u8>> {
        let from_file;
        let compressed: &[u8] = match &self.verified {
            // Verified: the payload is the one the signature covered, whatever the path
            // holds now.
            Some(signed) => &signed[self.payload_offset as usize..],
            None => {
                self.file
                    .seek(SeekFrom::Start(self.payload_offset))
                    .map_err(Error::IoBare)?;
                let mut buf = vec![0u8; self.header.payload_len as usize];
                self.file.read_exact(&mut buf).map_err(Error::IoBare)?;
                from_file = buf;
                &from_file
            }
        };
        // Bound decompression against a zip-bomb payload (a tiny compressed blob that
        // inflates to gigabytes and OOMs the installer): stream-decode with a ceiling.
        const MAX_ARCHIVE: u64 = 4 * 1024 * 1024 * 1024; // 4 GiB decompressed
        let mut decoder =
            zstd::Decoder::new(compressed).map_err(|e| Error::Compression(e.to_string()))?;
        let mut out = Vec::new();
        decoder
            .by_ref()
            .take(MAX_ARCHIVE + 1)
            .read_to_end(&mut out)
            .map_err(|e| Error::Compression(e.to_string()))?;
        if out.len() as u64 > MAX_ARCHIVE {
            return Err(Error::Compression(
                "decompressed archive exceeds size limit".into(),
            ));
        }
        Ok(out)
    }

    /// Iterate the inner archive, calling `f(path, data)` for each file.
    fn for_each_entry(&mut self, mut f: impl FnMut(&str, &[u8]) -> Result<()>) -> Result<()> {
        let archive = self.read_archive()?;
        let mut pos = 0usize;
        let len = archive.len();
        while pos < len {
            let path_len = read_u32(&archive, &mut pos)? as usize;
            let path = std::str::from_utf8(slice(&archive, &mut pos, path_len)?)
                .map_err(|_| Error::Corrupt("non-utf8 path in archive".into()))?
                .to_string();
            let data_len = read_u64(&archive, &mut pos)? as usize;
            let data = slice(&archive, &mut pos, data_len)?;
            f(&path, data)?;
        }
        Ok(())
    }

    /// Verify every file's SHA-256 against the manifest.
    pub fn verify(&mut self) -> Result<()> {
        let expected: HashMap<String, String> = self
            .manifest
            .files
            .iter()
            .map(|e| (e.path.clone(), e.sha256.clone()))
            .collect();
        self.for_each_entry(|path, data| {
            let mut h = Sha256::new();
            h.update(data);
            let actual = hex(&h.finalize());
            match expected.get(path) {
                Some(exp) if *exp == actual => Ok(()),
                Some(exp) => Err(Error::HashMismatch {
                    path: path.to_string(),
                    expected: exp.clone(),
                    actual,
                }),
                None => Err(Error::Corrupt(format!("file {path} not in manifest"))),
            }
        })
    }

    /// Read the raw bytes of specific payload files in one decompression pass
    /// (used to show license documents before installing). Missing files are
    /// simply absent from the result.
    pub fn read_files(&mut self, paths: &[String]) -> Result<HashMap<String, Vec<u8>>> {
        let want: std::collections::HashSet<&str> = paths.iter().map(|s| s.as_str()).collect();
        let mut out = HashMap::new();
        self.for_each_entry(|p, data| {
            if want.contains(p) {
                out.insert(p.to_string(), data.to_vec());
            }
            Ok(())
        })?;
        Ok(out)
    }

    /// Whether the package carries an Ed25519 signature.
    pub fn is_signed(&self) -> bool {
        self.header.flags & FLAG_SIGNED != 0
    }

    /// Verify the package signature against `vk`. Returns `Ok(false)` for an
    /// unsigned package, `Ok(true)`/`Ok(false)` for a valid/invalid signature.
    pub fn verify_signature(&mut self, vk: &VerifyingKey) -> Result<bool> {
        if !self.is_signed() {
            return Ok(false);
        }
        // Header INCLUDED — see sign_package. The two lengths used to decide the range are
        // themselves in the header, so verifying a range chosen by unverified bytes proved
        // less than it looked like it proved.
        let signed_len =
            HEADER_LEN as u64 + self.header.manifest_len as u64 + self.header.payload_len;
        self.file.seek(SeekFrom::Start(0)).map_err(Error::IoBare)?;
        let mut buf = vec![0u8; signed_len as usize];
        self.file.read_exact(&mut buf).map_err(Error::IoBare)?;
        let mut sig = [0u8; SIGNATURE_LEN];
        self.file.read_exact(&mut sig).map_err(Error::IoBare)?;
        // The signature has to be over the package THIS value holds — the header and
        // manifest `open` parsed — not merely over whatever the path holds right now.
        let n = self.manifest_raw.len();
        if buf[..HEADER_LEN] != self.head
            || buf[HEADER_LEN..HEADER_LEN + n] != self.manifest_raw[..]
        {
            return Ok(false);
        }
        if !crate::sign::verify_message(vk, &buf, &sig) {
            return Ok(false);
        }
        self.verified = Some(buf);
        Ok(true)
    }

    /// Whether a file's component is selected for install.
    fn selected(component: Option<&str>, components: Option<&[String]>) -> bool {
        match (component, components) {
            (Some(c), Some(set)) => set.iter().any(|s| s == c),
            _ => true, // core/None, or "install everything"
        }
    }

    /// The full install pass: verify each file's SHA-256 *and* write it, reporting
    /// `(done, total, path)` after every file. This is what the installer's Install
    /// step calls. Returns the number of files written.
    pub fn install_with_progress(
        &mut self,
        dest: &Path,
        components: Option<&[String]>,
        mut on_progress: impl FnMut(usize, usize, &str),
    ) -> Result<u64> {
        let comp_of: HashMap<String, Option<String>> = self
            .manifest
            .files
            .iter()
            .map(|e| (e.path.clone(), e.component.clone()))
            .collect();
        let expected: HashMap<String, String> = self
            .manifest
            .files
            .iter()
            .map(|e| (e.path.clone(), e.sha256.clone()))
            .collect();
        let total = self
            .manifest
            .files
            .iter()
            .filter(|f| Self::selected(f.component.as_deref(), components))
            .count();

        let mut done = 0usize;
        let dest = dest.to_path_buf();
        self.for_each_entry(|path, data| {
            let comp = comp_of.get(path).and_then(|c| c.as_deref());
            if !Self::selected(comp, components) {
                return Ok(()); // unselected optional component
            }
            // Integrity: never write a file whose hash doesn't match the manifest, and
            // never write one the manifest does not describe at all.
            //
            // `if let Some(exp)` used to mean an UNLISTED entry skipped the hash check and
            // was written anyway — and `Self::selected` reads a missing component as
            // "core", so it also ignored the component tick-boxes. Everything shown before
            // the install (the component list and its sizes, `bpkg info`) is read off the
            // manifest, so a file that is not in it is a file nobody agreed to. `verify()`
            // has always refused this; the path that actually writes did not.
            let exp = expected
                .get(path)
                .ok_or_else(|| Error::Corrupt(format!("file {path} not in manifest")))?;
            let mut h = Sha256::new();
            h.update(data);
            let actual = hex(&h.finalize());
            if actual != *exp {
                return Err(Error::HashMismatch {
                    path: path.to_string(),
                    expected: exp.clone(),
                    actual,
                });
            }
            if unsafe_entry_path(path) {
                return Err(Error::Corrupt(format!("unsafe path in archive: {path}")));
            }
            let out = dest.join(path);
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
            }
            std::fs::write(&out, data).map_err(|e| Error::io(&out, e))?;
            done += 1;
            on_progress(done, total, path);
            Ok(())
        })?;
        Ok(done as u64)
    }

    /// Extract files into `dest`. When `components` is `Some`, only files whose
    /// manifest component is in the set (or `None`/core) are written.
    pub fn extract(&mut self, dest: &Path, components: Option<&[String]>) -> Result<u64> {
        let comp_of: HashMap<String, Option<String>> = self
            .manifest
            .files
            .iter()
            .map(|e| (e.path.clone(), e.component.clone()))
            .collect();

        let mut written = 0u64;
        let dest = dest.to_path_buf();
        self.for_each_entry(|path, data| {
            // Same rule as the install path: the manifest is the list of what this
            // package contains, and an entry outside it is not part of the package.
            if !comp_of.contains_key(path) {
                return Err(Error::Corrupt(format!("file {path} not in manifest")));
            }
            // component gate
            if let Some(sel) = components {
                if let Some(Some(c)) = comp_of.get(path) {
                    if !sel.iter().any(|s| s == c) {
                        return Ok(()); // skipped: unselected optional component
                    }
                }
            }
            // path-traversal guard
            if unsafe_entry_path(path) {
                return Err(Error::Corrupt(format!("unsafe path in archive: {path}")));
            }
            let out = dest.join(path);
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
            }
            std::fs::write(&out, data).map_err(|e| Error::io(&out, e))?;
            written += 1;
            Ok(())
        })?;
        Ok(written)
    }
}

/// Whether `path` is a relative path that stays inside the directory it is joined to —
/// the rule every archive entry is held to, exported for the other places that turn a
/// recorded relative path back into a file to touch (the uninstaller's file list).
pub fn is_safe_entry_path(path: &str) -> bool {
    !unsafe_entry_path(path)
}

/// Reject any archive entry path that could escape the destination directory when
/// joined: parent-dir traversal (`..`), POSIX-absolute (`/…`), Windows drive-absolute
/// (`C:\…` / `C:foo`) or UNC (`\\server`). Host-OS-independent, so a package packed on
/// one platform can't smuggle an absolute path past extraction on another — in
/// particular `dest.join("C:\\Windows\\…")` on Windows silently discards `dest`.
fn unsafe_entry_path(path: &str) -> bool {
    if path.is_empty() || path.contains("..") {
        return true;
    }
    if path.starts_with('/') || path.starts_with('\\') {
        return true;
    }
    let b = path.as_bytes();
    // Windows drive-letter prefix "X:" (optionally followed by \ or /).
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        return true;
    }
    // Belt-and-suspenders: honour the current platform's own notion of "absolute".
    Path::new(path).is_absolute()
}

fn slice<'a>(buf: &'a [u8], pos: &mut usize, len: usize) -> Result<&'a [u8]> {
    let end = pos
        .checked_add(len)
        .ok_or_else(|| Error::Corrupt("length overflow".into()))?;
    if end > buf.len() {
        return Err(Error::Corrupt("truncated archive".into()));
    }
    let s = &buf[*pos..end];
    *pos = end;
    Ok(s)
}

fn read_u32(buf: &[u8], pos: &mut usize) -> Result<u32> {
    let s = slice(buf, pos, 4)?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn read_u64(buf: &[u8], pos: &mut usize) -> Result<u64> {
    let s = slice(buf, pos, 8)?;
    Ok(u64::from_le_bytes([
        s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7],
    ]))
}

#[cfg(test)]
mod tests {
    use super::{unsafe_entry_path, Package, HEADER_LEN};

    #[test]
    fn rejects_escaping_entry_paths() {
        // Traversal, POSIX-absolute, UNC, and Windows drive-absolute must all be blocked
        // (host-OS-independent — this test passes on Linux CI too).
        for p in [
            "../evil",
            "a/../../evil",
            "/etc/passwd",
            "\\\\server\\share\\x",
            "C:\\Windows\\System32\\evil.dll",
            "C:evil",
            "",
        ] {
            assert!(unsafe_entry_path(p), "should reject: {p:?}");
        }
        for p in ["ok.txt", "dir/ok.txt", "a/b/c.dll", "deep/nested/file"] {
            assert!(!unsafe_entry_path(p), "should allow: {p:?}");
        }
    }

    /// A package whose ARCHIVE holds a file its MANIFEST does not list.
    ///
    /// `verify()` has always refused this ("file {path} not in manifest"), but `verify()`
    /// is only what `bpkg verify` calls. The install path checked a hash *if* the manifest
    /// had one and wrote the file either way, and `Self::selected` reads a missing entry as
    /// "core", so the smuggled file also ignored the component tick-boxes. Everything the
    /// user is shown before installing — the component list, its sizes, what `bpkg info`
    /// prints — comes from the manifest, so this is a file that installs without ever
    /// appearing in what was agreed to.
    fn package_with_an_unlisted_entry(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let base = std::env::temp_dir().join(format!("bpkg-unlisted-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let src = base.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("ok.txt"), b"legit").unwrap();
        std::fs::write(src.join("extra.dll"), b"smuggled").unwrap();
        let out = base.join("p.bpkg");
        let mut m = crate::package::create_from_dir(
            &src,
            crate::manifest::AppMeta {
                id: "t".into(),
                name: "T".into(),
                version: "1".into(),
                publisher: "P".into(),
                homepage: None,
                platforms: vec![],
            },
            vec![],
            |_| None,
            &out,
        )
        .unwrap();

        // Drop `extra.dll` from the manifest, leaving it in the payload.
        m.files.retain(|f| f.path != "extra.dll");
        let new_manifest = serde_json::to_vec(&m).unwrap();
        let data = std::fs::read(&out).unwrap();
        let h = super::super::format::Header::from_bytes(&data).unwrap();
        let payload = &data[HEADER_LEN + h.manifest_len as usize..][..h.payload_len as usize];
        let mut rebuilt = super::super::format::Header {
            format_version: h.format_version,
            flags: h.flags,
            manifest_len: new_manifest.len() as u32,
            payload_len: h.payload_len,
        }
        .to_bytes()
        .to_vec();
        rebuilt.extend_from_slice(&new_manifest);
        rebuilt.extend_from_slice(payload);
        std::fs::write(&out, &rebuilt).unwrap();
        (base, out)
    }

    #[test]
    fn an_entry_missing_from_the_manifest_is_never_installed() {
        let (base, out) = package_with_an_unlisted_entry("install");
        let dest = base.join("dest");
        let err = Package::open(&out)
            .unwrap()
            .install_with_progress(&dest, None, |_, _, _| {})
            .expect_err("a file the manifest does not describe must not install");
        assert!(format!("{err}").contains("extra.dll"), "{err}");
        assert!(!dest.join("extra.dll").exists(), "the smuggled file landed");

        // Same rule on the plain extract path — it had no hash check at all.
        let dest2 = base.join("dest2");
        assert!(Package::open(&out).unwrap().extract(&dest2, None).is_err());
        assert!(!dest2.join("extra.dll").exists());

        // And a package whose manifest DOES describe everything still installs.
        let _ = std::fs::remove_dir_all(&base);
    }

    fn pkg_of(base: &std::path::Path, tag: &str, body: &[u8]) -> Vec<u8> {
        let src = base.join(format!("src-{tag}"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("app.exe"), body).unwrap();
        let out = base.join(format!("{tag}.bpkg"));
        crate::package::create_from_dir(
            &src,
            crate::manifest::AppMeta {
                id: "t".into(),
                name: "T".into(),
                version: "1".into(),
                publisher: "P".into(),
                homepage: None,
                platforms: vec![],
            },
            vec![],
            |_| None,
            &out,
        )
        .unwrap();
        std::fs::read(&out).unwrap()
    }

    /// The same path read three times — at open, at verification, at extraction — with
    /// the file swapped in between. Local-attacker timing, done here deterministically.
    #[test]
    fn a_package_swapped_between_reads_never_installs_as_verified() {
        let base = std::env::temp_dir().join(format!("bpkg-toctou-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let sk = crate::sign::generate();
        let vk = sk.verifying_key();

        let genuine_path = base.join("genuine.bpkg");
        std::fs::write(&genuine_path, pkg_of(&base, "g", b"GENUINE")).unwrap();
        crate::package::sign_package(&genuine_path, &sk).unwrap();
        let genuine = std::fs::read(&genuine_path).unwrap();
        let evil = pkg_of(&base, "e", b"EVIL"); // nobody signed this

        let target = base.join("staged.bpkg");

        // 1) hostile at open, genuine while verifying, hostile again for extraction.
        std::fs::write(&target, &evil).unwrap();
        let mut p = Package::open(&target).unwrap();
        std::fs::write(&target, &genuine).unwrap();
        let verified = p.verify_signature(&vk).unwrap_or(false);
        std::fs::write(&target, &evil).unwrap();
        let dest = base.join("d1");
        if verified {
            let _ = p.install_with_progress(&dest, None, |_, _, _| {});
            assert_ne!(
                std::fs::read(dest.join("app.exe")).ok().as_deref(),
                Some(&b"EVIL"[..]),
                "an unsigned payload installed after the signature check passed"
            );
        }
        assert!(
            !verified,
            "verified a package other than the one that was opened"
        );

        // 2) genuine throughout open + verify, swapped only for extraction: what installs
        //    is what was verified.
        std::fs::write(&target, &genuine).unwrap();
        let mut p = Package::open(&target).unwrap();
        assert!(p.verify_signature(&vk).unwrap());
        std::fs::write(&target, &evil).unwrap();
        let dest = base.join("d2");
        p.install_with_progress(&dest, None, |_, _, _| {})
            .expect("the verified bytes are still in hand");
        assert_eq!(std::fs::read(dest.join("app.exe")).unwrap(), b"GENUINE");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn header_lengths_larger_than_the_file_are_refused_before_allocating() {
        let base = std::env::temp_dir().join(format!("bpkg-lens-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let mut bytes = pkg_of(&base, "l", b"x");
        bytes[16..24].copy_from_slice(&(u64::MAX / 2).to_le_bytes()); // payload_len
        let p = base.join("huge.bpkg");
        std::fs::write(&p, &bytes).unwrap();
        let err = Package::open(&p).err().expect("must not open");
        assert!(format!("{err}").contains("exceed"), "{err}");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_well_formed_package_still_installs() {
        let base = std::env::temp_dir().join(format!("bpkg-wf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let src = base.join("src");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("a.txt"), b"a").unwrap();
        std::fs::write(src.join("sub/b.txt"), b"b").unwrap();
        let out = base.join("p.bpkg");
        crate::package::create_from_dir(
            &src,
            crate::manifest::AppMeta {
                id: "t".into(),
                name: "T".into(),
                version: "1".into(),
                publisher: "P".into(),
                homepage: None,
                platforms: vec![],
            },
            vec![],
            |_| None,
            &out,
        )
        .unwrap();
        let dest = base.join("dest");
        assert_eq!(
            Package::open(&out)
                .unwrap()
                .install_with_progress(&dest, None, |_, _, _| {})
                .unwrap(),
            2
        );
        assert!(dest.join("sub/b.txt").exists());
        let _ = std::fs::remove_dir_all(&base);
    }
}
