//! Build a `.bpkg` from a source directory.

use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::format::{Header, FORMAT_VERSION, ZSTD_LEVEL};
use crate::error::{Error, Result};
use crate::manifest::{AppMeta, Component, FileEntry, Manifest};

/// Append one inner-archive entry (`path_len | path | data_len | data`).
fn push_entry(buf: &mut Vec<u8>, rel: &str, data: &[u8]) {
    let path = rel.as_bytes();
    buf.extend_from_slice(&(path.len() as u32).to_le_bytes());
    buf.extend_from_slice(path);
    buf.extend_from_slice(&(data.len() as u64).to_le_bytes());
    buf.extend_from_slice(data);
}

/// Recursively collect files under `root`, returning (relative-forward-slash path,
/// absolute path) pairs. Hidden files are included; the caller controls the tree.
fn collect_files(root: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).map_err(|e| Error::io(&dir, e))? {
            let entry = entry.map_err(|e| Error::io(&dir, e))?;
            let path = entry.path();
            let ty = entry.file_type().map_err(|e| Error::io(&path, e))?;
            if ty.is_dir() {
                stack.push(path);
            } else if ty.is_file() {
                let rel = path
                    .strip_prefix(root)
                    .map_err(|_| Error::Other("path escaped root".into()))?
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((rel, path));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0)); // deterministic ordering → reproducible packages
    Ok(out)
}

/// Create a `.bpkg` at `out` containing every file under `root`.
///
/// `component_of` maps a relative path to a component id (None = core/always).
pub fn create_from_dir(
    root: &Path,
    app: AppMeta,
    components: Vec<Component>,
    component_of: impl Fn(&str) -> Option<String>,
    out: &Path,
) -> Result<Manifest> {
    let files = collect_files(root)?;

    let mut manifest = Manifest::new(app);
    manifest.components = components;

    let mut archive: Vec<u8> = Vec::new();
    let mut total: u64 = 0;

    for (rel, abs) in &files {
        let data = std::fs::read(abs).map_err(|e| Error::io(abs, e))?;
        let mut hasher = Sha256::new();
        hasher.update(&data);
        let sha = hex(&hasher.finalize());

        total += data.len() as u64;
        manifest.files.push(FileEntry {
            path: rel.clone(),
            size: data.len() as u64,
            sha256: sha,
            component: component_of(rel),
            executable: rel.ends_with(".exe") || rel.ends_with(".sh"),
        });
        push_entry(&mut archive, rel, &data);
    }
    manifest.total_size = total;

    let payload = zstd::encode_all(&archive[..], ZSTD_LEVEL)
        .map_err(|e| Error::Compression(e.to_string()))?;

    let manifest_json = serde_json::to_vec(&manifest)?;
    let header = Header {
        format_version: FORMAT_VERSION,
        flags: 0,
        manifest_len: manifest_json.len() as u32,
        payload_len: payload.len() as u64,
    };

    let mut f = std::fs::File::create(out).map_err(|e| Error::io(out, e))?;
    f.write_all(&header.to_bytes())
        .map_err(|e| Error::io(out, e))?;
    f.write_all(&manifest_json).map_err(|e| Error::io(out, e))?;
    f.write_all(&payload).map_err(|e| Error::io(out, e))?;
    f.flush().map_err(|e| Error::io(out, e))?;

    Ok(manifest)
}

/// Lowercase hex encoding (kept local to avoid a hex crate dependency).
pub(crate) fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}
