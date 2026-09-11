//! The install/update path, exercised end to end on a real directory.
//!
//! `update.rs` already covers the happy path and a rollback from a corrupt archive. A
//! corrupt archive fails at `Package::open`-ish depth — before a single destination file
//! has been touched — so it proves the error is propagated, not that a rollback restores
//! anything. Everything here starts from a directory that was already being written into,
//! or checks a claim the code makes in a comment and nothing verifies.

use bpkg_core::manifest::AppMeta;
use bpkg_core::package::{self, Package};
use bpkg_core::update::apply_package_update;

fn app() -> AppMeta {
    AppMeta {
        id: "test".into(),
        name: "Test".into(),
        version: "1".into(),
        publisher: "p".into(),
        homepage: None,
        platforms: vec!["windows".into()],
    }
}

/// A scratch directory keyed by test name, so the tests can run in parallel without
/// fighting over one path (they all live in the same process).
fn scratch(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("bpkg-it-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Make `path` unwritable, and report whether it actually IS.
///
/// The two rollback tests need a destination file that fails to open for writing, and they
/// used to assume `set_readonly(true)` delivered one. It does on Windows, where the read-only
/// attribute is checked for everybody. On Unix it clears the write bits — and root has
/// CAP_DAC_OVERRIDE, so the write succeeds regardless. The premise evaporates, the update
/// completes, and the test fails with a sentence about the rollback being broken.
///
/// So the lock is PROVED, by the only question that matters: can this process still write
/// there. A probe write rather than a `geteuid() == 0` check, because that is the property
/// the tests depend on — and it covers a container with capabilities dropped, a filesystem
/// mounted so modes are ignored, and Windows identically.
#[must_use]
fn lock_file(path: &std::path::Path) -> bool {
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(path, perms).unwrap();
    // Appending, not truncating: a probe that emptied the file when it DID succeed would
    // destroy the very contents the assertions below compare against.
    std::fs::OpenOptions::new().append(true).open(path).is_err()
}

/// Undo `lock_file`, so the scratch directory can be removed by the next run.
fn unlock_file(path: &std::path::Path) {
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    std::fs::set_permissions(path, perms).unwrap();
}

fn write(path: &std::path::Path, body: &[u8]) {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).unwrap();
    }
    std::fs::write(path, body).unwrap();
}

fn pack(src: &std::path::Path, out: &std::path::Path) {
    package::create_from_dir(src, app(), vec![], |_| None, out).unwrap();
}

fn install(bpkg: &std::path::Path, dest: &std::path::Path) {
    Package::open(bpkg)
        .unwrap()
        .install_with_progress(dest, None, |_, _, _| {})
        .unwrap();
}

/// The backup lives beside the install dir as `<name>.bak`. Nothing should outlive the
/// call — a stray copy of the whole app next to itself is both a disk cost and a thing a
/// later "repair" could pick up.
fn backup_of(install_dir: &std::path::Path) -> std::path::PathBuf {
    let name = install_dir
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    install_dir.parent().unwrap().join(format!("{name}.bak"))
}

/// `apply_package_update` says it refuses an untrusted package "before we snapshot or
/// write anything". That ordering is the whole value of the check: if it snapshotted
/// first, a rejected update would still have copied the entire install dir to `.bak` and
/// left it there, and if it wrote first there would be nothing left to protect.
#[test]
fn a_package_signed_by_the_wrong_key_touches_nothing() {
    let base = scratch("wrongkey");
    let (v1, v2) = (base.join("v1"), base.join("v2"));
    write(&v1.join("app.exe"), b"VERSION ONE");
    write(&v2.join("app.exe"), b"VERSION TWO");

    let (p1, p2) = (base.join("v1.bpkg"), base.join("v2.bpkg"));
    pack(&v1, &p1);
    pack(&v2, &p2);

    // v2 is signed — but by a key the installer does not trust.
    let attacker = bpkg_core::sign::generate();
    let publisher = bpkg_core::sign::generate();
    package::sign_package(&p2, &attacker).unwrap();

    let dir = base.join("install");
    install(&p1, &dir);

    let err = apply_package_update(&p2, &dir, None, Some(&publisher.verifying_key()))
        .expect_err("a package signed by an untrusted key must be refused");
    assert!(
        err.to_string().contains("signature"),
        "unexpected error: {err}"
    );

    assert_eq!(
        std::fs::read(dir.join("app.exe")).unwrap(),
        b"VERSION ONE",
        "the refused update overwrote the installed files"
    );
    assert!(
        !backup_of(&dir).exists(),
        "refused before writing, yet a .bak snapshot was left behind"
    );
}

