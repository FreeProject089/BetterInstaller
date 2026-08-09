//! Perf / resource harness for the .bpkg lifecycle.
//!
//! Runs the REAL pack → sign → verify → install → delta → update+rollback pipeline over a
//! synthetic payload and records, for each stage: wall-time, incremental PEAK HEAP (via a
//! tracking global allocator), and the relevant byte size. It writes a machine-readable
//! JSON report under the target dir and prints a table, so it can run automatically in CI
//! and its numbers can be diffed release-to-release.
//!
//! It is also an integration TEST — it asserts the pipeline works and that compression +
//! delta actually shrink things — so a regression fails `cargo test`, not just a dashboard.
//!
//! Run:  cargo test -p bpkg-core --test perf -- --nocapture

use std::alloc::{GlobalAlloc, Layout, System};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

use bpkg_core::manifest::AppMeta;
use bpkg_core::package::{self, Package};
use bpkg_core::{delta, sign, update};

// ── Tracking allocator: current + peak live bytes, dependency-free. This file compiles to
// its own test binary, so the global allocator only instruments THIS harness. ──
struct Tracking;
static CUR: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static BASE: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Tracking {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = System.alloc(l);
        if !p.is_null() {
            let c = CUR.fetch_add(l.size(), Relaxed) + l.size();
            PEAK.fetch_max(c, Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l);
        CUR.fetch_sub(l.size(), Relaxed);
    }
}
#[global_allocator]
static ALLOC: Tracking = Tracking;

fn reset_peak() {
    let c = CUR.load(Relaxed);
    BASE.store(c, Relaxed);
    PEAK.store(c, Relaxed);
}
/// Incremental peak heap (KiB) since the last `reset_peak` — "how much this op allocated".
fn peak_kb() -> f64 {
    PEAK.load(Relaxed).saturating_sub(BASE.load(Relaxed)) as f64 / 1024.0
}

struct Row {
    op: &'static str,
    ms: f64,
    peak_kb: f64,
    bytes: u64,
}

fn dir_size(p: &Path) -> u64 {
    let mut n = 0;
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            let path = e.path();
            n += if path.is_dir() {
                dir_size(&path)
            } else {
                std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0)
            };
        }
    }
    n
}

