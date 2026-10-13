//! Unattended runs: `--update`, `--mode <m>`, `--uninstall --quiet` (cli.rs has the rules).
//!
//! Same engine as the window: the package is verified by `run_real_install` exactly as for
//! a click on Install (signature against the pinned key, SHA-256 of every file), the
//! handoff is built by `handoff::build`, consent receipts by `legal_receipts`. What changes
//! is only who answers: the command line, and never by assumption.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bpkg_core::config::{InstallerConfig, SetupOption, SetupOptionKind};
use bpkg_core::handoff;
use bpkg_core::i18n::{Direction, Translator};
use bpkg_core::manifest::{AppMeta, FileEntry};
use bpkg_core::package::Package;
use bpkg_core::platform;
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::cli::{self, Cli, Exit, Mode, Plan, Preset, Ui};
use crate::{Hooks, I18n, MainWindow, StepRow, SystemIntegration};

/// An unattended run that did not succeed: its exit code and what to tell the person.
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    pub code: Exit,
    pub message: String,
}

impl Failure {
    pub fn new(code: Exit, message: impl Into<String>) -> Self {
        Failure {
            code,
            message: message.into(),
        }
    }

    fn bad_args(message: impl Into<String>) -> Self {
        Failure::new(Exit::BadArgs, message)
    }

    /// A package / engine error, classified: a package that does not verify is
    /// `Verification` whatever the step that found it.
    pub fn from_pkg(e: &bpkg_core::error::Error) -> Self {
        use bpkg_core::error::Error as E;
        let code = match e {
            E::BadMagic | E::UnsupportedVersion(..) | E::Corrupt(_) | E::HashMismatch { .. } => {
                Exit::Verification
            }
            E::Io { source, .. } | E::IoBare(source) => io_exit(source),
            E::Other(m) if m.contains("signature") || m.starts_with("update rejected") => {
                Exit::Verification
            }
            _ => Exit::Failed,
        };
        Failure::new(code, e.to_string())
    }
}

/// The exit code an I/O error stands for.
pub fn io_exit(e: &std::io::Error) -> Exit {
    #[cfg(windows)]
    match e.raw_os_error() {
        // ERROR_HANDLE_DISK_FULL, ERROR_DISK_FULL
        Some(39) | Some(112) => return Exit::Disk,
        // ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION: a file the app holds open.
        Some(32) | Some(33) => return Exit::AppRunning,
        _ => {}
    }
    #[cfg(unix)]
    if e.raw_os_error() == Some(28) {
        // ENOSPC
        return Exit::Disk;
    }
    match e.kind() {
        std::io::ErrorKind::PermissionDenied => Exit::Elevation,
        _ => Exit::Failed,
    }
}

// ── Log ─────────────────────────────────────────────────────────────────────

/// `--log <file>` (appended) and stderr: every line of an unattended run, timestamped.
pub struct Log {
    file: Mutex<Option<std::fs::File>>,
}

impl Log {
    fn open(path: Option<&Path>) -> Result<Log, Failure> {
        let file = match path {
            Some(p) => Some(
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(p)
                    .map_err(|e| Failure::bad_args(format!("--log {}: {e}", p.display())))?,
            ),
            None => None,
        };
        Ok(Log {
            file: Mutex::new(file),
        })
    }

    pub fn line(&self, msg: &str) {
        let line = format!("{} {msg}", chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ"));
        eprintln!("{line}");
        if let Ok(mut f) = self.file.lock() {
            if let Some(f) = f.as_mut() {
                let _ = writeln!(f, "{line}");
            }
        }
    }
}

// ── Entry point ─────────────────────────────────────────────────────────────

/// Run `mode` without a question. Returns the process exit code.
pub fn run(
    args: &Cli,
    mode: Mode,
    cfg: InstallerConfig,
    package_path: Option<PathBuf>,
    config_dir: Option<PathBuf>,
) -> Exit {
    #[cfg(windows)]
    crate::attach_parent_console();
    let log = match Log::open(args.log.as_deref()) {
        Ok(l) => Arc::new(l),
        Err(f) => {
            eprintln!("{}", f.message);
            return f.code;
        }
    };
    log.line(&format!(
        "{} {} setup (BetterInstaller {}): --mode {}{}",
        cfg.app.name,
        cfg.app.version,
        bpkg_core::VERSION,
        mode.name(),
        if args.ui_level() == Ui::Quiet {
            " --quiet"
        } else {
            " --passive"
        }
    ));
    match run_inner(args, mode, cfg, package_path, config_dir, &log) {
        Ok(msg) => {
            log.line(&format!("done: {msg}"));
            Exit::Ok
        }
        Err(f) => {
            log.line(&format!("failed (exit {}): {}", f.code.code(), f.message));
            f.code
        }
    }
}

/// The installation this run works on: where, which version (empty when unknown), what
/// it recorded.
struct Existing {
    dir: PathBuf,
    version: String,
    info: serde_json::Value,
}

fn record_is_for(dir: &Path, app_id: &str) -> bool {
    crate::uninstall::read_info(dir)["app_id"].as_str() == Some(app_id)
}

/// The registry entry (Apps & Features), else an `uninstall-info.json` for this app in
/// `--dir`, else beside this executable (the installed `uninstall.exe` run by hand).
fn find_existing(cfg: &InstallerConfig, dir_arg: Option<&Path>) -> Option<Existing> {
    let plat = platform::current();
    let app_id = cfg.app.id.as_str();
    let (dir, version) = match plat.installed_dir(app_id) {
        Some(d) => (d, plat.installed_version(app_id).unwrap_or_default()),
        None => {
            let here = std::env::current_exe()
                .ok()
                .and_then(|e| e.parent().map(Path::to_path_buf));
            let d = dir_arg
                .map(Path::to_path_buf)
                .into_iter()
                .chain(here)
                .find(|d| record_is_for(d, app_id))?;
            (d, String::new())
        }
    };
    let info = crate::uninstall::read_info(&dir);
    Some(Existing { dir, version, info })
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let c = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    c(a).to_string_lossy()
        .eq_ignore_ascii_case(&c(b).to_string_lossy())
}

fn absolute(p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(p)
    }
}

