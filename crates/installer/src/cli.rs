//! The installer's command line: parsing, the unattended modes, exit codes and the consent
//! rules an unattended install must follow.
//!
//! Everything here is pure (no file system, no registry, no window) so it is unit-tested
//! whole; `unattended.rs` is what acts on it.
//!
//! The rules, in one place:
//!
//! - no `--mode` (and no `--update`) → the interactive installer, exactly as before;
//! - `--update` = `--mode update --passive`;
//! - any `--mode` runs unattended: a progress window (`--passive`, the default) or no window
//!   at all (`--quiet` / `--silent`), never a question;
//! - a FRESH install needs `--accept-terms` and `--accept-privacy` for the documents the
//!   package ships: acceptance is never assumed, and each acceptance is written as a consent
//!   receipt (document, SHA-256 of the exact text, language, time, source `cli`);
//! - an option that sends data is OFF unless it is named on the command line.

use std::path::PathBuf;

/// The process exit codes of an unattended run (and of argument errors). Stable: callers
/// such as BMM's updater branch on them. 10 is left out on purpose: `--check-update`
/// already answers 10 for "an update is available".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Exit {
    Ok = 0,
    /// Anything not covered below (network, prerequisite, handoff write...).
    Failed = 1,
    BadArgs = 2,
    /// update / repair / modify / uninstall found no installation of this app.
    NotInstalled = 3,
    /// The target needs administrator rights and they were refused, or `--quiet` forbids
    /// asking for them.
    Elevation = 4,
    /// The package's signature or a file hash did not verify. Never skippable.
    Verification = 5,
    /// Not enough free space, or the disk refused a write.
    Disk = 6,
    /// The app is still running and could not be closed.
    AppRunning = 7,
    /// The person closed the progress window before anything was written.
    Cancelled = 8,
    /// `--mode install` while the app is already installed.
    AlreadyInstalled = 9,
    /// A fresh install without `--accept-terms` / `--accept-privacy`.
    ConsentRequired = 11,
    /// The bundled version is older than the installed one.
    Downgrade = 12,
}

impl Exit {
    pub fn code(self) -> i32 {
        self as i32
    }

    /// Every code with its meaning, for `--help` and the docs.
    pub const TABLE: &'static [(Exit, &'static str)] = &[
        (
            Exit::Ok,
            "success (for --update: also \"already up to date\")",
        ),
        (
            Exit::Failed,
            "failed (network, prerequisite, handoff write, other)",
        ),
        (Exit::BadArgs, "bad arguments"),
        (Exit::NotInstalled, "no existing installation found"),
        (
            Exit::Elevation,
            "administrator rights needed and refused (or --quiet)",
        ),
        (
            Exit::Verification,
            "package signature or hash verification failed",
        ),
        (Exit::Disk, "not enough disk space / disk write error"),
        (
            Exit::AppRunning,
            "the app is running and could not be closed",
        ),
        (Exit::Cancelled, "cancelled"),
        (Exit::AlreadyInstalled, "already installed (--mode install)"),
        (
            Exit::ConsentRequired,
            "fresh install without --accept-terms / --accept-privacy",
        ),
        (
            Exit::Downgrade,
            "bundled version older than the installed one",
        ),
    ];
}

/// What an unattended run does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Install,
    Update,
    Repair,
    Modify,
    Uninstall,
}

impl Mode {
    fn parse(s: &str) -> Option<Mode> {
        Some(match s.to_ascii_lowercase().as_str() {
            "install" => Mode::Install,
            "update" => Mode::Update,
            "repair" => Mode::Repair,
            "modify" => Mode::Modify,
            "uninstall" => Mode::Uninstall,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::Install => "install",
            Mode::Update => "update",
            Mode::Repair => "repair",
            Mode::Modify => "modify",
            Mode::Uninstall => "uninstall",
        }
    }

    /// Works on an existing installation (and fails with `NotInstalled` without one).
    pub fn needs_existing(self) -> bool {
        !matches!(self, Mode::Install)
    }
}

/// How much is shown while working.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ui {
    /// A progress window, no question, closes at the end.
    Passive,
    /// No window at all.
    Quiet,
}

/// `--preset`: which components a fresh install (or a modify) takes when `--components` is
/// not given. The setup options always start from their declared defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    /// Required + default components (what the « Recommended » card installs).
    Recommended,
    /// Required components only.
    Minimal,
    /// Every component.
    Full,
}

