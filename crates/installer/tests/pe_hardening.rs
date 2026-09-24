//! Properties of the BUILT installer executable that no unit test can see.
//!
//! The DLL-search hardening (see `harden_dll_search` in main.rs) has a link-time half:
//! `/DEPENDENTLOADFLAG:0x800`, emitted by build.rs, makes the loader resolve the static
//! imports from System32 only. Nothing fails if that line stops reaching the linker — a
//! renamed env var, a moved `if` — so the flag is read back out of the PE here.

#![cfg(all(windows, target_env = "msvc"))]

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

/// `IMAGE_LOAD_CONFIG_DIRECTORY64.DependentLoadFlags` of a PE32+ image.
fn dependent_load_flags(b: &[u8]) -> Option<u16> {
    let pe = u32_at(b, 60) as usize;
    let sections = u16_at(b, pe + 6) as usize;
    let opt_size = u16_at(b, pe + 20) as usize;
    let opt = pe + 24;
    if u16_at(b, opt) != 0x20b {
        return None;
    }
    // Data directory 10: the load configuration table (an RVA).
    let rva = u32_at(b, opt + 112 + 10 * 8) as usize;
    if rva == 0 {
        return None;
    }
    let table = opt + opt_size;
    let file_off = (0..sections).find_map(|i| {
        let s = table + i * 40;
        let (vsize, va, raw) = (
            u32_at(b, s + 8) as usize,
            u32_at(b, s + 12) as usize,
            u32_at(b, s + 20) as usize,
        );
        (va..va + vsize.max(1))
            .contains(&rva)
            .then(|| raw + (rva - va))
    })?;
    // Size(4) TimeDateStamp(4) Major/Minor(2+2) GlobalFlagsClear/Set(4+4)
    // CriticalSectionDefaultTimeout(4) six u64 fields(48) ProcessHeapFlags(4) CSDVersion(2)
    // → DependentLoadFlags at 0x4E.
    Some(u16_at(b, file_off + 0x4E))
}

#[test]
fn static_imports_resolve_from_system32_only() {
    let exe = std::fs::read(env!("CARGO_BIN_EXE_betterinstaller")).unwrap();
    assert_eq!(
        dependent_load_flags(&exe),
        Some(0x800),
        "the installer resolves its DLLs from the folder it runs in (Downloads) — \
         /DEPENDENTLOADFLAG:0x800 did not reach the linker"
    );
}
