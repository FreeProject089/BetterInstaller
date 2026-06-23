//! On-disk layout of a `.bpkg` container.
//!
//! ```text
//!  offset  size  field
//!  ------  ----  -----------------------------------------------------------
//!     0     6    magic = b"BPKG\x1a\x00"
//!     6     2    format_version (u16 LE)
//!     8     2    flags (u16 LE)  — bit0: signed (reserved for Phase 7)
//!    10     2    reserved (u16, 0)
//!    12     4    manifest_len (u32 LE)        — JSON, uncompressed
//!    16     8    payload_len (u64 LE)         — zstd-compressed inner archive
//!    24     N    manifest bytes
//!  24+N     M    payload bytes
//! ```
//!
//! The inner archive (pre-compression) is a flat sequence of entries:
//! `path_len(u32) | path(utf8) | data_len(u64) | data`.

pub const MAGIC: &[u8; 6] = b"BPKG\x1a\x00";
pub const FORMAT_VERSION: u16 = 1;
pub const HEADER_LEN: usize = 24;

/// `flags` bit 0: an Ed25519 signature (64 bytes) follows the payload.
pub const FLAG_SIGNED: u16 = 0x0001;
/// Length of the appended Ed25519 signature.
pub const SIGNATURE_LEN: usize = 64;

/// zstd level — high ratio, the package is built once and downloaded many times.
pub const ZSTD_LEVEL: i32 = 19;

#[derive(Debug, Clone, Copy)]
pub struct Header {
    pub format_version: u16,
    pub flags: u16,
    pub manifest_len: u32,
    pub payload_len: u64,
}

impl Header {
    pub fn to_bytes(&self) -> [u8; HEADER_LEN] {
        let mut b = [0u8; HEADER_LEN];
        b[0..6].copy_from_slice(MAGIC);
        b[6..8].copy_from_slice(&self.format_version.to_le_bytes());
        b[8..10].copy_from_slice(&self.flags.to_le_bytes());
        // bytes 10..12 reserved (0)
        b[12..16].copy_from_slice(&self.manifest_len.to_le_bytes());
        b[16..24].copy_from_slice(&self.payload_len.to_le_bytes());
        b
    }

    pub fn from_bytes(b: &[u8]) -> crate::error::Result<Header> {
        use crate::error::Error;
        if b.len() < HEADER_LEN {
            return Err(Error::Corrupt("file shorter than header".into()));
        }
        if &b[0..6] != MAGIC {
            return Err(Error::BadMagic);
        }
        let format_version = u16::from_le_bytes([b[6], b[7]]);
        if format_version != FORMAT_VERSION {
            return Err(Error::UnsupportedVersion(format_version, FORMAT_VERSION));
        }
        let flags = u16::from_le_bytes([b[8], b[9]]);
        let manifest_len = u32::from_le_bytes([b[12], b[13], b[14], b[15]]);
        let payload_len =
            u64::from_le_bytes([b[16], b[17], b[18], b[19], b[20], b[21], b[22], b[23]]);
        Ok(Header {
            format_version,
            flags,
            manifest_len,
            payload_len,
        })
    }
}