fn run_inner(
    args: &Cli,
    mode: Mode,
    cfg: InstallerConfig,
    package_path: Option<PathBuf>,
    config_dir: Option<PathBuf>,
    log: &Arc<Log>,
) -> Result<String, Failure> {
    let plat = platform::current();
    let app_meta = AppMeta {
        id: cfg.app.id.clone(),
        name: cfg.app.name.clone(),
        version: cfg.app.version.clone(),
        publisher: cfg.app.publisher.clone(),
        homepage: cfg.app.homepage.clone(),
        platforms: cfg.app.platforms.clone(),
    };

    // ── The package: verified before anything else, in every mode but uninstall (which
    //    only reads its file list, and works without one).
    let trust = crate::detect_signature(&cfg, package_path.as_deref());
    if mode != Mode::Uninstall {
        let require = cfg.security.as_ref().is_some_and(|s| s.require_signature);
        match trust {
            crate::Trust::Preview => {
                return Err(Failure::new(
                    Exit::Failed,
                    "no package: this executable carries no payload (dev mode needs \
                     <installer.toml> <package.bpkg>)",
                ))
            }
            crate::Trust::Unreadable | crate::Trust::Invalid => {
                return Err(Failure::new(
                    Exit::Verification,
                    "the package does not verify (unreadable, or its signature does not match \
                     the publisher key); nothing was changed",
                ))
            }
            crate::Trust::Unsigned if require => {
                return Err(Failure::new(
                    Exit::Verification,
                    "the package is not signed, but a signature is required; nothing was changed",
                ))
            }
            _ => {}
        }
    }

    // ── Language: --lang, else as the window would pick it.
    let mut tr = Translator::builtin();
    tr.set_ctx("app", cfg.app.name.clone());
    tr.set_ctx("publisher", cfg.app.publisher.clone());
    tr.set_ctx("version", cfg.app.version.clone());
    tr.set_ctx("new_version", cfg.app.version.clone());
    if trust.may_read() {
        for c in crate::load_product_catalogs(&cfg, package_path.as_deref(), config_dir.as_deref())
        {
            tr.add_catalog(c);
        }
    }
    let available: Vec<String> = tr.available().into_iter().map(|(c, _)| c).collect();
    tr.set_language(&crate::initial_language(
        &cfg,
        args.lang.as_deref(),
        &available,
    ));

    // ── Where is it installed?
    let existing = find_existing(&cfg, args.dir.as_deref());
    if let (Some(e), Some(d), true) = (&existing, &args.dir, mode.needs_existing()) {
        if !same_dir(&e.dir, d) {
            return Err(Failure::bad_args(format!(
                "{} is installed in {}; --dir {} cannot move it (uninstall, then install there)",
                cfg.app.name,
                e.dir.display(),
                d.display()
            )));
        }
    }
    if let Some(e) = &existing {
        tr.set_ctx("installed_version", e.version.clone());
        log.line(&format!(
            "installed: {} (version {})",
            e.dir.display(),
            if e.version.is_empty() {
                "unknown"
            } else {
                &e.version
            }
        ));
    } else {
        log.line("installed: no");
    }
    let remote = cfg.update.is_some() && !args.offline;
    let plan = cli::resolve(
        mode,
        existing.as_ref().map(|e| e.version.as_str()),
        &cfg.app.version,
        remote,
        args.allow_downgrade,
    )
    .map_err(|(c, m)| Failure::new(c, m))?;
    log.line(&format!("plan: {plan:?}"));

    let pkg_files: Vec<FileEntry> = package_path
        .as_deref()
        .and_then(|p| Package::open(p).ok())
        .map(|p| p.manifest.files)
        .unwrap_or_default();

    if plan == Plan::UpToDate {
        // Nothing to replace: a --wait-pid caller is not waited for, nor closed.
        if args.launch {
            launch_app(
                &cfg,
                &existing.as_ref().map(|e| e.dir.clone()).unwrap_or_default(),
            );
        }
        return Ok(format!(
            "{} is up to date (installed {}, this setup {})",
            cfg.app.name,
            existing
                .as_ref()
                .map(|e| e.version.as_str())
                .unwrap_or_default(),
            cfg.app.version
        ));
    }

    // ── The target folder and what goes in it.
    let dest = match (&plan, &existing) {
        (Plan::Fresh, _) => absolute(
            &args
                .dir
                .clone()
                .unwrap_or_else(|| plat.default_install_dir(&app_meta)),
        ),
        (_, Some(e)) => e.dir.clone(),
        (_, None) => unreachable!("resolve() refuses every other plan without an installation"),
    };
    let mut integ = SystemIntegration::from_config(&cfg, app_meta.clone());
    if let Some(e) = &existing {
        integ.apply_record(&e.info);
    } else {
        // A fresh install's defaults, as the Location page shows them.
        integ.start_menu = cfg.install.create_shortcuts;
        integ.desktop = cfg.install.create_shortcuts && cfg.install.desktop_shortcut;
        integ.autostart = cfg.install.main_exe.is_some() && cfg.install.autostart.unwrap_or(false);
    }
    if let Some(s) = args.shortcuts {
        check_shortcuts(&cfg, s).map_err(Failure::bad_args)?;
        integ.start_menu = s.start_menu;
        integ.desktop = s.desktop;
        integ.autostart = s.autostart;
    }

    let (comps, drop) = match (&plan, mode) {
        (Plan::Fresh, _) | (_, Mode::Modify) => {
            let wanted = chosen_components(&cfg, args.components.as_deref(), args.preset)
                .map_err(Failure::bad_args)?;
            let wanted = match (
                &existing,
                args.components.is_some() || args.preset.is_some(),
            ) {
                // Modify with no component flag keeps what is installed.
                (Some(e), false) => installed_components(&cfg, &e.info, &pkg_files, &e.dir),
                _ => wanted,
            };
            let drop = existing
                .as_ref()
                .map(|e| {
                    installed_components(&cfg, &e.info, &pkg_files, &e.dir)
                        .into_iter()
                        .filter(|c| !wanted.contains(c))
                        .collect()
                })
                .unwrap_or_default();
            (wanted, drop)
        }
        (_, _) => (
            existing
                .as_ref()
                .map(|e| installed_components(&cfg, &e.info, &pkg_files, &e.dir))
                .unwrap_or_default(),
            Vec::new(),
        ),
    };

    // ── A fresh install: the answers, and the consent that is never assumed.
    let mut fresh: Option<(BTreeMap<String, serde_json::Value>, Vec<crate::LegalDoc>)> = None;
    let mut comps = comps;
    if plan == Plan::Fresh {
        let mut chosen = unattended_choices(&cfg.setup_options, &args.sets, args.telemetry)
            .map_err(Failure::bad_args)?;
        let docs = crate::load_legal_docs(&cfg, package_path.as_deref(), trust.may_read(), &tr);
        let legal_opt = cfg
            .setup_options
            .iter()
            .find(|o| matches!(o.kind, SetupOptionKind::License) && o.required);
        let ids: Vec<Option<&str>> = docs
            .iter()
            .map(|d| handoff::legal_doc_id(&d.file))
            .collect();
        // A required licence checkbox with no document text (shown on the setup page).
        let plain_checkbox = legal_opt.is_some_and(|o| o.documents.is_empty());
        let missing =
            cli::missing_consents(&ids, plain_checkbox, args.accept_terms, args.accept_privacy);
        if !missing.is_empty() {
            let mut msg = format!(
                "a fresh install needs your consent, given explicitly: add {}.\n",
                missing.join(" ")
            );
            for d in &docs {
                msg.push_str(&format!(
                    "  {} ({}, {}): SHA-256 {} -> {}\n",
                    d.title,
                    d.file,
                    d.lang,
                    d.sha256,
                    cli::accept_flag_for(handoff::legal_doc_id(&d.file))
                ));
            }
            msg.push_str(
                "Read them first: they are in the package, and the installer window shows them.",
            );
            return Err(Failure::new(Exit::ConsentRequired, msg));
        }
        if let Some(o) = legal_opt {
            chosen.insert(o.id.clone(), serde_json::json!(true));
        }
        for id in cfg.prereqs_chosen_by_options(&chosen) {
            let id = format!("prereq:{id}");
            if !comps.contains(&id) {
                comps.push(id);
            }
        }
        for d in &docs {
            log.line(&format!(
                "accepted {} ({}, {}) SHA-256 {}",
                d.file,
                handoff::legal_doc_id(&d.file).unwrap_or("licence"),
                d.lang,
                d.sha256
            ));
        }
        fresh = Some((chosen, docs));
    }
    log.line(&format!(
        "folder: {}; components: {}{}; shortcuts: start menu {}, desktop {}, autostart {}",
        dest.display(),
        if comps.is_empty() {
            "all".to_string()
        } else {
            comps.join(",")
        },
        if drop.is_empty() {
            String::new()
        } else {
            format!(" (removing {})", drop.join(","))
        },
        integ.start_menu,
        integ.desktop,
        integ.autostart
    ));

    // ── Administrator rights, asked once (never in --quiet).
    if !writable(&dest) {
        if args.elevated || args.ui_level() == Ui::Quiet {
            return Err(Failure::new(
                Exit::Elevation,
                format!(
                    "{} needs administrator rights{}",
                    dest.display(),
                    if args.elevated {
                        ", and the elevated copy still cannot write it"
                    } else {
                        "; --quiet never asks for them (run from an elevated prompt)"
                    }
                ),
            ));
        }
        log.line("administrator rights needed: asking once (UAC)");
        let code = elevate()?;
        return match code {
            Exit::Ok => Ok("finished by the elevated copy".into()),
            c => Err(Failure::new(c, "the elevated copy failed (see its log)")),
        };
    }

    // ── Room on the disk (prerequisite downloads are not counted).
    if matches!(plan, Plan::Fresh | Plan::Reinstall) {
        let pkg_comps: Vec<String> = comps
            .iter()
            .filter(|c| !c.starts_with("prereq:"))
            .cloned()
            .collect();
        let need = bytes_needed(&pkg_files, &pkg_comps, &dest);
        if let Some(free) = free_space(&dest) {
            if free < need {
                return Err(Failure::new(
                    Exit::Disk,
                    format!(
                        "not enough free space on the drive of {}: {} MB needed, {} MB free",
                        dest.display(),
                        need.div_ceil(1_048_576),
                        free / 1_048_576
                    ),
                ));
            }
        }
    }

    // ── The work, with or without a window.
    let verb = tr.t(match mode {
        Mode::Install | Mode::Modify => "steps.install",
        Mode::Update => "steps.update",
        Mode::Repair => "steps.repair",
        Mode::Uninstall => "steps.uninstall",
    });
    let job = Job {
        plan: plan.clone(),
        pkg: package_path.clone(),
        pkg_files: pkg_files.iter().map(|f| f.path.clone()).collect(),
        dest: dest.clone(),
        comps,
        drop,
        integ,
        tr: tr.clone(),
        wait_pid: args.wait_pid,
        wait_timeout: Duration::from_secs(args.wait_timeout.unwrap_or(60)),
        installed_version: existing
            .as_ref()
            .map(|e| e.version.clone())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| cfg.app.version.clone()),
        update: cfg.update.clone(),
        pinned: crate::pinned_key(&cfg),
        app_id: cfg.app.id.clone(),
        log: log.clone(),
    };
    let msg = match args.ui_level() {
        Ui::Quiet => {
            let r = Reporter {
                log: log.clone(),
                weak: None,
                cancel: Arc::new(AtomicBool::new(false)),
            };
            job.run(&r)?
        }
        Ui::Passive => with_window(&cfg, &tr, &verb, job)?,
    };

    // ── A fresh install hands its answers and receipts to the app.
    if let Some((mut chosen, docs)) = fresh {
        write_handoff(
            &cfg,
            &tr,
            &mut chosen,
            &docs,
            &dest,
            plat.as_ref(),
            &app_meta,
        )?;
    }

    if args.launch && mode != Mode::Uninstall {
        launch_app(&cfg, &dest);
    }
    Ok(msg)
}

