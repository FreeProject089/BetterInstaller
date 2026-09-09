//! The format docs have to say what the format does.
//!
//! This is here because they did not, and nobody noticed for the life of the format.
//! BPKG-FORMAT.md and SIGNING.md both stated the signature covered
//! `header[6..] ⧺ manifest ⧺ payload` — everything but the magic. `sign_package` signed
//! `data[HEADER_LEN..mp_end]`: manifest and payload, header excluded entirely.
//!
//! The documented promise was STRONGER than the code's, which is the direction nobody
//! catches. A doc that under-promises gets corrected the first time somebody relies on it. A
//! doc that over-promises is only found by somebody auditing the code against it — and if
//! they read the doc first, they may not look.
//!
//! So: the two facts a reader would act on, checked against the constants, in four files.

use std::path::PathBuf;

fn doc(name: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs")
        .join(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("reading {}: {e}", p.display()))
}

#[test]
fn the_version_cell_matches_the_constant() {
    let want = format!(
        "| 6 | 2 | format_version | `{}` |",
        bpkg_core::package::format::FORMAT_VERSION
    );
    for f in ["BPKG-FORMAT.md", "BPKG-FORMAT.fr.md"] {
        assert!(
            doc(f).contains(&want),
            "{f} does not carry the current format_version — expected the row {want:?}",
        );
    }
}

#[test]
fn the_docs_state_the_range_the_code_actually_signs() {
    // The code signs bytes 0 .. HEADER_LEN+N+M — the header included, magic and all. Any doc
    // describing a narrower range is describing a weaker guarantee than the one shipped, or
    // (as it was) a stronger one than the code gave.
    let want = "0 .. 24+N+M";
    assert_eq!(
        bpkg_core::package::format::HEADER_LEN,
        24,
        "the header grew; every doc saying 24+N+M now says the wrong number",
    );
    for f in [
        "BPKG-FORMAT.md",
        "BPKG-FORMAT.fr.md",
        "SIGNING.md",
        "SIGNING.fr.md",
    ] {
        let body = doc(f);
        assert!(
            body.contains(want),
            "{f} does not state the signed range as {want:?}"
        );
        // The old claim may only survive as history — a sentence saying it was never true.
        // Standing alone, as a description of what happens, it is the bug this test exists
        // for, so the file has to acknowledge it in the past tense somewhere.
        if body.contains("header[6..]") {
            assert!(
                body.contains("never did")
                    || body.contains("did not")
                    || body.contains("n'a jamais")
                    || body.contains("était faux"),
                "{f} still describes the signature as covering header[6..] as if it were true",
            );
        }
    }
}