/// The rollback path, entered from a directory that was genuinely half-written rather
/// than from an archive that failed to open.
///
/// The lever is a read-only destination file: extraction overwrites entries in order, so
/// by the time it reaches the locked one it has already replaced others with v2 content.
/// That is the shape of the case rollback exists for — an update that dies partway — and
/// it is the one a corrupt-archive test cannot reach.
#[test]
fn an_update_that_dies_partway_restores_every_file_it_had_already_replaced() {
    let base = scratch("partial");
    let (v1, v2) = (base.join("v1"), base.join("v2"));
    // Enough files that at least one is written before the locked one is reached,
    // whatever order the manifest ends up in.
    for name in ["a.txt", "b.txt", "locked.txt", "y.txt", "z.txt"] {
        write(&v1.join(name), b"VERSION ONE");
        write(&v2.join(name), b"VERSION TWO");
    }
    // A nested file too: rollback copies recursively, and a flat fixture would not notice
    // if it stopped at the top level.
    write(&v1.join("sub/nested.txt"), b"VERSION ONE");
    write(&v2.join("sub/nested.txt"), b"VERSION TWO");

    let (p1, p2) = (base.join("v1.bpkg"), base.join("v2.bpkg"));
    pack(&v1, &p1);
    pack(&v2, &p2);

    let dir = base.join("install");
    install(&p1, &dir);

    // Lock one destination file so writing over it fails mid-extraction — and check that
    // the lock took. As root it does not, and without this the test blames the rollback for
    // something that happened to the tester.
    let locked = dir.join("locked.txt");
    if !lock_file(&locked) {
        unlock_file(&locked);
        eprintln!(
            "SKIP an_update_that_dies_partway_restores_every_file_it_had_already_replaced: \
             this process can write to a read-only file (running as root?), so the \
             mid-extraction failure this test needs cannot be provoked here"
        );
        return;
    }

    let result = apply_package_update(&p2, &dir, None, None);

    // Unlock first, unconditionally — otherwise a failure here leaves an undeletable
    // file in the temp dir for every future run.
    unlock_file(&locked);

    assert!(
        result.is_err(),
        "writing over a read-only file should have failed the update"
    );

    // The point of the test: every file is back at v1, including ones the extraction had
    // already overwritten with v2 before it hit the locked one.
    for name in ["a.txt", "b.txt", "y.txt", "z.txt"] {
        assert_eq!(
            std::fs::read(dir.join(name)).unwrap(),
            b"VERSION ONE",
            "{name} was left at the half-applied version"
        );
    }
    assert_eq!(
        std::fs::read(dir.join("sub/nested.txt")).unwrap(),
        b"VERSION ONE",
        "the nested file was not restored — rollback did not recurse"
    );
    assert!(
        !backup_of(&dir).exists(),
        "the .bak snapshot survived the rollback"
    );
}

/// A file that exists in the installed version and not in the new one — a user's config
/// left over from an older layout. The update itself has no reason to remove it, but
/// rollback WIPES the directory before restoring the snapshot, so this is the file that
/// disappears for good if the snapshot missed it.
///
/// The failure has to be a partway one (same read-only lever as above). Corrupting the
/// archive instead would prove nothing here: that fails before extraction writes a byte,
/// so the wipe-and-restore this test is about never runs, and the assertions pass on a
/// directory nobody touched. Checked by disabling the restore — this test stayed green
/// until it was rewritten to fail partway.
#[test]
fn rollback_restores_files_the_new_package_never_contained() {
    let base = scratch("orphan");
    let (v1, v2) = (base.join("v1"), base.join("v2"));
    for name in ["a.txt", "b.txt", "locked.txt", "z.txt"] {
        write(&v1.join(name), b"VERSION ONE");
        write(&v2.join(name), b"VERSION TWO");
    }
    // Only v1 has this one.
    write(&v1.join("legacy/config.ini"), b"USER SETTINGS");

    let (p1, p2) = (base.join("v1.bpkg"), base.join("v2.bpkg"));
    pack(&v1, &p1);
    pack(&v2, &p2);

    let dir = base.join("install");
    install(&p1, &dir);
    assert!(dir.join("legacy/config.ini").exists());

    let locked = dir.join("locked.txt");
    if !lock_file(&locked) {
        unlock_file(&locked);
        eprintln!(
            "SKIP rollback_restores_files_the_new_package_never_contained: this process can \
             write to a read-only file (running as root?), so the mid-extraction failure this \
             test needs cannot be provoked here"
        );
        return;
    }

    let result = apply_package_update(&p2, &dir, None, None);

    unlock_file(&locked);

    assert!(result.is_err(), "the update should have failed partway");
    assert_eq!(
        std::fs::read(dir.join("legacy/config.ini")).unwrap(),
        b"USER SETTINGS",
        "a file only the old version had was wiped and never restored"
    );
    assert_eq!(std::fs::read(dir.join("a.txt")).unwrap(), b"VERSION ONE");
}

