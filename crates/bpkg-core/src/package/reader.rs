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
pub struct Package {
    file: std::fs::File,
    header: Header,
    pub manifest: Manifest,
    payload_offset: u64,
}

impl Package {
    pub fn open(path: impl AsRef<Path>) -> Result<Package> {
        let path = path.as_ref();
        let mut file = std::fs::File::open(path).map_err(|e| Error::io(path, e))?;

        let mut head = [0u8; HEADER_LEN];
        file.read_exact(&mut head).map_err(|e| Error::io(path, e))?;
        let header = Header::from_bytes(&head)?;

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
        })
    }

    /// Read and decompress the full inner archive into memory.
    fn read_archive(&mut self) -> Result<Vec<u8>> {
        self.file
            .seek(SeekFrom::Start(self.payload_offset))
            .map_err(Error::IoBare)?;
        let mut compressed = vec![0u8; self.header.payload_len as usize];
        self.file
            .read_exact(&mut compressed)
            .map_err(Error::IoBare)?;
        zstd::decode_all(&compressed[..]).map_err(|e| Error::Compression(e.to_string()))
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
        let mp_len = self.header.manifest_len as u64 + self.header.payload_len;
        self.file
            .seek(SeekFrom::Start(HEADER_LEN as u64))
            .map_err(Error::IoBare)?;
        let mut buf = vec![0u8; mp_len as usize];
        self.file.read_exact(&mut buf).map_err(Error::IoBare)?;
        let mut sig = [0u8; SIGNATURE_LEN];
        self.file.read_exact(&mut sig).map_err(Error::IoBare)?;
        Ok(crate::sign::verify_message(vk, &buf, &sig))
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
            // Integrity: never write a file whose hash doesn't match the manifest.
            if let Some(exp) = expected.get(path) {
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
            }
            if path.contains("..") || path.starts_with('/') || path.starts_with('\\') {
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
            // component gate
            if let Some(sel) = components {
                if let Some(Some(c)) = comp_of.get(path) {
                    if !sel.iter().any(|s| s == c) {
                        return Ok(()); // skipped: unselected optional component
                    }
                }
            }
            // path-traversal guard
            if path.contains("..") || path.starts_with('/') || path.starts_with('\\') {
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