impl Preset {
    fn parse(s: &str) -> Option<Preset> {
        Some(match s.to_ascii_lowercase().as_str() {
            "recommended" | "default" => Preset::Recommended,
            "minimal" => Preset::Minimal,
            "full" | "all" => Preset::Full,
            _ => return None,
        })
    }
}

/// `--shortcuts`: which entries to create. `none` clears all three.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Shortcuts {
    pub start_menu: bool,
    pub desktop: bool,
    pub autostart: bool,
}

impl Shortcuts {
    fn parse(s: &str) -> Result<Shortcuts, String> {
        let mut out = Shortcuts::default();
        for t in s.split(',').map(str::trim).filter(|t| !t.is_empty()) {
            match t.to_ascii_lowercase().as_str() {
                "none" => {}
                "desktop" => out.desktop = true,
                "startmenu" | "start-menu" | "start" => out.start_menu = true,
                "autostart" => out.autostart = true,
                other => {
                    return Err(format!(
                        "--shortcuts: unknown entry {other:?} (desktop, startmenu, autostart, none)"
                    ))
                }
            }
        }
        Ok(out)
    }
}

/// The parsed command line.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Cli {
    pub help: bool,
    pub version: bool,
    pub check_update: bool,
    pub mode: Option<Mode>,
    /// `--passive` / `--quiet` as given (`None`: the mode's default).
    pub ui: Option<Ui>,
    /// `--uninstall` with neither `--passive` nor `--quiet`: the interactive maintenance
    /// window (what the Apps & Features "Uninstall" button opens).
    pub gui_uninstall: bool,
    pub dir: Option<PathBuf>,
    pub components: Option<Vec<String>>,
    pub preset: Option<Preset>,
    pub shortcuts: Option<Shortcuts>,
    pub lang: Option<String>,
    pub launch: bool,
    pub log: Option<PathBuf>,
    pub no_restart: bool,
    pub accept_terms: bool,
    pub accept_privacy: bool,
    pub telemetry: Option<bool>,
    /// `--set <option>=<value>`, in order.
    pub sets: Vec<(String, String)>,
    pub wait_pid: Option<u32>,
    /// Seconds `--wait-pid` waits before closing the app itself (default 60).
    pub wait_timeout: Option<u64>,
    /// `--update` never asks the network: only the bundled package is considered.
    pub offline: bool,
    pub allow_downgrade: bool,
    /// Internal: this process is the elevated copy started by the UAC prompt.
    pub elevated: bool,
    /// Non-flag arguments: dev mode's `<installer.toml> [package.bpkg]`.
    pub positionals: Vec<String>,
}

/// Flags that take a value (`--flag value` or `--flag=value`).
const VALUE_FLAGS: &[&str] = &[
    "--mode",
    "--dir",
    "--components",
    "--preset",
    "--shortcuts",
    "--lang",
    "--log",
    "--telemetry",
    "--set",
    "--wait-pid",
    "--wait-timeout",
];

fn on_off(flag: &str, v: &str) -> Result<bool, String> {
    match v.to_ascii_lowercase().as_str() {
        "on" | "true" | "1" | "yes" => Ok(true),
        "off" | "false" | "0" | "no" => Ok(false),
        _ => Err(format!("{flag}: expected on or off, got {v:?}")),
    }
}

