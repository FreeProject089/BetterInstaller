//! Comparing the version a machine has against the one a prerequisite needs.
//!
//! Deliberately NOT semver. What comes back from `--version` is whatever the tool felt
//! like printing: `Python 3.12.9`, `v22.11.0`, `GNU bash, version 5.3.9(1)-release`,
//! `go1.22.3`. A strict semver parser rejects most of those, and rejecting means telling
//! somebody their perfectly good Node is missing.
//!
//! So: find the first dotted number run, compare it component by component, and treat a
//! missing component as zero — `3.12` and `3.12.0` are the same version, because to
//! everyone except a parser they are.

/// Pull a version out of whatever a tool printed. `None` when there is no number at all,
/// which is different from "0" and must not be conflated with it: a tool that answered
/// something unparseable has not told us it is old, it has told us nothing.
pub fn extract_version(text: &str) -> Option<Vec<u64>> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            // Take the whole dotted run from here.
            let start = i;
            let mut saw_dot = false;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                if bytes[i] == b'.' {
                    // A trailing dot ends the run rather than joining what follows —
                    // "version 5." should be 5, not 5 and whatever the next sentence starts with.
                    if i + 1 >= bytes.len() || !bytes[i + 1].is_ascii_digit() {
                        break;
                    }
                    saw_dot = true;
                }
                i += 1;
            }
            let run = &text[start..i];
            let parts: Vec<u64> = run.split('.').filter_map(|x| x.parse().ok()).collect();
            if !parts.is_empty() {
                // A bare number with no dot is a version too (`go1` is unusual but `5` is
                // not), so saw_dot is informational, not a requirement.
                let _ = saw_dot;
                return Some(parts);
            }
        }
        i += 1;
    }
    None
}

/// Compare two version component lists, treating a missing component as zero.
pub fn cmp_version(a: &[u64], b: &[u64]) -> std::cmp::Ordering {
    let n = a.len().max(b.len());
    for i in 0..n {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if x != y {
            return x.cmp(&y);
        }
    }
    std::cmp::Ordering::Equal
}

/// Is release `a` strictly newer than release `b`?
///
/// The ONE version ordering in the workspace (card C-5). There used to be three: this
/// module (prerequisites), `update::is_newer` (which dropped any component that was not a
/// bare number, so `1.2.3-rc1` read as `1.2` and ranked BELOW `1.2.2`) and the installer's
/// `version_gt` (which turned a non-number into 0 and split on `-`/`+`, so `v1.3.0` ranked
/// below `1.2.0` and `1.2.0+5` above `1.2.0`). The update check, the package gate and the
/// "Update" button could each give a different answer about the same two strings.
///
/// The rule is [`extract_version`] + [`cmp_version`]: the first dotted number run, compared
/// component by component, a missing component being zero. A suffix after the run
/// (`-beta.2`, `+5`, `(1)-release`) is not part of the version, so `1.2.0-beta` and `1.2.0`
/// are the SAME release here — publish a pre-release under its own numbers (`1.2.90`), not
/// under a suffix, if it must be offered as an update.
///
/// `a` with no number at all is never newer than anything: a version nobody can read has
/// not been shown to be an upgrade. `b` with no number counts as `0`.
pub fn is_newer(a: &str, b: &str) -> bool {
    let Some(va) = extract_version(a) else {
        return false;
    };
    let vb = extract_version(b).unwrap_or_default();
    cmp_version(&va, &vb) == std::cmp::Ordering::Greater
}

/// `a` and `b` name the same release under [`is_newer`]'s rule (`1.3` = `1.3.0`). Both must
/// contain a version: two strings with no number in them are not "the same version".
pub fn same_release(a: &str, b: &str) -> bool {
    match (extract_version(a), extract_version(b)) {
        (Some(x), Some(y)) => cmp_version(&x, &y) == std::cmp::Ordering::Equal,
        _ => false,
    }
}