/// The handoff of a fresh unattended install: the same document the window writes, with
/// the consent receipts marked `cli`.
fn write_handoff(
    cfg: &InstallerConfig,
    tr: &Translator,
    chosen: &mut BTreeMap<String, serde_json::Value>,
    docs: &[crate::LegalDoc],
    dest: &Path,
    plat: &dyn platform::PlatformOps,
    app: &AppMeta,
) -> Result<(), Failure> {
    let Some(h) = cfg.handoff.as_ref().filter(|h| h.enabled) else {
        return Ok(());
    };
    resolve_auto_selects(&cfg.setup_options, chosen, tr);
    let comps: Vec<String> = Vec::new();
    let mut doc = handoff::build(
        &cfg.setup_options,
        chosen,
        comps,
        &cfg.app.version,
        bpkg_core::VERSION,
    );
    doc.components = crate::uninstall::read_info(dest)["components"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    doc.install_dir = dest.to_string_lossy().to_string();
    let accepted = cfg
        .setup_options
        .iter()
        .filter(|o| matches!(o.kind, SetupOptionKind::License))
        .any(|o| chosen.get(&o.id).and_then(|v| v.as_bool()) == Some(true));
    if accepted {
        doc.consents = crate::legal_receipts(docs, &doc.installed_at.clone(), "cli");
    }
    let dir = match h.location {
        bpkg_core::config::HandoffLocation::AppData => plat.app_data_dir(app),
        bpkg_core::config::HandoffLocation::InstallDir => dest.to_path_buf(),
    };
    let path = dir.join(&h.file);
    doc.write_atomic(&path).map_err(|e| {
        Failure::new(
            Exit::Failed,
            format!(
                "installed, but the first-run settings could not be written to {}: {e}",
                path.display()
            ),
        )
    })
}

/// A select left on "auto" becomes the installer's language among its choices (as the
/// window does): `fr-CH` → `fr`, an unsupported language → `en`.
fn resolve_auto_selects(
    opts: &[SetupOption],
    chosen: &mut BTreeMap<String, serde_json::Value>,
    tr: &Translator,
) {
    for opt in opts {
        if matches!(opt.kind, SetupOptionKind::Select) {
            let eff = chosen
                .get(&opt.id)
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| opt.default.as_str().unwrap_or_default().to_string());
            if eff == "auto" {
                if let Some(v) = tr.resolve_among(&opt.choices) {
                    chosen.insert(opt.id.clone(), serde_json::json!(v));
                }
            }
        }
    }
}

/// Start the app after the run: `[install] main_exe`, else the first default `[[launch]]`.
fn launch_app(cfg: &InstallerConfig, dir: &Path) {
    let exe = cfg
        .install
        .main_exe
        .clone()
        .or_else(|| cfg.launch.iter().find(|l| l.default).map(|l| l.exe.clone()));
    if let Some(exe) = exe {
        crate::launch_detached(&dir.join(exe));
    }
}

