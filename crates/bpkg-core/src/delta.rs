//! Binary delta patches (bsdiff via `qbsdiff`). A patch turns an old byte blob
//! into a new one, typically far smaller than the new blob — so an update can
//! download a patch instead of the whole package.

use qbsdiff::{Bsdiff, Bspatch};

use crate::error::{Error, Result};

/// Produce a patch that transforms `old` into `new`.
pub fn make_delta(old: &[u8], new: &[u8]) -> Result<Vec<u8>> {
    let mut patch = Vec::new();
    Bsdiff::new(old, new)
        .compare(&mut patch)
        .map_err(|e| Error::Other(format!("bsdiff: {e}")))?;
    Ok(patch)
}

/// Apply a patch to `old`, producing the new bytes.
pub fn apply_delta(old: &[u8], patch: &[u8]) -> Result<Vec<u8>> {
    let bspatch = Bspatch::new(patch).map_err(|e| Error::Other(format!("bspatch: {e}")))?;
    let mut out = Vec::with_capacity(bspatch.hint_target_size() as usize);
    bspatch
        .apply(old, &mut out)
        .map_err(|e| Error::Other(format!("bspatch apply: {e}")))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_roundtrip_and_smaller() {
        // Two similar blobs (a small edit + an appended tail).
        let old: Vec<u8> = (0..20_000u32).map(|i| (i % 251) as u8).collect();
        let mut new = old.clone();
        for i in (0..new.len()).step_by(500) {
            new[i] = new[i].wrapping_add(1);
        }
        new.extend_from_slice(b"...a freshly appended tail of new content...");

        let patch = make_delta(&old, &new).unwrap();
        let rebuilt = apply_delta(&old, &patch).unwrap();
        assert_eq!(rebuilt, new, "patch must rebuild the new blob exactly");
        assert!(
            patch.len() < new.len(),
            "patch ({}) should be smaller than the new blob ({})",
            patch.len(),
            new.len()
        );
    }
}
