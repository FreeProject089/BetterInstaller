//! The `.bpkg` package format: a signed-capable, zstd-compressed container with
//! an uncompressed JSON manifest header for fast inspection.

pub mod format;
pub mod reader;
pub mod writer;

pub use reader::Package;
pub use writer::create_from_dir;

use std::path::Path;

use ed25519_dalek::SigningKey;

use crate::error::{Error, Result};
use format::{Header, FLAG_SIGNED, HEADER_LEN};

/// Sign an existing `.bpkg` in place: sign the manifest+payload bytes, set the
/// signed flag, and append the 64-byte Ed25519 signature after the payload.
pub fn sign_package(path: &Path, sk: &SigningKey) -> Result<()> {
    let mut data = std::fs::read(path).map_err(|e| Error::io(path, e))?;
    let header = Header::from_bytes(&data)?;
    if header.flags & FLAG_SIGNED != 0 {
        return Err(Error::Other("package is already signed".into()));
    }
    let mp_start = HEADER_LEN;
    let mp_end = HEADER_LEN + header.manifest_len as usize + header.payload_len as usize;
    if mp_end > data.len() {
        return Err(Error::Corrupt("payload truncated".into()));
    }

    let sig = crate::sign::sign_message(sk, &data[mp_start..mp_end]);
    // Flip the signed bit in the header (flags live at bytes 8..10).
    let new_flags = header.flags | FLAG_SIGNED;
    data[8..10].copy_from_slice(&new_flags.to_le_bytes());
    // Drop anything past the payload (there shouldn't be any), then append the sig.
    data.truncate(mp_end);
    data.extend_from_slice(&sig);

    std::fs::write(path, &data).map_err(|e| Error::io(path, e))?;
    Ok(())
}
