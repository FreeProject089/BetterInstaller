//! Embeds every engine catalogue in `locales/` so a new language is a new FILE, not code.
//!
//! Generates `$OUT_DIR/builtin_locales.rs`: a sorted `&[(code, toml_text)]`. The file stem
//! is the language code (`fr.toml`, `pt-BR.toml`), and a stem that is not a plausible
//! BCP-47 tag fails the build here rather than becoming a language nobody can select.

use std::fmt::Write as _;
use std::path::Path;

fn main() {
    let dir = Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("locales");
    // The directory itself, so ADDING a file re-runs this script. Edits to an existing file
    // are tracked by include_str! on its own.
    println!("cargo:rerun-if-changed={}", dir.display());

    let mut entries: Vec<(String, String)> = Vec::new();
    for e in std::fs::read_dir(&dir).expect("crates/bpkg-core/locales must exist") {
        let p = e.expect("readable locales entry").path();
        if p.extension().and_then(|x| x.to_str()) != Some("toml") {
            continue;
        }
        let code = p.file_stem().unwrap().to_string_lossy().to_string();
        assert!(
            is_tag(&code),
            "locales/{code}.toml: the file name must be a language code like fr or pt-BR"
        );
        entries.push((code, p.to_string_lossy().replace('\\', "/")));
    }
    entries.sort();
    assert!(
        entries.iter().any(|(c, _)| c == "en"),
        "locales/en.toml is the fallback every other language relies on"
    );

    let mut out = String::from("pub(crate) static BUILTIN: &[(&str, &str)] = &[\n");
    for (code, path) in &entries {
        writeln!(out, "    ({code:?}, include_str!({path:?})),").unwrap();
    }
    out.push_str("];\n");
    let dest = Path::new(&std::env::var("OUT_DIR").unwrap()).join("builtin_locales.rs");
    std::fs::write(dest, out).unwrap();
}

/// `ll`, `lll`, optionally followed by `-subtag` parts of 2 to 8 alphanumerics.
fn is_tag(s: &str) -> bool {
    let mut parts = s.split('-');
    let lang = parts.next().unwrap_or("");
    (2..=3).contains(&lang.len())
        && lang.chars().all(|c| c.is_ascii_alphabetic())
        && parts.all(|p| (2..=8).contains(&p.len()) && p.chars().all(|c| c.is_ascii_alphanumeric()))
}
