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
    }
}