// ── Choices ─────────────────────────────────────────────────────────────────

/// `--shortcuts` must ask only for what this installer offers.
fn check_shortcuts(cfg: &InstallerConfig, s: cli::Shortcuts) -> Result<(), String> {
    let offers = cfg.install.create_shortcuts && cfg.install.main_exe.is_some();
    if (s.start_menu || s.desktop) && !offers {
        return Err("--shortcuts: this installer creates no shortcuts".into());
    }
    if s.autostart && !(cfg.install.autostart.is_some() && cfg.install.main_exe.is_some()) {
        return Err("--shortcuts: this installer does not offer autostart".into());
    }
    Ok(())
}

/// The components of a fresh install or a modify: `--components` (required ones added,
/// optional prerequisites as `prereq:<id>`), else `--preset` (default: recommended).
fn chosen_components(
    cfg: &InstallerConfig,
    list: Option<&[String]>,
    preset: Option<Preset>,
) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = cfg
        .components
        .iter()
        .filter(|c| c.required)
        .map(|c| c.id.clone())
        .collect();
    match list {
        Some(list) => {
            for id in list {
                let known = cfg.components.iter().any(|c| c.id == *id)
                    || id.strip_prefix("prereq:").is_some_and(|p| {
                        cfg.prerequisites.iter().any(|x| x.id == p && !x.required)
                    });
                if !known {
                    let valid: Vec<String> = cfg
                        .components
                        .iter()
                        .map(|c| c.id.clone())
                        .chain(
                            cfg.prerequisites
                                .iter()
                                .filter(|p| !p.required)
                                .map(|p| format!("prereq:{}", p.id)),
                        )
                        .collect();
                    return Err(format!(
                        "--components: unknown component {id:?} (this installer has: {})",
                        valid.join(", ")
                    ));
                }
                if !out.contains(id) {
                    out.push(id.clone());
                }
            }
        }
        None => {
            let preset = preset.unwrap_or(Preset::Recommended);
            for c in &cfg.components {
                let on = match preset {
                    Preset::Recommended => c.default,
                    Preset::Minimal => false,
                    Preset::Full => true,
                };
                if on && !out.contains(&c.id) {
                    out.push(c.id.clone());
                }
            }
        }
    }
    Ok(out)
}

/// The components an installation has: what it recorded, else (an install recorded before
/// the list existed) the required ones and every optional one with a file present.
fn installed_components(
    cfg: &InstallerConfig,
    info: &serde_json::Value,
    pkg_files: &[FileEntry],
    dir: &Path,
) -> Vec<String> {
    let required = cfg.components.iter().filter(|c| c.required);
    if let Some(rec) = info["components"].as_array() {
        if rec.is_empty() {
            // Recorded as "everything".
            return cfg.components.iter().map(|c| c.id.clone()).collect();
        }
        let mut out: Vec<String> = required.map(|c| c.id.clone()).collect();
        for v in rec.iter().filter_map(|v| v.as_str()) {
            if cfg.components.iter().any(|c| c.id == v) && !out.iter().any(|o| o == v) {
                out.push(v.to_string());
            }
        }
        return out;
    }
    cfg.components
        .iter()
        .filter(|c| {
            if c.required {
                return true;
            }
            let mut files = pkg_files
                .iter()
                .filter(|f| f.component.as_deref() == Some(c.id.as_str()))
                .peekable();
            if files.peek().is_none() {
                return c.default;
            }
            files.any(|f| dir.join(&f.path).exists())
        })
        .map(|c| c.id.clone())
        .collect()
}

/// The answers of an unattended fresh install.
///
/// - `--set` must name a real option with a value it can take, and never a licence;
/// - every option that sends data is OFF unless named (a child of a parent explicitly
///   turned on keeps its declared default, as when the parent is ticked in the window);
/// - a choice whose DEFAULT card would turn a data-sending option on must be answered
///   explicitly: « Accept all » is never implied.
fn unattended_choices(
    opts: &[SetupOption],
    sets: &[(String, String)],
    telemetry: Option<bool>,
) -> Result<BTreeMap<String, serde_json::Value>, String> {
    let mut explicit: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    let mut all: Vec<(String, String)> = sets.to_vec();
    if let Some(t) = telemetry {
        if !opts.iter().any(|o| o.id == "telemetry") {
            return Err("--telemetry: this installer has no telemetry option".into());
        }
        all.push(("telemetry".into(), if t { "on" } else { "off" }.into()));
    }
    for (id, raw) in &all {
        let Some(opt) = opts.iter().find(|o| o.id == *id) else {
            let ids: Vec<&str> = opts
                .iter()
                .filter(|o| !matches!(o.kind, SetupOptionKind::License))
                .map(|o| o.id.as_str())
                .collect();
            return Err(format!(
                "--set: unknown option {id:?} (this installer has: {})",
                ids.join(", ")
            ));
        };
        if matches!(opt.kind, SetupOptionKind::License) {
            return Err(format!(
                "--set {id}: terms are accepted with --accept-terms / --accept-privacy, never --set"
            ));
        }
        let v = crate::set_value(opt, raw).ok_or_else(|| match opt.kind {
            SetupOptionKind::Bool => format!("--set {id}: expected on or off, got {raw:?}"),
            _ => format!(
                "--set {id}: {raw:?} is not one of {}",
                opt.choices.join(", ")
            ),
        })?;
        explicit.insert(id.clone(), v);
    }

    let mut chosen = explicit.clone();
    for o in opts
        .iter()
        .filter(|o| o.sends_data && !explicit.contains_key(&o.id))
    {
        let parent_on = o
            .parent
            .as_ref()
            .is_some_and(|p| explicit.get(p).and_then(|v| v.as_bool()) == Some(true));
        if !parent_on && matches!(o.kind, SetupOptionKind::Bool) {
            chosen.insert(o.id.clone(), serde_json::json!(false));
        }
    }
    for o in opts.iter().filter(|o| !explicit.contains_key(&o.id)) {
        if let Some(card) = bpkg_core::config::picked_card(o, &chosen) {
            let turns_on = card.sets.iter().any(|s| {
                s.value == serde_json::json!(true)
                    && opts.iter().any(|x| x.id == s.option && x.sends_data)
            });
            if turns_on {
                return Err(format!(
                    "option {:?} defaults to {:?}, which turns on data-sending options: answer it \
                     with --set {}=<{}>",
                    o.id,
                    card.value,
                    o.id,
                    o.choices.join("|")
                ));
            }
        }
    }
    Ok(chosen)
}