// A deterministic pseudo-random fill (no `rand` dep) so payload sizes are reproducible but
// not trivially compressible — a realistic mix for the compressor.
fn pseudo_bytes(seed: u64, len: usize) -> Vec<u8> {
    let mut x = seed.wrapping_add(0x9E3779B97F4A7C15);
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

fn write_payload(root: &Path, tweak: bool) {
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    // A big-ish compressible text file (the "app resources"), plus a couple of binary-ish
    // files. `tweak` produces v2: same shape, a slice changed → a good delta candidate.
    let mut text = "BetterInstaller payload resource line.\n".repeat(40_000);
    if tweak {
        text.push_str("v2 appended tail — only a small part of the file changed.\n");
    }
    std::fs::write(root.join("resources.txt"), text).unwrap();
    // The binaries are IDENTICAL across v1/v2 — a realistic point release only tweaks a bit
    // of text — so the v1→v2 delta stays small (that's the whole point of shipping deltas).
    std::fs::write(bin.join("app.bin"), pseudo_bytes(1, 2 * 1024 * 1024)).unwrap();
    std::fs::write(bin.join("data.bin"), pseudo_bytes(42, 512 * 1024)).unwrap();
}

fn app_meta() -> AppMeta {
    AppMeta {
        id: "com.perf.test".into(),
        name: "Perf".into(),
        version: "1.0.0".into(),
        publisher: "BetterInstaller".into(),
        homepage: None,
        platforms: vec!["windows".into()],
    }
}

#[test]
fn perf_lifecycle() {
    let base = std::env::temp_dir().join(format!("bpkg-perf-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();

    let src_v1 = base.join("payload_v1");
    let src_v2 = base.join("payload_v2");
    write_payload(&src_v1, false);
    write_payload(&src_v2, true);
    let payload_bytes = dir_size(&src_v1);

    let mut rows: Vec<Row> = Vec::new();
    macro_rules! timed {
        ($name:expr, $bytes:expr, $body:block) => {{
            reset_peak();
            let t = Instant::now();
            let r = $body;
            rows.push(Row {
                op: $name,
                ms: t.elapsed().as_secs_f64() * 1000.0,
                peak_kb: peak_kb(),
                bytes: $bytes,
            });
            r
        }};
    }

    // ── pack ──
    let p1 = base.join("v1.bpkg");
    timed!("pack", payload_bytes, {
        package::create_from_dir(&src_v1, app_meta(), vec![], |_| None, &p1).unwrap();
    });
    let pkg_size = std::fs::metadata(&p1).unwrap().len();

    // ── sign / verify ──
    let sk = sign::generate();
    let vk = sk.verifying_key();
    timed!("sign", pkg_size, {
        package::sign_package(&p1, &sk).unwrap();
    });
    let verified = timed!("verify", pkg_size, {
        let mut pkg = Package::open(&p1).unwrap();
        pkg.verify_signature(&vk).unwrap()
    });
    assert!(verified, "signed package must verify with its key");

    // ── install (verify SHA + extract to disk) ──
    let install = base.join("install");
    timed!("install", pkg_size, {
        let mut pkg = Package::open(&p1).unwrap();
        pkg.install_with_progress(&install, None, |_, _, _| {})
            .unwrap();
    });
    assert_eq!(
        dir_size(&install),
        payload_bytes,
        "installed tree must match the source bytes"
    );

    // ── delta: build v2, diff v1→v2, apply the patch ──
    let p2 = base.join("v2.bpkg");
    package::create_from_dir(&src_v2, app_meta(), vec![], |_| None, &p2).unwrap();
    let v1_bytes = std::fs::read(&p1).unwrap();
    let v2_bytes = std::fs::read(&p2).unwrap();
    let patch = timed!("delta_make", v2_bytes.len() as u64, {
        delta::make_delta(&v1_bytes, &v2_bytes).unwrap()
    });
    let rebuilt = timed!("delta_apply", patch.len() as u64, {
        delta::apply_delta(&v1_bytes, &patch).unwrap()
    });
    assert_eq!(rebuilt, v2_bytes, "delta apply must reconstruct v2 exactly");

    // ── update over the install dir (with rollback safety net), signature-gated ──
    package::sign_package(&p2, &sk).unwrap();
    timed!("update", std::fs::metadata(&p2).unwrap().len(), {
        update::apply_package_update(&p2, &install, None, Some(&vk)).unwrap();
    });

    // ── report ──
    println!("\n  bpkg perf — payload {:.2} MiB → package {:.2} MiB ({:.1}% of payload), delta patch {:.2} KiB\n",
        payload_bytes as f64 / 1048576.0, pkg_size as f64 / 1048576.0,
        pkg_size as f64 / payload_bytes as f64 * 100.0, patch.len() as f64 / 1024.0);
    println!(
        "  {:<14} {:>10} {:>14} {:>14}",
        "stage", "time(ms)", "peak heap(KiB)", "bytes"
    );
    println!("  {}", "-".repeat(54));
    for r in &rows {
        println!(
            "  {:<14} {:>10.2} {:>14.1} {:>14}",
            r.op, r.ms, r.peak_kb, r.bytes
        );
    }

    // Machine-readable report under the crate's target dir (stable path cargo hands us).
    let mut json = String::from("{\n  \"payloadBytes\": ");
    json.push_str(&payload_bytes.to_string());
    json.push_str(",\n  \"packageBytes\": ");
    json.push_str(&pkg_size.to_string());
    json.push_str(",\n  \"deltaPatchBytes\": ");
    json.push_str(&patch.len().to_string());
    json.push_str(",\n  \"stages\": [\n");
    for (i, r) in rows.iter().enumerate() {
        json.push_str(&format!(
            "    {{ \"op\": \"{}\", \"ms\": {:.3}, \"peakHeapKiB\": {:.1}, \"bytes\": {} }}{}\n",
            r.op,
            r.ms,
            r.peak_kb,
            r.bytes,
            if i + 1 < rows.len() { "," } else { "" }
        ));
    }
    json.push_str("  ]\n}\n");
    let out = Path::new(env!("CARGO_TARGET_TMPDIR")).join("bpkg-perf.json");
    std::fs::write(&out, json).unwrap();
    println!("\n  report → {}\n", out.display());

    // Invariants: the compressor and the delta must actually pay off.
    assert!(
        pkg_size < payload_bytes,
        "package should be smaller than the raw payload"
    );
    assert!(
        (patch.len() as u64) < pkg_size / 2,
        "a small change should yield a small delta"
    );

    let _ = std::fs::remove_dir_all(&base);
}
