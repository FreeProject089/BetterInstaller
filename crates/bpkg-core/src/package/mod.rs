//! The `.bpkg` package format: a signed-capable, zstd-compressed container with
//! an uncompressed JSON manifest header for fast inspection.

pub mod format;
pub mod reader;
pub mod writer;

pub use reader::Package;
pub use writer::create_from_dir;