/// `--help` lines specific to this installer: its components and setup options.
pub fn help_for_config(cfg: &InstallerConfig) -> String {
    let mut s = String::new();
    if !cfg.components.is_empty() {
        s.push_str("Components (--components):\n");
        for c in &cfg.components {
            s.push_str(&format!(
                "  {:<22}{}{}\n",
                c.id,
                c.name,
                if c.required {
                    " (required)"
                } else if c.default {
                    " (default)"
                } else {
                    ""
                }
            ));
        }
        for p in cfg.prerequisites.iter().filter(|p| !p.required) {
            s.push_str(&format!(
                "  {:<22}{} (optional prerequisite)\n",
                format!("prereq:{}", p.id),
                p.name
            ));
        }
    }
    let opts: Vec<&SetupOption> = cfg
        .setup_options
        .iter()
        .filter(|o| !matches!(o.kind, SetupOptionKind::License))
        .collect();
    if !opts.is_empty() {
        s.push_str("\nSetup options (--set <option>=<value>):\n");
        for o in opts {
            let values = match o.kind {
                SetupOptionKind::Bool => "on|off".to_string(),
                _ => o.choices.join("|"),
            };
            let def = match &o.default {
                serde_json::Value::Bool(b) => if *b { "on" } else { "off" }.to_string(),
                serde_json::Value::String(v) if !v.is_empty() => v.clone(),
                _ => "unanswered".to_string(),
            };
            let def = if o.sends_data {
                "off unless given (sends data)".to_string()
            } else {
                format!("default {def}")
            };
            s.push_str(&format!("  {:<22}{values}  ({def})\n", o.id));
        }
    }
    s
}

// ── The job ─────────────────────────────────────────────────────────────────

/// Where a job reports: the log always, the progress window in `--passive`.
struct Reporter {
    log: Arc<Log>,
    weak: Option<slint::Weak<MainWindow>>,
    cancel: Arc<AtomicBool>,
}

impl Reporter {
    fn report(&self, p: Option<f32>, label: String) {
        // The bar moves per percent; the log keeps the labels.
        self.log.line(&label);
        if let Some(w) = &self.weak {
            let _ = w.upgrade_in_event_loop(move |ui| {
                if let Some(p) = p {
                    ui.set_progress(p);
                }
                ui.set_progress_label(label.into());
            });
        }
    }
}

struct Job {
    plan: Plan,
    pkg: Option<PathBuf>,
    pkg_files: Vec<String>,
    dest: PathBuf,
    comps: Vec<String>,
    drop: Vec<String>,
    integ: SystemIntegration,
    tr: Translator,
    wait_pid: Option<u32>,
    wait_timeout: Duration,
    installed_version: String,
    update: Option<bpkg_core::config::UpdateConfig>,
    pinned: Result<Option<bpkg_core::sign::VerifyingKey>, String>,
    app_id: String,
    log: Arc<Log>,
}

impl Job {
    fn run(self, r: &Reporter) -> Result<String, Failure> {
        let tr = &self.tr;
        let report = |p: Option<f32>, s: String| r.report(p, s);
        let hooks = Hooks {
            report: &report,
            cancel: Some(&r.cancel),
            drop_components: &self.drop,
        };

        // The caller (BMM's updater) passes its own pid and exits: wait for it, then close
        // what is left of the app ourselves.
        if let Some(pid) = self.wait_pid {
            report(None, tr.t("progress.waiting_app"));
            if !crate::procs::wait_pid(pid, self.wait_timeout) {
                self.log.line(&format!(
                    "process {pid} still running after {} s: closing the app",
                    self.wait_timeout.as_secs()
                ));
                let files = if self.pkg_files.is_empty() {
                    crate::uninstall::plan(
                        &self.dest,
                        &crate::uninstall::read_info(&self.dest),
                        &[],
                    )
                    .files
                } else {
                    self.pkg_files.clone()
                };
                crate::kill_running_apps(&self.dest, &files);
                if !crate::procs::wait_pid(pid, Duration::from_secs(5)) {
                    return Err(Failure::new(
                        Exit::AppRunning,
                        format!(
                            "process {pid} did not exit and is not the app's: {}",
                            tr.t("errors.app_running")
                        ),
                    ));
                }
            }
        }
        hooks.check_cancel(tr)?;

        match self.plan {
            Plan::Fresh | Plan::Reinstall => {
                let pkg = self
                    .pkg
                    .as_deref()
                    .ok_or_else(|| Failure::new(Exit::Failed, "no package in this setup"))?;
                let n =
                    crate::run_real_install(&hooks, pkg, &self.dest, &self.comps, &self.integ, tr)?;
                Ok(format!("{n} files written to {}", self.dest.display()))
            }
            Plan::UpdateRemote => self.remote_update(&hooks),
            Plan::Uninstall => {
                report(Some(0.4), tr.t("progress.uninstalling"));
                let fallback = self.pkg_files.clone();
                crate::do_uninstall_full(&self.dest, &fallback)
                    .map_err(|e| Failure::new(Exit::Failed, e))?;
                Ok(format!("removed from {}", self.dest.display()))
            }
            Plan::UpToDate => Ok("up to date".into()),
        }
    }

    /// The bundled package is not newer: the signed `[update]` manifest, as the window's
    /// Update button does it (download, verify against the pinned key, apply with rollback).
    fn remote_update(&self, hooks: &Hooks) -> Result<String, Failure> {
        let tr = &self.tr;
        let Some(uc) = self.update.as_ref() else {
            return Ok("up to date".into());
        };
        let key = self
            .pinned
            .clone()
            .map_err(|e| Failure::new(Exit::Verification, e))?;
        if key.is_none() && self.integ.require_signature {
            return Err(Failure::new(
                Exit::Verification,
                tr.t("errors.signature_missing"),
            ));
        }
        let policy = crate::manifest_policy(key.as_ref(), &self.app_id, Some(&self.dest));
        let m = match bpkg_core::update::check_remote_multi(
            &uc.sources(),
            &self.installed_version,
            &policy,
        ) {
            Ok(Some(m)) => m,
            Ok(None) => return Ok(format!("up to date ({})", self.installed_version)),
            Err(e) => return Err(Failure::from_pkg(&e)),
        };
        (hooks.report)(
            Some(0.2),
            tr.t_with("progress.downloading", &[("new_version", &m.version)]),
        );
        hooks.check_cancel(tr)?;
        let cur_bpkg = if uc.allow_delta {
            self.pkg.clone()
        } else {
            None
        };
        let bytes =
            bpkg_core::update::download_update(&m, &self.installed_version, cur_bpkg.as_deref())
                .map_err(|e| Failure::from_pkg(&e))?;
        hooks.check_cancel(tr)?;
        let closing = tr.t("progress.closing");
        let n = crate::apply_remote_update(
            &bytes,
            &m,
            &self.installed_version,
            &self.dest,
            key.as_ref(),
            &self.app_id,
            |dir, files| {
                (hooks.report)(None, closing.clone());
                crate::kill_running_apps(dir, files);
                std::thread::sleep(Duration::from_millis(400));
            },
        )
        .map_err(|e| {
            let code = if e.contains("signature") || e.starts_with("update rejected") {
                Exit::Verification
            } else {
                Exit::Failed
            };
            Failure::new(code, e)
        })?;
        let mut msg = format!("updated to {} ({n} files)", m.version);
        if m.trust == bpkg_core::update::ManifestTrust::UnsignedAccepted {
            msg.push_str("; ");
            msg.push_str(&tr.t("done.update_unsigned_manifest"));
        }
        Ok(msg)
    }
}

