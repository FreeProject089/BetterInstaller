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
//!  [Authenticode certificate table, once the stamped exe is code-signed]
//! ```
//!
//! Code-signing the STAMPED exe is what authenticates the config: `installer.toml` rides
//! outside the Ed25519-signed package, so without it a re-stamped setup can carry any
//! `public_key` it likes and show "Signed & verified". `signtool` appends the certificate
//! table at the end of the file (after padding it to 8 bytes) and the Authenticode hash
//! covers everything before it — config and package included. The trailer is then no
//! longer the last 24 bytes, so the reader also looks for it just before that table.

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

    let end = match trailer_end(&mut f, total_len)? {
        Some(end) => end,
        None => return Ok(None), // not a stamped installer
    };
    let trailer = read_at::<TRAILER_LEN>(&mut f, end - TRAILER_LEN as u64)?;

    let config_len = u64::from_le_bytes(trailer[0..8].try_into().unwrap());
    let bpkg_len = u64::from_le_bytes(trailer[8..16].try_into().unwrap());
    // Checked: two u64s read from the file can sum past u64::MAX and wrap to something
    // small enough to pass the bound below.
    let blob = config_len
        .checked_add(bpkg_len)
        .and_then(|n| n.checked_add(TRAILER_LEN as u64))
        .filter(|&n| n <= end)
        .ok_or_else(|| Error::Corrupt("embedded blob larger than file".into()))?;

    f.seek(SeekFrom::Start(end - blob)).map_err(Error::IoBare)?;
    let mut config = vec![0u8; config_len as usize];
    f.read_exact(&mut config).map_err(Error::IoBare)?;
    let mut bpkg = vec![0u8; bpkg_len as usize];
    f.read_exact(&mut bpkg).map_err(Error::IoBare)?;
    Ok(Some(Embedded { config, bpkg }))
}

/// Where the stamped data ends: the end of the file, or — on a code-signed exe — the start
/// of the Authenticode certificate table, give or take the up-to-7 zero bytes of alignment
/// `signtool` inserts before it.
fn trailer_end(f: &mut std::fs::File, total_len: u64) -> Result<Option<u64>> {
    if magic_ends_at(f, total_len)? {
        return Ok(Some(total_len));
    }
    let Some((offset, size)) = pe_certificate_table(f)? else {
        return Ok(None);
    };
    // Only a table that IS the end of the file: anything after it would be data the
    // signature does not cover, and nothing we wrote.
    if offset.checked_add(size) != Some(total_len) {
        return Ok(None);
    }
    for pad in 0..8u64 {
        let Some(end) = offset.checked_sub(pad) else {
            break;
        };
        if end < TRAILER_LEN as u64 {
            break;
        }
        let gap_is_zero = pad == 0 || {
            f.seek(SeekFrom::Start(end)).map_err(Error::IoBare)?;
            let mut gap = vec![0u8; pad as usize];
            f.read_exact(&mut gap).map_err(Error::IoBare)?;
            gap.iter().all(|&b| b == 0)
        };
        if gap_is_zero && magic_ends_at(f, end)? {
            return Ok(Some(end));
        }
    }
    Ok(None)
}

fn magic_ends_at(f: &mut std::fs::File, end: u64) -> Result<bool> {
    if end < TRAILER_LEN as u64 {
        return Ok(false);
    }
    Ok(&read_at::<8>(f, end - 8)? == TRAILER_MAGIC)
}

fn read_at<const N: usize>(f: &mut std::fs::File, pos: u64) -> Result<[u8; N]> {
    let mut buf = [0u8; N];
    f.seek(SeekFrom::Start(pos)).map_err(Error::IoBare)?;
    f.read_exact(&mut buf).map_err(Error::IoBare)?;
    Ok(buf)
}