/// Parse the arguments after the program name. An unknown flag is an error: a typo in an
/// unattended command must not quietly become a different install.
pub fn parse(args: &[String]) -> Result<Cli, String> {
    let mut c = Cli::default();
    let mut update_shortcut = false;
    let mut uninstall_flag = false;
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        i += 1;
        if a == "-h" || a == "/?" {
            c.help = true;
            continue;
        }
        if !a.starts_with("--") {
            c.positionals.push(a.clone());
            continue;
        }
        let (flag, inline) = match a.split_once('=') {
            Some((f, v)) => (f, Some(v.to_string())),
            None => (a.as_str(), None),
        };
        let value = if VALUE_FLAGS.contains(&flag) {
            match inline {
                Some(v) => Some(v),
                None => {
                    let v = args
                        .get(i)
                        .filter(|v| !v.starts_with("--"))
                        .ok_or_else(|| format!("{flag} needs a value"))?;
                    i += 1;
                    Some(v.clone())
                }
            }
        } else {
            if inline.is_some() {
                return Err(format!("{flag} takes no value"));
            }
            None
        };
        let v = value.unwrap_or_default();
        match flag {
            "--help" | "-h" => c.help = true,
            "--version" => c.version = true,
            "--check-update" => c.check_update = true,
            "--update" => update_shortcut = true,
            "--uninstall" => uninstall_flag = true,
            "--passive" => set_ui(&mut c, Ui::Passive)?,
            "--quiet" | "--silent" => set_ui(&mut c, Ui::Quiet)?,
            "--mode" => {
                let m = Mode::parse(&v).ok_or_else(|| {
                    format!(
                        "--mode: unknown mode {v:?} (install, update, repair, modify, uninstall)"
                    )
                })?;
                set_mode(&mut c, m)?;
            }
            "--dir" => {
                if v.trim().is_empty() {
                    return Err("--dir needs a path".into());
                }
                c.dir = Some(PathBuf::from(v));
            }
            "--components" => {
                c.components = Some(
                    v.split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect(),
                )
            }
            "--preset" => {
                c.preset = Some(Preset::parse(&v).ok_or_else(|| {
                    format!("--preset: unknown preset {v:?} (recommended, minimal, full)")
                })?)
            }
            "--shortcuts" => c.shortcuts = Some(Shortcuts::parse(&v)?),
            "--lang" => c.lang = Some(v),
            "--log" => c.log = Some(PathBuf::from(v)),
            "--telemetry" => c.telemetry = Some(on_off("--telemetry", &v)?),
            "--set" => {
                let (id, val) = v
                    .split_once('=')
                    .ok_or_else(|| format!("--set: expected <option>=<value>, got {v:?}"))?;
                c.sets.push((id.trim().to_string(), val.trim().to_string()));
            }
            "--wait-pid" => {
                c.wait_pid = Some(
                    v.parse::<u32>()
                        .ok()
                        .filter(|p| *p != 0)
                        .ok_or_else(|| format!("--wait-pid: not a process id: {v:?}"))?,
                )
            }
            "--wait-timeout" => {
                c.wait_timeout = Some(
                    v.parse::<u64>()
                        .map_err(|_| format!("--wait-timeout: not a number of seconds: {v:?}"))?,
                )
            }
            "--launch" => c.launch = true,
            "--no-restart" => c.no_restart = true,
            "--accept-terms" => c.accept_terms = true,
            "--accept-privacy" => c.accept_privacy = true,
            "--offline" => c.offline = true,
            "--allow-downgrade" => c.allow_downgrade = true,
            "--elevated" => c.elevated = true,
            other => return Err(format!("unknown option {other}")),
        }
    }

    if update_shortcut {
        set_mode(&mut c, Mode::Update)?;
    }
    if uninstall_flag {
        if c.ui.is_none() && c.mode.is_none() {
            // Apps & Features: the interactive maintenance window, as always.
            c.gui_uninstall = true;
        } else {
            set_mode(&mut c, Mode::Uninstall)?;
        }
    }
    validate(&c)?;
    Ok(c)
}

fn set_mode(c: &mut Cli, m: Mode) -> Result<(), String> {
    match c.mode {
        Some(old) if old != m => Err(format!(
            "two modes given: {} and {} (pick one)",
            old.name(),
            m.name()
        )),
        _ => {
            c.mode = Some(m);
            Ok(())
        }
    }
}

fn set_ui(c: &mut Cli, ui: Ui) -> Result<(), String> {
    match c.ui {
        Some(old) if old != ui => Err("--passive and --quiet are exclusive".into()),
        _ => {
            c.ui = Some(ui);
            Ok(())
        }
    }
}