// ── The progress window (--passive) ─────────────────────────────────────────

/// Run `job` on a worker thread behind the installer's own progress page. No question: the
/// window closes by itself on success; a failure stays on screen for 15 s (or until
/// closed) so it is not missed. Closing the window early asks to cancel, which takes effect
/// before anything is written.
fn with_window(
    cfg: &InstallerConfig,
    tr: &Translator,
    verb: &str,
    job: Job,
) -> Result<String, Failure> {
    if let Ok(backend) = i_slint_backend_winit::Backend::new() {
        let _ = slint::platform::set_platform(Box::new(backend));
    }
    let ui = MainWindow::new().map_err(|e| Failure::new(Exit::Failed, e.to_string()))?;
    let trc = Rc::new(RefCell::new(tr.clone()));
    {
        let trc = trc.clone();
        ui.global::<I18n>()
            .on_tr(move |key, _rev| trc.borrow().t(&key).into());
    }
    let g = ui.global::<I18n>();
    g.set_rtl(tr.direction() == Direction::Rtl);
    g.set_rev(g.get_rev() + 1);
    let light = match cfg.theme.mode.as_deref() {
        Some("light") => true,
        Some("dark") => false,
        _ => platform::prefers_light_theme().unwrap_or(false),
    };
    crate::apply_palette(&ui, light, &cfg.theme, cfg.branding.accent.as_deref());
    ui.set_app_name(cfg.app.name.clone().into());
    ui.set_app_version(cfg.app.version.clone().into());
    ui.set_publisher(cfg.app.publisher.clone().into());
    ui.set_maintenance(true);
    ui.set_maintenance_verb(verb.into());
    ui.set_steps(ModelRc::from(Rc::new(VecModel::from(vec![StepRow {
        label: verb.into(),
        state: 1,
    }]))));
    ui.set_page(3);
    ui.set_progress(0.0);
    ui.set_progress_label(tr.t("progress.preparing").into());

    let cancel = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));
    // Window controls: drag and minimise work; closing asks to cancel while working.
    {
        let w = ui.as_weak();
        ui.on_start_drag(move || {
            if let Some(ui) = w.upgrade() {
                use i_slint_backend_winit::WinitWindowAccessor;
                ui.window().with_winit_window(|win| {
                    let _ = win.drag_window();
                });
            }
        });
    }
    {
        let w = ui.as_weak();
        ui.on_minimize(move || {
            if let Some(ui) = w.upgrade() {
                use i_slint_backend_winit::WinitWindowAccessor;
                ui.window().with_winit_window(|win| win.set_minimized(true));
            }
        });
    }
    let ask_close: Rc<dyn Fn()> = Rc::new({
        let w = ui.as_weak();
        let cancel = cancel.clone();
        let finished = finished.clone();
        let label = tr.t("progress.cancelling");
        move || {
            if finished.load(Ordering::SeqCst) {
                let _ = slint::quit_event_loop();
            } else {
                cancel.store(true, Ordering::SeqCst);
                if let Some(ui) = w.upgrade() {
                    ui.set_progress_label(label.clone().into());
                }
            }
        }
    });
    {
        let f = ask_close.clone();
        ui.on_close_window(move || f());
    }
    ui.on_finish(|| {
        let _ = slint::quit_event_loop();
    });
    {
        let f = ask_close.clone();
        let finished = finished.clone();
        ui.window().on_close_requested(move || {
            f();
            if finished.load(Ordering::SeqCst) {
                slint::CloseRequestResponse::HideWindow
            } else {
                slint::CloseRequestResponse::KeepWindowShown
            }
        });
    }

    let result: Arc<Mutex<Option<Result<String, Failure>>>> = Arc::new(Mutex::new(None));
    let worker = {
        let weak = ui.as_weak();
        let log = job.log.clone();
        let cancel = cancel.clone();
        let result = result.clone();
        let finished = finished.clone();
        let title = tr.t("done.install_failed_title");
        std::thread::spawn(move || {
            let r = Reporter {
                log,
                weak: Some(weak.clone()),
                cancel,
            };
            let res = job.run(&r);
            *result.lock().unwrap() = Some(res.clone());
            finished.store(true, Ordering::SeqCst);
            let _ = weak.upgrade_in_event_loop(move |ui| match res {
                Ok(_) => {
                    ui.set_progress(1.0);
                    let _ = slint::quit_event_loop();
                }
                Err(f) => {
                    ui.set_success(false);
                    ui.set_result_title(title.into());
                    ui.set_result_message(f.message.into());
                    ui.set_progress(1.0);
                    ui.set_page(4);
                    slint::Timer::single_shot(Duration::from_secs(15), || {
                        let _ = slint::quit_event_loop();
                    });
                }
            });
        })
    };
    if let Err(e) = ui.run() {
        // No window could be shown (no display): the work goes on without one.
        eprintln!("progress window unavailable: {e}");
    }
    let _ = worker.join();
    let out = result.lock().unwrap().take().unwrap_or_else(|| {
        Err(Failure::new(
            Exit::Failed,
            "the installer stopped unexpectedly",
        ))
    });
    out
}

// ── System helpers ──────────────────────────────────────────────────────────

/// Bytes still to write: each selected file's size, less what an existing file of that
/// path already occupies (an update overwrites in place).
fn bytes_needed(files: &[FileEntry], comps: &[String], dest: &Path) -> u64 {
    files
        .iter()
        .filter(|f| match (&f.component, comps.is_empty()) {
            (Some(c), false) => comps.contains(c),
            _ => true,
        })
        .map(|f| {
            let have = std::fs::metadata(dest.join(&f.path))
                .map(|m| m.len())
                .unwrap_or(0);
            f.size.saturating_sub(have)
        })
        .sum()
}

/// The nearest folder of `p` that exists (`p` itself, or a parent).
fn existing_ancestor(p: &Path) -> Option<&Path> {
    let mut cur = p;
    loop {
        if cur.is_dir() {
            return Some(cur);
        }
        cur = cur.parent()?;
    }
}

/// Can this process create files in `dest` (or, when it does not exist yet, in the folder
/// it would be created in)?
fn writable(dest: &Path) -> bool {
    let Some(dir) = existing_ancestor(dest) else {
        return false;
    };
    let probe = dir.join(format!(".bi-write-probe-{}", std::process::id()));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(f) => {
            drop(f);
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(e) => e.kind() != std::io::ErrorKind::PermissionDenied,
    }
}

#[cfg(windows)]
fn free_space(dest: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    extern "system" {
        fn GetDiskFreeSpaceExW(
            dir: *const u16,
            free_to_caller: *mut u64,
            total: *mut u64,
            total_free: *mut u64,
        ) -> i32;
    }
    let dir = existing_ancestor(dest)?;
    let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut free = 0u64;
    // SAFETY: a documented kernel32 call with a NUL-terminated path and valid out-pointers.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut free,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(free)
}

