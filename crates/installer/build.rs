fn main() {
    slint_build::compile("ui/main.slint").expect("compiling ui/main.slint");

    // Embed a Windows application manifest with requestedExecutionLevel=asInvoker.
    // Without it, Windows' "installer detection" heuristic auto-elevates any exe
    // whose name contains "install"/"setup"/"update" and forces a UAC prompt —
    // wrong for a per-user install. (CARGO_CFG_WINDOWS is set only for Windows targets.)
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        use embed_manifest::manifest::ExecutionLevel;
        use embed_manifest::{embed_manifest, new_manifest};
        embed_manifest(
            new_manifest("BetterCommunity.BetterInstaller")
                .requested_execution_level(ExecutionLevel::AsInvoker),
        )
        .expect("unable to embed Windows manifest");

        // Static imports resolve from System32 only, not from the folder the setup runs in
        // (Downloads): see harden_dll_search in main.rs. MSVC linker only — the GNU
        // toolchain has no equivalent switch.
        if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
            println!("cargo:rustc-link-arg-bins=/DEPENDENTLOADFLAG:0x800");
        }

        // The Windows EXE icon, taken from BI_ICON when the packaging script sets it.
        //
        // It belongs here rather than in the stamping step because a self-extracting
        // installer is the base exe with the config and package APPENDED — the PE itself
        // is never rewritten, and editing RT_ICON in place afterwards would mean either
        // fitting inside whatever placeholder was compiled in, or rebuilding the resource
        // section and every offset after it. Compiling the icon in is exact and cheap, and
        // it is per-product in practice because build-installer.ps1 rebuilds the engine on
        // every run (a presence check there used to accept a stale binary).
        //
        // Absent or unreadable BI_ICON leaves the default icon rather than failing the
        // build: an engine compiled for a project that ships no icon is still a valid
        // engine. The packaging script is where a MISSING icon is worth shouting about,
        // and it does.
        if let Some(ico) = std::env::var_os("BI_ICON") {
            let path = std::path::PathBuf::from(&ico);
            println!("cargo:rerun-if-env-changed=BI_ICON");
            println!("cargo:rerun-if-changed={}", path.display());
            if path.is_file() {
                let mut res = winresource::WindowsResource::new();
                res.set_icon(&path.to_string_lossy());
                if let Err(e) = res.compile() {
                    println!("cargo:warning=BI_ICON set but the icon could not be embedded: {e}");
                }
            } else {
                println!(
                    "cargo:warning=BI_ICON points at no file: {}",
                    path.display()
                );
            }
        }
    }
}