/// Combinations that cannot mean what the caller thinks.
fn validate(c: &Cli) -> Result<(), String> {
    if c.check_update && c.mode.is_some() {
        return Err("--check-update cannot be combined with a mode".into());
    }
    let Some(mode) = c.mode else {
        // The interactive installer: unattended-only flags would be silently ignored.
        if c.ui.is_some() {
            return Err(
                "--passive / --quiet need a mode (--mode <m>, --update or --uninstall)".into(),
            );
        }
        for (on, name) in [
            (c.components.is_some(), "--components"),
            (c.preset.is_some(), "--preset"),
            (c.shortcuts.is_some(), "--shortcuts"),
            (c.dir.is_some(), "--dir"),
            (c.accept_terms, "--accept-terms"),
            (c.accept_privacy, "--accept-privacy"),
            (c.telemetry.is_some(), "--telemetry"),
            (c.wait_pid.is_some(), "--wait-pid"),
            (c.launch, "--launch"),
            (c.log.is_some(), "--log"),
        ] {
            if on && !c.help && !c.version {
                return Err(format!(
                    "{name} needs a mode (--mode <m>, --update or --uninstall)"
                ));
            }
        }
        return Ok(());
    };
    let choices =
        c.components.is_some() || c.preset.is_some() || !c.sets.is_empty() || c.telemetry.is_some();
    match mode {
        Mode::Install | Mode::Modify => {}
        Mode::Update | Mode::Repair => {
            if choices || c.shortcuts.is_some() {
                return Err(format!(
                    "--mode {} keeps the installed components, shortcuts and answers; \
                     use --mode modify to change them",
                    mode.name()
                ));
            }
        }
        Mode::Uninstall => {
            if choices || c.shortcuts.is_some() || c.launch {
                return Err("--mode uninstall takes no install choices and no --launch".into());
            }
        }
    }
    if mode == Mode::Modify && (!c.sets.is_empty() || c.telemetry.is_some()) {
        return Err(
            "--mode modify changes components and shortcuts; the app's settings are changed \
             in the app"
                .into(),
        );
    }
    if mode != Mode::Install && (c.accept_terms || c.accept_privacy) {
        return Err(format!(
            "--accept-terms / --accept-privacy are for a fresh install; --mode {} keeps the \
             consent already given",
            mode.name()
        ));
    }
    if c.components.is_some() && c.preset.is_some() {
        return Err("--components and --preset are exclusive".into());
    }
    if c.wait_timeout.is_some() && c.wait_pid.is_none() {
        return Err("--wait-timeout needs --wait-pid".into());
    }
    if c.offline && mode != Mode::Update {
        return Err("--offline is for --mode update".into());
    }
    Ok(())
}

impl Cli {
    /// The window level of an unattended run: `--passive` unless `--quiet`.
    pub fn ui_level(&self) -> Ui {
        self.ui.unwrap_or(Ui::Passive)
    }
}

// ── Mode resolution ─────────────────────────────────────────────────────────

/// What an unattended run will actually do, decided from the mode, whether (and which
/// version) the app is installed and the version bundled in this setup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Fresh install (writes the handoff, needs consent).
    Fresh,
    /// Re-extract the bundled package over the installation (repair, modify, or an update
    /// whose bundled package is newer).
    Reinstall,
    /// The bundled package is not newer: ask the remote manifest (when configured).
    UpdateRemote,
    /// Nothing newer anywhere that may be asked: success, nothing written.
    UpToDate,
    Uninstall,
}

/// Decide the plan. `installed`: `Some(version)` when an installation was found (an empty
/// string when its version is unknown). `remote`: an `[update]` manifest is configured and
/// the run is not `--offline`.
pub fn resolve(
    mode: Mode,
    installed: Option<&str>,
    bundled: &str,
    remote: bool,
    allow_downgrade: bool,
) -> Result<Plan, (Exit, String)> {
    let Some(iv) = installed else {
        return match mode {
            Mode::Install => Ok(Plan::Fresh),
            m => Err((
                Exit::NotInstalled,
                format!(
                    "no existing installation found: --mode {} needs one. A first install is \
                     --mode install with --accept-terms and --accept-privacy (or run the setup \
                     without options).",
                    m.name()
                ),
            )),
        };
    };
    let older = !iv.is_empty() && bpkg_core::version::is_newer(iv, bundled);
    let newer = iv.is_empty() || bpkg_core::version::is_newer(bundled, iv);
    match mode {
        Mode::Install => Err((
            Exit::AlreadyInstalled,
            format!(
                "already installed (version {}); use --update, --mode repair or --mode modify",
                if iv.is_empty() { "unknown" } else { iv }
            ),
        )),
        Mode::Uninstall => Ok(Plan::Uninstall),
        Mode::Update => Ok(if newer {
            Plan::Reinstall
        } else if remote {
            Plan::UpdateRemote
        } else {
            Plan::UpToDate
        }),
        Mode::Repair | Mode::Modify => {
            if older && !allow_downgrade {
                Err((
                    Exit::Downgrade,
                    format!(
                        "installed version {iv} is newer than this setup ({bundled}); \
                         pass --allow-downgrade to go back"
                    ),
                ))
            } else {
                Ok(Plan::Reinstall)
            }
        }
    }
}

