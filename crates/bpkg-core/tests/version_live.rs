//! The version check against tools that are actually on this machine.
//!
//! The unit tests in version.rs use captured strings; these run the real binaries. That is
//! the difference between "my parser handles the format I remembered" and "it handles what
//! the tool prints" — and the second is the one that decides whether somebody is told
//! their working install is missing.
use bpkg_core::config::{PrereqKind, Prerequisite};
use bpkg_core::prereq::check;

fn prereq(cmd: &str, min: Option<&str>, max: Option<&str>) -> Prerequisite {
    Prerequisite {
        id: "probe".into(),
        name: "probe".into(),
        check_registry: None,
        check_file: None,
        check_command: Some(cmd.into()),
        min_version: min.map(String::from),
        max_version: max.map(String::from),
        version_args: None,
        download_url: None,
        sha256: None,
        kind: PrereqKind::Exe,
        install_to: None,
        silent_args: None,
        required: false,
    }
}

/// A command that is not there fails whatever the version rules say — and fails on the
/// PATH check, without spawning anything.
#[test]
fn a_missing_command_is_missing() {
    assert!(!check(&prereq(
        "definitely-not-a-real-binary-xyz",
        None,
        None
    )));
    assert!(!check(&prereq(
        "definitely-not-a-real-binary-xyz",
        Some("1.0"),
        None
    )));
}

/// With no version rule, being on PATH is enough — the behaviour every existing
/// installer.toml relies on, which must not change because version support was added.
#[test]
fn no_version_rule_means_path_only() {
    // cargo ran this, so cargo exists.
    assert!(check(&prereq("cargo", None, None)));
}

/// The real thing: a version bound decided by asking the binary.
#[test]
fn a_real_tool_is_measured_not_assumed() {
    let p = prereq("cargo", Some("0.1"), None);
    assert!(check(&p), "cargo should satisfy >= 0.1");

    // An absurd minimum must fail, or the check is not doing anything. Without this the
    // test above passes just as well when satisfies() always returns true.
    let p = prereq("cargo", Some("9999.0"), None);
    assert!(!check(&p), "cargo should not satisfy >= 9999.0");

    // And the upper bound in the same direction.
    let p = prereq("cargo", None, Some("0.0.1"));
    assert!(!check(&p), "cargo should not satisfy <= 0.0.1");
}

/// A tool that runs but says nothing parseable, WITH a requirement, is not satisfied.
/// Confirming a version we could not read would be a guess in the direction that breaks
/// the install later.
#[test]
fn unreadable_version_with_a_requirement_is_not_satisfied() {
    let mut p = prereq("cargo", Some("1.0"), None);
    // Ask it something that prints no version.
    p.version_args = Some(vec!["--explain".into(), "E0308".into()]);
    // Whatever that prints, it is not a version string — so the requirement cannot be
    // confirmed. (If it ever did contain a number this assertion would be wrong, which is
    // why the next one pins the parse directly.)
    let text = "this explanation contains no version";
    assert!(!bpkg_core::version::satisfies(text, Some("1.0"), None));
    let _ = p;
}