/// A successful update must not leave the snapshot behind either — for BMM that is ~49 MB
/// of duplicate app sitting next to the install for good.
#[test]
fn a_successful_update_cleans_up_its_snapshot() {
    let base = scratch("cleanup");
    let (v1, v2) = (base.join("v1"), base.join("v2"));
    write(&v1.join("app.exe"), b"VERSION ONE");
    write(&v2.join("app.exe"), b"VERSION TWO");

    let (p1, p2) = (base.join("v1.bpkg"), base.join("v2.bpkg"));
    pack(&v1, &p1);
    pack(&v2, &p2);

    let dir = base.join("install");
    install(&p1, &dir);
    apply_package_update(&p2, &dir, None, None).unwrap();

    assert_eq!(std::fs::read(dir.join("app.exe")).unwrap(), b"VERSION TWO");
    assert!(
        !backup_of(&dir).exists(),
        "the .bak snapshot outlived a successful update"
    );
}

/// A signature has to cover the FRAMING, not only the framed bytes.
///
/// Format v1 signed the manifest and payload and left the 24-byte header out — and
/// verification reads `manifest_len` and `payload_len` OUT OF THAT HEADER to decide how much
/// to hash. The two numbers choosing the verified range were themselves unverified.
///
/// The obvious way to show that is to move the manifest/payload boundary while keeping the
/// sum — every byte v1 hashed stays identical. It is also a BAD test: a shifted boundary
/// breaks the manifest JSON, so the package fails to open and the assertion never runs. That
/// version passed against the old code too, which is the only thing a test doing nothing ever
/// tells you.
///
/// So this flips the two RESERVED header bytes instead. Nothing reads them, so the package
/// opens and parses exactly as before, every byte v1 hashed is untouched, and the only
/// difference is inside the header. Under v1 it verified. It must not now.
#[test]
fn editing_the_header_breaks_the_signature() {
    let base = scratch("hdrsig");
    let src = base.join("src");
    write(
        &src.join("app.exe"),
        b"VERSION ONE, with some bytes after it",
    );

    let pkg = base.join("v1.bpkg");
    pack(&src, &pkg);
    let publisher = bpkg_core::sign::generate();
    package::sign_package(&pkg, &publisher).unwrap();

    let vk = publisher.verifying_key();
    assert!(
        Package::open(&pkg).unwrap().verify_signature(&vk).unwrap(),
        "it must verify before being tampered with, or this test proves nothing",
    );

    // Bytes 10..12 are the reserved u16. Not read by anything, inside the header, and
    // therefore outside everything format v1 signed.
    let mut raw = std::fs::read(&pkg).unwrap();
    assert_eq!(
        &raw[10..12],
        &[0, 0],
        "reserved bytes are expected to be zero"
    );
    raw[10..12].copy_from_slice(&0xBEEFu16.to_le_bytes());
    let tampered = base.join("tampered.bpkg");
    std::fs::write(&tampered, &raw).unwrap();

    // It still opens — that is the point of picking a field nothing reads.
    let mut p = Package::open(&tampered).expect("a reserved-byte edit must not stop it opening");
    assert!(
        !p.verify_signature(&vk).unwrap_or(false),
        "a header edit must break the signature — v1 signed only the manifest and payload,          so this one did not",
    );
}