#[cfg(not(windows))]
fn free_space(_dest: &Path) -> Option<u64> {
    None
}

/// One argument quoted for a Windows command line (the rules CommandLineToArgvW and the
/// Rust runtime parse back): backslashes are literal except before a quote.
fn quote_arg(a: &str) -> String {
    if !a.is_empty() && !a.contains([' ', '\t', '\n', '"']) {
        return a.to_string();
    }
    let mut out = String::from('"');
    let mut backslashes = 0usize;
    for c in a.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            c => {
                out.push_str(&"\\".repeat(backslashes));
                backslashes = 0;
                out.push(c);
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

/// The same command line, for the elevated copy: every argument as given, plus `--elevated`
/// so it never asks again.
fn elevated_args(raw: &[String]) -> String {
    raw.iter()
        .map(|a| quote_arg(a))
        .chain(std::iter::once("--elevated".to_string()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Ask for administrator rights once (UAC) by starting this setup again elevated with the
/// same arguments, wait for it and take its exit code. A refused prompt is `Elevation`.
#[cfg(windows)]
fn elevate() -> Result<Exit, Failure> {
    use std::os::windows::ffi::OsStrExt;
    #[repr(C)]
    struct ShellExecuteInfoW {
        cb_size: u32,
        f_mask: u32,
        hwnd: isize,
        verb: *const u16,
        file: *const u16,
        parameters: *const u16,
        directory: *const u16,
        show: i32,
        inst_app: isize,
        id_list: *mut core::ffi::c_void,
        class: *const u16,
        hkey_class: isize,
        hot_key: u32,
        icon_or_monitor: isize,
        process: isize,
    }
    #[link(name = "shell32")]
    extern "system" {
        fn ShellExecuteExW(info: *mut ShellExecuteInfoW) -> i32;
    }
    extern "system" {
        fn GetLastError() -> u32;
        fn WaitForSingleObject(h: isize, ms: u32) -> u32;
        fn GetExitCodeProcess(h: isize, code: *mut u32) -> i32;
        fn CloseHandle(h: isize) -> i32;
    }
    const SEE_MASK_NOCLOSEPROCESS: u32 = 0x0000_0040;
    const SEE_MASK_NOASYNC: u32 = 0x0000_0100;
    const ERROR_CANCELLED: u32 = 1223;
    const INFINITE: u32 = 0xFFFF_FFFF;
    let wide = |s: &std::ffi::OsStr| -> Vec<u16> { s.encode_wide().chain(Some(0)).collect() };

    let exe = std::env::current_exe().map_err(|e| Failure::new(Exit::Failed, e.to_string()))?;
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let verb = wide(std::ffi::OsStr::new("runas"));
    let file = wide(exe.as_os_str());
    let params = wide(std::ffi::OsStr::new(&elevated_args(&raw)));
    let mut info = ShellExecuteInfoW {
        cb_size: std::mem::size_of::<ShellExecuteInfoW>() as u32,
        f_mask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
        hwnd: 0,
        verb: verb.as_ptr(),
        file: file.as_ptr(),
        parameters: params.as_ptr(),
        directory: std::ptr::null(),
        show: 1,
        inst_app: 0,
        id_list: std::ptr::null_mut(),
        class: std::ptr::null(),
        hkey_class: 0,
        hot_key: 0,
        icon_or_monitor: 0,
        process: 0,
    };
    // SAFETY: documented shell32/kernel32 calls; every string is NUL-terminated and outlives
    // the call; the process handle is closed on every path.
    unsafe {
        if ShellExecuteExW(&mut info) == 0 {
            let err = GetLastError();
            return Err(Failure::new(
                Exit::Elevation,
                if err == ERROR_CANCELLED {
                    "administrator rights were refused (UAC prompt declined)".to_string()
                } else {
                    format!("could not ask for administrator rights (error {err})")
                },
            ));
        }
        if info.process == 0 {
            return Err(Failure::new(
                Exit::Failed,
                "the elevated copy did not start",
            ));
        }
        WaitForSingleObject(info.process, INFINITE);
        let mut code = 1u32;
        GetExitCodeProcess(info.process, &mut code);
        CloseHandle(info.process);
        Ok(Exit::TABLE
            .iter()
            .map(|(e, _)| *e)
            .find(|e| e.code() as u32 == code)
            .unwrap_or(Exit::Failed))
    }
}

#[cfg(not(windows))]
fn elevate() -> Result<Exit, Failure> {
    Err(Failure::new(
        Exit::Elevation,
        "the target folder needs root: run the setup with sudo",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BMM: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/bmm/installer.toml"
    );

    fn bmm() -> InstallerConfig {
        InstallerConfig::load(BMM).unwrap()
    }

    fn sets(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    fn effective(
        cfg: &InstallerConfig,
        chosen: &BTreeMap<String, serde_json::Value>,
        id: &str,
    ) -> serde_json::Value {
        let o = cfg.setup_options.iter().find(|o| o.id == id).unwrap();
        bpkg_core::config::effective_value(&cfg.setup_options, o, chosen)
    }

    #[test]
    fn nothing_that_sends_data_is_on_by_default() {
        let cfg = bmm();
        let chosen = unattended_choices(&cfg.setup_options, &[], None).unwrap();
        for o in cfg.setup_options.iter().filter(|o| o.sends_data) {
            assert_eq!(
                effective(&cfg, &chosen, &o.id),
                serde_json::json!(false),
                "{} is on in an unattended install nobody answered",
                o.id
            );
        }
        // The privacy choice stays unanswered: the app asks it on first launch.
        assert!(!chosen.contains_key("privacy_choice"));
        let doc = handoff::build(&cfg.setup_options, &chosen, vec![], "1", "x");
        assert_ne!(
            doc.settings.get("privacy_choice"),
            Some(&serde_json::json!("all"))
        );
    }

    #[test]
    fn accept_all_only_when_named() {
        let cfg = bmm();
        let chosen = unattended_choices(
            &cfg.setup_options,
            &sets(&[("privacy_choice", "all")]),
            None,
        )
        .unwrap();
        assert_eq!(
            effective(&cfg, &chosen, "telemetry"),
            serde_json::json!(true)
        );
        let chosen = unattended_choices(&cfg.setup_options, &[], Some(true)).unwrap();
        assert_eq!(
            effective(&cfg, &chosen, "telemetry"),
            serde_json::json!(true)
        );
        // Its categories keep their declared defaults, as when the box is ticked by hand...
        assert_eq!(
            effective(&cfg, &chosen, "telemetry_usage"),
            serde_json::json!(true)
        );
        // ...and the one that is off by default stays off.
        assert_eq!(
            effective(&cfg, &chosen, "telemetry_bench"),
            serde_json::json!(false)
        );
        // Something else that sends data is still off.
        assert_eq!(
            effective(&cfg, &chosen, "discord_rpc"),
            serde_json::json!(false)
        );
    }

    #[test]
    fn a_default_card_that_turns_data_on_must_be_answered() {
        let mut cfg = bmm();
        let o = cfg
            .setup_options
            .iter_mut()
            .find(|o| o.id == "privacy_choice")
            .unwrap();
        o.default = serde_json::json!("all");
        let e = unattended_choices(&cfg.setup_options, &[], None).unwrap_err();
        assert!(e.contains("privacy_choice"), "{e}");
        assert!(unattended_choices(
            &cfg.setup_options,
            &sets(&[("privacy_choice", "essential")]),
            None
        )
        .is_ok());
    }

    #[test]
    fn bad_set_values_are_bad_arguments() {
        let cfg = bmm();
        for bad in [
            ("nope", "on"),
            ("telemetry", "maybe"),
            ("theme", "not-a-theme"),
            ("legal", "true"),
        ] {
            assert!(
                unattended_choices(&cfg.setup_options, &sets(&[bad]), None).is_err(),
                "{bad:?} accepted"
            );
        }
        let ok = unattended_choices(
            &cfg.setup_options,
            &sets(&[("theme", "bmm-nord"), ("smart_io", "off")]),
            Some(false),
        )
        .unwrap();
        assert_eq!(ok.get("theme"), Some(&serde_json::json!("bmm-nord")));
        assert_eq!(ok.get("smart_io"), Some(&serde_json::json!(false)));
    }

    #[test]
    fn components_from_flags_and_presets() {
        let cfg = bmm();
        assert_eq!(
            chosen_components(&cfg, None, None).unwrap(),
            vec!["core", "mcp-server", "cli"]
        );
        assert_eq!(
            chosen_components(&cfg, None, Some(Preset::Minimal)).unwrap(),
            vec!["core"]
        );
        // Required ones are always added.
        assert_eq!(
            chosen_components(&cfg, Some(&["cli".to_string()]), None).unwrap(),
            vec!["core", "cli"]
        );
        assert_eq!(
            chosen_components(&cfg, Some(&["prereq:python".to_string()]), None).unwrap(),
            vec!["core", "prereq:python"]
        );
        assert!(chosen_components(&cfg, Some(&["nope".to_string()]), None).is_err());
    }

    #[test]
    fn installed_components_come_from_the_record_else_the_files() {
        let cfg = bmm();
        let info = serde_json::json!({ "components": ["core", "cli", "gone"] });
        assert_eq!(
            installed_components(&cfg, &info, &[], Path::new(".")),
            vec!["core", "cli"]
        );
        // No record: an optional component counts when one of its files is there.
        let dir = std::env::temp_dir().join(format!("bi-comps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("bmm-cli.exe"), b"x").unwrap();
        let fe = |p: &str, c: &str| FileEntry {
            path: p.into(),
            size: 1,
            sha256: String::new(),
            component: Some(c.into()),
            executable: false,
        };
        let files = [
            fe("bmm-cli.exe", "cli"),
            fe("bmm-mcp-server.exe", "mcp-server"),
        ];
        let got = installed_components(&cfg, &serde_json::Value::Null, &files, &dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(got, vec!["core", "cli"]);
    }

    #[test]
    fn bytes_needed_counts_only_what_is_missing() {
        let dir = std::env::temp_dir().join(format!("bi-space-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.bin"), vec![0u8; 40]).unwrap();
        let fe = |p: &str, size: u64, c: Option<&str>| FileEntry {
            path: p.into(),
            size,
            sha256: String::new(),
            component: c.map(str::to_string),
            executable: false,
        };
        let files = [
            fe("a.bin", 100, None),
            fe("b.bin", 50, Some("cli")),
            fe("c.bin", 7, Some("mcp")),
        ];
        let all = bytes_needed(&files, &[], &dir);
        let some = bytes_needed(&files, &["cli".into()], &dir);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(all, 60 + 50 + 7);
        assert_eq!(some, 60 + 50);
    }

    #[test]
    fn windows_command_line_quoting_round_trips() {
        assert_eq!(quote_arg("--update"), "--update");
        assert_eq!(quote_arg(""), "\"\"");
        assert_eq!(
            quote_arg("C:\\Program Files\\BMM"),
            "\"C:\\Program Files\\BMM\""
        );
        assert_eq!(quote_arg("C:\\a b\\"), "\"C:\\a b\\\\\"");
        assert_eq!(quote_arg("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(
            elevated_args(&[
                "--mode".into(),
                "install".into(),
                "--dir".into(),
                "C:\\P F".into()
            ]),
            "--mode install --dir \"C:\\P F\" --elevated"
        );
    }

    #[test]
    fn errors_are_classified() {
        use bpkg_core::error::Error as E;
        assert_eq!(
            Failure::from_pkg(&E::HashMismatch {
                path: "a".into(),
                expected: "1".into(),
                actual: "2".into()
            })
            .code,
            Exit::Verification
        );
        assert_eq!(Failure::from_pkg(&E::BadMagic).code, Exit::Verification);
        assert_eq!(
            Failure::from_pkg(&E::Other("update rejected: no".into())).code,
            Exit::Verification
        );
        assert_eq!(
            Failure::from_pkg(&E::Other("network".into())).code,
            Exit::Failed
        );
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert_eq!(io_exit(&denied), Exit::Elevation);
        #[cfg(windows)]
        {
            assert_eq!(io_exit(&std::io::Error::from_raw_os_error(112)), Exit::Disk);
            assert_eq!(
                io_exit(&std::io::Error::from_raw_os_error(32)),
                Exit::AppRunning
            );
        }
    }

    #[test]
    fn shortcuts_must_be_offered() {
        let cfg = bmm();
        let s = |sm, d, a| cli::Shortcuts {
            start_menu: sm,
            desktop: d,
            autostart: a,
        };
        assert!(check_shortcuts(&cfg, s(true, true, false)).is_ok());
        // BMM declares `autostart = false`: offered, off by default.
        assert!(check_shortcuts(&cfg, s(false, false, true)).is_ok());
        let mut none = cfg.clone();
        none.install.create_shortcuts = false;
        none.install.autostart = None;
        assert!(check_shortcuts(&none, s(false, true, false)).is_err());
        assert!(check_shortcuts(&none, s(false, false, true)).is_err());
        assert!(check_shortcuts(&none, s(false, false, false)).is_ok());
    }

    #[test]
    fn help_lists_this_installers_components_and_options() {
        let h = help_for_config(&bmm());
        for want in [
            "mcp-server",
            "prereq:python",
            "privacy_choice",
            "telemetry",
            "theme",
        ] {
            assert!(h.contains(want), "help misses {want}");
        }
        assert!(!h.contains("legal "), "a licence is never a --set");
        assert!(h.contains("off unless given (sends data)"));
    }

    #[test]
    fn writable_probe_on_a_temp_folder() {
        let d = std::env::temp_dir()
            .join(format!("bi-w-{}", std::process::id()))
            .join("not-yet");
        assert!(writable(&d), "the temp folder must be writable");
    }
}