/// The PE security data directory (`IMAGE_DIRECTORY_ENTRY_SECURITY`): file offset and
/// size of the Authenticode certificate table, when the file is a PE that has one.
fn pe_certificate_table(f: &mut std::fs::File) -> Result<Option<(u64, u64)>> {
    let dos = match read_at::<64>(f, 0) {
        Ok(d) => d,
        Err(_) => return Ok(None),
    };
    if &dos[0..2] != b"MZ" {
        return Ok(None);
    }
    let pe = u32::from_le_bytes(dos[60..64].try_into().unwrap()) as u64;
    // "PE\0\0" (4) + COFF header (20) + the optional header's magic (2).
    let Ok(head) = read_at::<26>(f, pe) else {
        return Ok(None);
    };
    if &head[0..4] != b"PE\0\0" {
        return Ok(None);
    }
    let opt_size = u16::from_le_bytes([head[20], head[21]]) as u64;
    let opt = pe + 24;
    // Offsets inside the optional header of NumberOfRvaAndSizes and the data directories.
    let (count_at, dirs_at) = match u16::from_le_bytes([head[24], head[25]]) {
        0x10b => (92u64, 96u64),   // PE32
        0x20b => (108u64, 112u64), // PE32+
        _ => return Ok(None),
    };
    const SECURITY: u64 = 4;
    let entry = dirs_at + SECURITY * 8;
    if entry + 8 > opt_size {
        return Ok(None);
    }
    let count = u32::from_le_bytes(read_at::<4>(f, opt + count_at)?);
    if (count as u64) <= SECURITY {
        return Ok(None);
    }
    let dir = read_at::<8>(f, opt + entry)?;
    let offset = u32::from_le_bytes(dir[0..4].try_into().unwrap()) as u64;
    let size = u32::from_le_bytes(dir[4..8].try_into().unwrap()) as u64;
    Ok((offset != 0 && size != 0).then_some((offset, size)))
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

    /// A minimal PE32+ header: enough for `pe_certificate_table` to find the security
    /// directory at its real offset.
    fn fake_pe() -> Vec<u8> {
        let mut b = vec![0u8; 0x200];
        b[0..2].copy_from_slice(b"MZ");
        b[60..64].copy_from_slice(&0x80u32.to_le_bytes());
        b[0x80..0x84].copy_from_slice(b"PE\0\0");
        b[0x80 + 20..0x80 + 22].copy_from_slice(&240u16.to_le_bytes()); // SizeOfOptionalHeader
        let opt = 0x98;
        b[opt..opt + 2].copy_from_slice(&0x20bu16.to_le_bytes());
        b[opt + 108..opt + 112].copy_from_slice(&16u32.to_le_bytes());
        b
    }

    /// What `signtool sign` does to a stamped setup: pad to 8 bytes, append the
    /// certificate table, point the security directory at it.
    fn authenticode_sign(stamped: &mut Vec<u8>, cert_len: usize) {
        while !stamped.len().is_multiple_of(8) {
            stamped.push(0);
        }
        let offset = stamped.len() as u32;
        stamped.extend(std::iter::repeat_n(0xA5, cert_len));
        let entry = 0x98 + 112 + 4 * 8;
        stamped[entry..entry + 4].copy_from_slice(&offset.to_le_bytes());
        stamped[entry + 4..entry + 8].copy_from_slice(&(cert_len as u32).to_le_bytes());
    }

    /// The docs tell publishers to Authenticode-sign `*-Setup.exe`, and it is the only thing
    /// that authenticates the embedded config. Signing appends the certificate table, the
    /// trailer stops being the last 24 bytes, and a signed setup used to read as "not
    /// stamped" — falling back to dev mode and failing to start.
    #[test]
    fn a_code_signed_setup_still_finds_its_payload() {
        let dir = std::env::temp_dir().join(format!("bpkg-embed-sig-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let base = dir.join("base.exe");
        std::fs::write(&base, fake_pe()).unwrap();
        let (config, bpkg) = (
            b"[app]\nname = \"X\"\n".as_slice(),
            b"\x01bpkg\xff".as_slice(),
        );

        for cert_len in [8usize, 1500, 4099] {
            let out = dir.join(format!("signed-{cert_len}.exe"));
            stamp(&base, config, bpkg, &out).unwrap();
            let mut bytes = std::fs::read(&out).unwrap();
            authenticode_sign(&mut bytes, cert_len);
            std::fs::write(&out, &bytes).unwrap();

            let emb = read_embedded(&out)
                .unwrap()
                .expect("a signed setup must still find its config and package");
            assert_eq!(emb.config, config);
            assert_eq!(emb.bpkg, bpkg);
        }

        // Signed but never stamped: still "not stamped", not an error.
        let mut plain = fake_pe();
        authenticode_sign(&mut plain, 64);
        std::fs::write(&base, &plain).unwrap();
        assert!(read_embedded(&base).unwrap().is_none());

        // Bytes after the certificate table are not ours: not stamped.
        let out = dir.join("trailing.exe");
        stamp(&dir.join("base.exe"), config, bpkg, &out).unwrap();
        let mut bytes = std::fs::read(&out).unwrap();
        authenticode_sign(&mut bytes, 64);
        bytes.extend_from_slice(b"appended later");
        std::fs::write(&out, &bytes).unwrap();
        assert!(read_embedded(&out).unwrap().is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Lengths that wrap when summed must be refused, not trusted.
    #[test]
    fn trailer_lengths_that_overflow_are_corrupt() {
        let dir = std::env::temp_dir().join(format!("bpkg-embed-ovf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("x.exe");
        let mut b = b"MZ padding".to_vec();
        b.extend_from_slice(&u64::MAX.to_le_bytes());
        b.extend_from_slice(&30u64.to_le_bytes());
        b.extend_from_slice(TRAILER_MAGIC);
        std::fs::write(&p, &b).unwrap();
        assert!(read_embedded(&p).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