/// Does `found` satisfy `min` / `max`? Both bounds are INCLUSIVE.
///
/// Inclusive because that is what people mean. "Needs 3.10 or later" includes 3.10, and a
/// config saying `min_version = "3.10"` that rejected 3.10 would be a trap nobody expects
/// to have to test for.
///
/// An unparseable requirement is treated as no requirement rather than as unsatisfiable:
/// a typo in `min_version` should not make a prerequisite permanently missing on every
/// machine, which is a failure nobody can debug from the outside.
pub fn satisfies(found: &str, min: Option<&str>, max: Option<&str>) -> bool {
    let f = match extract_version(found) {
        Some(v) => v,
        // No number in the output. We know the tool ran, we do not know its version — so a
        // version requirement cannot be confirmed, and claiming it is met would be a guess
        // in the dangerous direction.
        None => return min.is_none() && max.is_none(),
    };
    if let Some(lo) = min.and_then(extract_version) {
        if cmp_version(&f, &lo) == std::cmp::Ordering::Less {
            return false;
        }
    }
    if let Some(hi) = max.and_then(extract_version) {
        if cmp_version(&f, &hi) == std::cmp::Ordering::Greater {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_version_out_of_what_tools_actually_print() {
        // Every one of these is a real string from a real tool. A semver parser rejects
        // most of them, and rejecting means telling somebody their working install is
        // missing.
        let cases = [
            ("Python 3.12.9", vec![3, 12, 9]),
            ("v22.11.0", vec![22, 11, 0]),
            (
                "GNU bash, version 5.3.9(1)-release (x86_64-pc-linux-gnu)",
                vec![5, 3, 9],
            ),
            ("go1.22.3", vec![1, 22, 3]),
            ("git version 2.45.1.windows.1", vec![2, 45, 1]),
            ("5.1.26100.9168", vec![5, 1, 26100, 9168]),
            (
                "Microsoft Windows [version 10.0.26200.9168]",
                vec![10, 0, 26200, 9168],
            ),
        ];
        for (text, want) in cases {
            assert_eq!(extract_version(text), Some(want), "failed on {text:?}");
        }
    }

    #[test]
    fn no_number_is_none_not_zero() {
        // "unknown" is not version 0. Conflating them would report an unparseable answer as
        // an ancient install and offer to replace something that was fine.
        for text in ["", "unknown", "not installed", "command not found"] {
            assert_eq!(extract_version(text), None, "{text:?} produced a version");
        }
    }

    #[test]
    fn a_missing_component_is_zero() {
        assert_eq!(
            cmp_version(&[3, 12], &[3, 12, 0]),
            std::cmp::Ordering::Equal
        );
        assert_eq!(cmp_version(&[3], &[3, 0, 0]), std::cmp::Ordering::Equal);
        assert_eq!(cmp_version(&[3, 12], &[3, 12, 1]), std::cmp::Ordering::Less);
    }

    #[test]
    fn compares_numerically_not_as_text() {
        // The bug every string-compared version check has: "9" > "10" alphabetically.
        assert_eq!(cmp_version(&[1, 10], &[1, 9]), std::cmp::Ordering::Greater);
        assert_eq!(
            cmp_version(&[3, 12, 9], &[3, 9, 12]),
            std::cmp::Ordering::Greater
        );
    }

    #[test]
    fn bounds_are_inclusive() {
        // "Needs 3.10 or later" includes 3.10. A min that rejected its own value would be a
        // trap nobody thinks to test.
        assert!(satisfies("Python 3.10.0", Some("3.10"), None));
        assert!(satisfies("Python 3.10", Some("3.10.0"), None));
        assert!(satisfies("v18.0.0", None, Some("18")));
    }

    #[test]
    fn rejects_outside_the_range() {
        assert!(!satisfies("Python 3.9.18", Some("3.10"), None));
        assert!(!satisfies("v22.11.0", None, Some("20")));
        assert!(satisfies("v20.11.0", Some("18"), Some("22")));
        assert!(!satisfies("v23.0.0", Some("18"), Some("22")));
    }

    #[test]
    fn an_unparseable_requirement_is_no_requirement() {
        // A typo in min_version must not make a prerequisite permanently missing on every
        // machine — a failure nobody can debug from the outside.
        assert!(satisfies("Python 3.12.0", Some("latest"), None));
        assert!(satisfies("Python 3.12.0", None, Some("")));
    }

    /// The cases `update::is_newer` passed, run against the one comparator that replaced it.
    #[test]
    fn release_order_keeps_what_the_update_check_always_answered() {
        assert!(is_newer("1.2.0", "1.1.9"));
        assert!(is_newer("2.0.0", "1.9.9"));
        assert!(is_newer("1.0.1", "1.0"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("1.0.0", "1.0.1"));
        assert!(is_newer("1.10.0", "1.9.0"));
        assert!(!is_newer("1.9.0", "1.10.0"));
        assert!(is_newer("1.0.10", "1.0.9"));
        assert!(!is_newer("1.2", "1.2.0"));
        assert!(!is_newer("1.2.0", "1.2"));
        assert!(same_release("1.3", "1.3.0"));
        assert!(!same_release("1.2.5", "1.3.0"));
    }

    /// Card C-5: every pair on which the three comparators that used to exist disagreed,
    /// with the answer that now holds everywhere (update check, package gate, "Update"
    /// button). Each line names who got it wrong.
    #[test]
    fn the_pairs_the_three_comparators_disagreed_on() {
        // update::is_newer dropped "3-rc1" and compared 1.2 with 1.2.2: an rc of the next
        // patch looked OLDER than the current one.
        assert!(is_newer("1.2.3-rc1", "1.2.2"));
        // installer's version_gt read "v1" as 0: a v-prefixed release was never newer.
        assert!(is_newer("v1.3.0", "1.2.0"));
        assert!(!is_newer("1.2.0", "v1.3.0"));
        // version_gt split on '+' and read build metadata as a fourth component.
        assert!(!is_newer("1.2.0+5", "1.2.0"));
        assert!(same_release("1.2.0+5", "1.2.0"));
        // Both update::is_newer (1.2.2) and version_gt (1.2.0.0.2) ranked a second beta of
        // 1.2.0 above the 1.2.0 release itself. A suffix is not part of the version.
        assert!(!is_newer("1.2.0-beta.2", "1.2.0"));
        assert!(same_release("1.2.0-beta.2", "1.2.0"));
        // The one all three agreed on, pinned so it stays deliberate.
        assert!(same_release("1.2.0-beta", "1.2.0"));
        // update::is_newer skipped the empty component of "1..2" and read 1.2.
        assert!(!is_newer("1..2", "1.1"));
        // No number: never newer, never "the same".
        assert!(!is_newer("latest", "1.0"));
        assert!(!same_release("latest", "latest"));
        assert!(is_newer("1.0", ""));
    }

    #[test]
    fn an_unparseable_version_fails_only_when_something_was_required() {
        // The tool ran but said nothing we understand. With no requirement that is fine;
        // with one, it cannot be confirmed, and confirming it anyway is a guess in the
        // direction that breaks things.
        assert!(satisfies("unknown", None, None));
        assert!(!satisfies("unknown", Some("1.0"), None));
    }
}
