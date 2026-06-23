//! Self-extracting installer support.
//!
//! `bpkg build` appends a project's `installer.toml` + its `.bpkg` to a copy of
//! the prebuilt installer exe, followed by a fixed 24-byte trailer. At runtime
//! the installer reads its OWN file, finds the trailer, and loads the embedded
//! config + package — so one prebuilt binary serves every project, no recompile.
//!
//! ```text
//!  … original installer.exe …
//!  [config bytes]
//!  [bpkg bytes]
//!  [trailer: config_len(u64 LE) | bpkg_len(u64 LE) | MAGIC(8) ]   ← last 24 bytes
//! ```

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::error::{Error, Result};

const TRAILER_MAGIC: &[u8; 8] = b"BPKGSFX1";
const TRAILER_LEN: usize = 24; // config_len(8) + bpkg_len(8) + magic(8)

/// The payload recovered from a stamped installer.
pub struct Embedded {
    pub config: Vec<u8>,
    pub bpkg: Vec<u8>,
}

/// Produce a self-extracting installer at `out` = `base_exe` + config + bpkg + trailer.
pub fn stamp(base_exe: &Path, config: &[u8], bpkg_bytes: &[u8], out: &Path) -> Result<()> {
    let base = std::fs::read(base_exe).map_err(|e| Error::io(base_exe, e))?;
    let mut f = std::fs::File::create(out).map_err(|e| Error::io(out, e))?;
    f.write_all(&base).map_err(|e| Error::io(out, e))?;
    f.write_all(config).map_err(|e| Error::io(out, e))?;
    f.write_all(bpkg_bytes).map_err(|e| Error::io(out, e))?;

    let mut trailer = Vec::with_capacity(TRAILER_LEN);
    trailer.extend_from_slice(&(config.len() as u64).to_le_bytes());
    trailer.extend_from_slice(&(bpkg_bytes.len() as u64).to_le_bytes());
    trailer.extend_from_slice(TRAILER_MAGIC);
    f.write_all(&trailer).map_err(|e| Error::io(out, e))?;
    Ok(())
}

/// Read the embedded config + package from `exe`, or `None` if it isn't stamped
/// (the normal case when running the dev binary directly).
pub fn read_embedded(exe: &Path) -> Result<Option<Embedded>> {
    let mut f = std::fs::File::open(exe).map_err(|e| Error::io(exe, e))?;
    let total_len = f.metadata().map_err(Error::IoBare)?.len();
    if total_len < TRAILER_LEN as u64 {
        return Ok(None);
    }

    f.seek(SeekFrom::End(-(TRAILER_LEN as i64)))
        .map_err(Error::IoBare)?;
    let mut trailer = [0u8; TRAILER_LEN];
    f.read_exact(&mut trailer).map_err(Error::IoBare)?;
    if &trailer[16..24] != TRAILER_MAGIC {
        return Ok(None); // not a stamped installer
    }

    let config_len = u64::from_le_bytes(trailer[0..8].try_into().unwrap());
    let bpkg_len = u64::from_le_bytes(trailer[8..16].try_into().unwrap());
    let blob = config_len + bpkg_len + TRAILER_LEN as u64;
    if blob > total_len {
        return Err(Error::Corrupt("embedded blob larger than file".into()));
    }

    f.seek(SeekFrom::Start(total_len - blob))
        .map_err(Error::IoBare)?;
    let mut config = vec![0u8; config_len as usize];
    f.read_exact(&mut config).map_err(Error::IoBare)?;
    let mut bpkg = vec![0u8; bpkg_len as usize];
    f.read_exact(&mut bpkg).map_err(Error::IoBare)?;
    Ok(Some(Embedded { config, bpkg }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamp_and_read_roundtrip() {
        let dir = std::env::temp_dir().join(format!("bpkg-embed-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let base = dir.join("base.exe");
        std::fs::write(&base, b"MZ...fake installer bytes...").unwrap();

        let config = b"[app]\nname = \"X\"\n";
        let bpkg = b"\x00\x01\x02 bpkg bytes \xff\xfe";
        let out = dir.join("stamped.exe");
        stamp(&base, config, bpkg, &out).unwrap();

        assert!(std::fs::metadata(&out).unwrap().len() > std::fs::metadata(&base).unwrap().len());
        let emb = read_embedded(&out)
            .unwrap()
            .expect("stamped exe should read back");
        assert_eq!(emb.config, config);
        assert_eq!(emb.bpkg, bpkg);

        // An un-stamped file reads back as None (the dev-run case).
        assert!(read_embedded(&base).unwrap().is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