// ── Consent ─────────────────────────────────────────────────────────────────

/// Which flag accepts a legal document: `privacy` → `--accept-privacy`; terms, EULA and any
/// other licence text → `--accept-terms`.
pub fn accept_flag_for(doc_id: Option<&str>) -> &'static str {
    match doc_id {
        Some("privacy") => "--accept-privacy",
        _ => "--accept-terms",
    }
}

/// The flags a fresh install is missing, one per document not accepted (deduplicated, in
/// document order). `docs` is each document's id (`handoff::legal_doc_id`). `required`:
/// the project declares a required licence option even if no document text could be read
/// — then `--accept-terms` is still needed.
pub fn missing_consents(
    docs: &[Option<&str>],
    required: bool,
    accept_terms: bool,
    accept_privacy: bool,
) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    let mut need = |f: &'static str| {
        let ok = match f {
            "--accept-privacy" => accept_privacy,
            _ => accept_terms,
        };
        if !ok && !out.contains(&f) {
            out.push(f);
        }
    };
    for d in docs {
        need(accept_flag_for(*d));
    }
    if docs.is_empty() && required {
        need("--accept-terms");
    }
    out
}

// ── Help ────────────────────────────────────────────────────────────────────

/// `--help`. `product`: the app name and version when the config could be read, and the
/// lines listing its components and setup options.
pub fn help_text(exe: &str, product: Option<(&str, &str)>, extra: &str) -> String {
    let mut s = String::new();
    if let Some((name, ver)) = product {
        s.push_str(&format!(
            "{name} {ver} setup (BetterInstaller {})\n\n",
            bpkg_core::VERSION
        ));
    }
    s.push_str(&format!(
        "\
Usage:
  {exe}                         interactive installer
  {exe} --update [--wait-pid <pid>] [--launch]
                                update the existing installation, no questions
  {exe} --mode <mode> [options]  unattended run

Modes (--mode):
  install     fresh install (needs --accept-terms --accept-privacy)
  update      install the bundled version over the existing installation, keeping its
              folder, components, shortcuts and answers; when the bundled version is not
              newer, the [update] manifest is asked (unless --offline)
  repair      re-verify and re-extract the bundled version, same choices
  modify      change components (--components / --preset) and shortcuts (--shortcuts)
  uninstall   remove the app

Window:
  --passive             progress window, no question, closes at the end (default)
  --quiet, --silent     no window at all; fails instead of asking for administrator rights
  --update              same as --mode update --passive
  --uninstall           with --passive/--quiet: --mode uninstall; alone: the uninstall window

Install / modify choices:
  --dir <path>          install folder (default: the per-user folder)
  --components <a,b>    components to install (required ones are always added)
  --preset <p>          recommended | minimal | full (components when --components is absent)
  --shortcuts <list>    desktop,startmenu,autostart or none (default: as the installer offers)
  --set <option>=<v>    answer a setup option (bool: on/off, select: one of its choices)
  --telemetry on|off    shorthand for --set telemetry=on|off
  --accept-terms        accept the Terms of Service shipped in the package
  --accept-privacy      accept the Privacy Policy shipped in the package
  --lang <code>         installer language; also what a setup option left on \"auto\" becomes

Other:
  --wait-pid <pid>      wait for this process to exit first (e.g. the app asking for its
                        update), then close the app if it is still running
  --wait-timeout <s>    how long --wait-pid waits (default 60)
  --launch              start the app at the end
  --log <file>          append a log of the run to <file>
  --no-restart          never restart the computer (the installer never does; accepted for
                        compatibility with scripts)
  --offline             --update: do not ask the network
  --allow-downgrade     repair/modify: allow a setup older than what is installed
  --check-update        print a JSON update report and exit (0 up to date, 10 available, 2 error)
  --version             print the version
  --help                this text

Options sending data are OFF unless given. Consent is never assumed: a fresh unattended
install refuses to run without --accept-terms and --accept-privacy, and records each
acceptance (document, SHA-256 of the text, language, time, source \"cli\").
Package signature and hash verification is mandatory in every mode.

Exit codes:
"
    ));
    for (e, what) in Exit::TABLE {
        s.push_str(&format!("  {:>2}  {what}\n", e.code()));
    }
    if !extra.is_empty() {
        s.push('\n');
        s.push_str(extra);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Result<Cli, String> {
        parse(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn no_arguments_is_the_interactive_installer() {
        let c = p(&[]).unwrap();
        assert_eq!(c.mode, None);
        assert!(!c.gui_uninstall);
    }

    #[test]
    fn bmm_updater_spelling() {
        // What BMM's in-app updater runs. Keep this exact spelling working.
        let c = p(&["--update", "--wait-pid", "1234", "--launch"]).unwrap();
        assert_eq!(c.mode, Some(Mode::Update));
        assert_eq!(c.ui_level(), Ui::Passive);
        assert_eq!(c.wait_pid, Some(1234));
        assert!(c.launch);
        // `=` form too.
        let c = p(&["--update", "--wait-pid=1234", "--launch"]).unwrap();
        assert_eq!(c.wait_pid, Some(1234));
        // And the bare form older BMM builds use.
        assert_eq!(p(&["--update"]).unwrap().mode, Some(Mode::Update));
    }

    #[test]
    fn update_quiet_has_no_window() {
        let c = p(&["--update", "--quiet"]).unwrap();
        assert_eq!(c.ui_level(), Ui::Quiet);
        assert_eq!(p(&["--update", "--silent"]).unwrap().ui_level(), Ui::Quiet);
    }

    #[test]
    fn uninstall_alone_keeps_the_window_with_a_ui_level_runs_unattended() {
        let c = p(&["--uninstall"]).unwrap();
        assert!(c.gui_uninstall);
        assert_eq!(c.mode, None);
        let c = p(&["--uninstall", "--quiet"]).unwrap();
        assert!(!c.gui_uninstall);
        assert_eq!(c.mode, Some(Mode::Uninstall));
        assert_eq!(c.ui_level(), Ui::Quiet);
    }

    #[test]
    fn modes_and_values() {
        let c = p(&[
            "--mode",
            "install",
            "--dir",
            "D:\\Apps\\BMM",
            "--components=core,cli",
            "--shortcuts",
            "desktop,startmenu",
            "--lang",
            "fr",
            "--set",
            "theme=bmm-arctic",
            "--set=smart_io=off",
            "--telemetry",
            "off",
            "--accept-terms",
            "--accept-privacy",
            "--log",
            "C:\\t\\setup.log",
            "--no-restart",
            "--quiet",
        ])
        .unwrap();
        assert_eq!(c.mode, Some(Mode::Install));
        assert_eq!(c.dir, Some(PathBuf::from("D:\\Apps\\BMM")));
        assert_eq!(c.components, Some(vec!["core".into(), "cli".into()]));
        assert_eq!(
            c.shortcuts,
            Some(Shortcuts {
                start_menu: true,
                desktop: true,
                autostart: false
            })
        );
        assert_eq!(c.lang.as_deref(), Some("fr"));
        assert_eq!(
            c.sets,
            vec![
                ("theme".into(), "bmm-arctic".into()),
                ("smart_io".into(), "off".into())
            ]
        );
        assert_eq!(c.telemetry, Some(false));
        assert!(c.accept_terms && c.accept_privacy && c.no_restart);
        assert_eq!(c.ui_level(), Ui::Quiet);
    }

    #[test]
    fn positionals_are_kept_apart_from_flag_values() {
        // Dev mode: `<installer.toml> <package>`; a `--dir` value must not be read as one.
        let c = p(&["cfg.toml", "--mode", "repair", "pkg.bpkg", "--dir", "X:\\d"]).unwrap();
        assert_eq!(c.positionals, vec!["cfg.toml", "pkg.bpkg"]);
        assert_eq!(c.dir, Some(PathBuf::from("X:\\d")));
    }

    #[test]
    fn bad_arguments_are_refused() {
        for bad in [
            &["--mode"][..],
            &["--mode", "explode"],
            &["--frobnicate"],
            &["--update", "--mode", "install"],
            &["--passive", "--quiet", "--update"],
            &["--passive"],
            &["--update", "--components", "core"],
            &["--update", "--set", "a=b"],
            &["--mode", "repair", "--preset", "full"],
            &["--mode", "uninstall", "--launch", "--quiet"],
            &["--mode", "update", "--accept-terms"],
            &["--mode", "install", "--components", "a", "--preset", "full"],
            &["--mode", "install", "--wait-timeout", "5"],
            &["--mode", "install", "--wait-pid", "abc"],
            &["--mode", "install", "--wait-pid", "0"],
            &["--mode", "install", "--telemetry", "maybe"],
            &["--mode", "install", "--set", "novalue"],
            &["--mode", "install", "--shortcuts", "taskbar"],
            &["--mode", "modify", "--set", "theme=x"],
            &["--mode", "repair", "--offline"],
            &["--launch"],
            &["--dir", "x"],
            &["--check-update", "--update"],
            &["--launch=yes", "--update"],
        ] {
            assert!(p(bad).is_err(), "accepted: {bad:?}");
        }
    }

    #[test]
    fn help_and_version_parse_alongside_anything() {
        assert!(p(&["--help"]).unwrap().help);
        assert!(p(&["-h"]).unwrap().help);
        assert!(p(&["--version"]).unwrap().version);
        let h = help_text("BMM-Setup.exe", Some(("BMM", "1.0.2")), "");
        for flag in [
            "--update",
            "--wait-pid",
            "--launch",
            "--passive",
            "--quiet",
            "--accept-terms",
            "--accept-privacy",
            "--components",
            "--preset",
            "--shortcuts",
            "--log",
            "--no-restart",
        ] {
            assert!(h.contains(flag), "help misses {flag}");
        }
        for (e, _) in Exit::TABLE {
            assert!(h.contains(&format!("{:>2}  ", e.code())));
        }
    }

    #[test]
    fn exit_codes_are_stable_and_distinct() {
        let codes: Vec<i32> = Exit::TABLE.iter().map(|(e, _)| e.code()).collect();
        assert_eq!(codes, vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 11, 12]);
        // 10 belongs to --check-update ("update available").
        assert!(!codes.contains(&10));
    }

    #[test]
    fn plan_without_an_installation() {
        assert_eq!(
            resolve(Mode::Install, None, "1.0.2", true, false),
            Ok(Plan::Fresh)
        );
        for m in [Mode::Update, Mode::Repair, Mode::Modify, Mode::Uninstall] {
            let e = resolve(m, None, "1.0.2", true, false).unwrap_err();
            assert_eq!(
                e.0,
                Exit::NotInstalled,
                "{m:?} must never become a fresh install"
            );
        }
    }

    #[test]
    fn plan_with_an_installation() {
        let r = |m, iv, remote| resolve(m, Some(iv), "1.0.2", remote, false);
        assert_eq!(
            r(Mode::Install, "1.0.1", true).unwrap_err().0,
            Exit::AlreadyInstalled
        );
        assert_eq!(r(Mode::Update, "1.0.1", true), Ok(Plan::Reinstall));
        assert_eq!(r(Mode::Update, "", false), Ok(Plan::Reinstall));
        // Same version: the uninstaller's own copy run with --update asks the manifest.
        assert_eq!(r(Mode::Update, "1.0.2", true), Ok(Plan::UpdateRemote));
        assert_eq!(r(Mode::Update, "1.0.2", false), Ok(Plan::UpToDate));
        // Never a downgrade through --update.
        assert_eq!(r(Mode::Update, "2.0.0", false), Ok(Plan::UpToDate));
        assert_eq!(r(Mode::Repair, "1.0.2", true), Ok(Plan::Reinstall));
        assert_eq!(
            r(Mode::Repair, "2.0.0", true).unwrap_err().0,
            Exit::Downgrade
        );
        assert_eq!(
            resolve(Mode::Modify, Some("2.0.0"), "1.0.2", false, true),
            Ok(Plan::Reinstall)
        );
        assert_eq!(r(Mode::Uninstall, "9.9.9", false), Ok(Plan::Uninstall));
    }

    #[test]
    fn consent_is_never_assumed() {
        let docs = [Some("tos"), Some("privacy")];
        assert_eq!(
            missing_consents(&docs, true, false, false),
            vec!["--accept-terms", "--accept-privacy"]
        );
        assert_eq!(
            missing_consents(&docs, true, true, false),
            vec!["--accept-privacy"]
        );
        assert_eq!(
            missing_consents(&docs, true, false, true),
            vec!["--accept-terms"]
        );
        assert!(missing_consents(&docs, true, true, true).is_empty());
        // A project licence that is neither terms nor privacy: --accept-terms.
        assert_eq!(
            missing_consents(&[None], true, false, true),
            vec!["--accept-terms"]
        );
        // A required licence option whose text could not be read still needs acceptance.
        assert_eq!(
            missing_consents(&[], true, false, false),
            vec!["--accept-terms"]
        );
        // No legal document at all: nothing to accept.
        assert!(missing_consents(&[], false, false, false).is_empty());
    }
}
