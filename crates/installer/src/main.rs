#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! BetterInstaller GUI (Phase 2 + 2.5 handoff).
//!
//! Loads an `installer.toml`, renders the Welcome + Configuration flow in Slint,
//! and on Install writes the real `installer-handoff.json` (the app reads it once
//! on first launch). Actual file extraction wires in during Phase 3; here the
//! progress is simulated so the end-to-end flow + handoff are demonstrable.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use slint::{
    Color, ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel, Weak,
};

use bpkg_core::config::{InstallerConfig, SetupOption, SetupOptionKind};
use bpkg_core::handoff;
use bpkg_core::manifest::AppMeta;
use bpkg_core::package::Package;
use bpkg_core::platform::{self, ShortcutSpec, UninstallEntry};
use std::path::Path;

/// Everything the install step needs to integrate with the OS (shortcuts,
/// protocol, uninstaller). Send + Clone so it crosses into the worker thread.
#[derive(Clone)]
struct SystemIntegration {
    app: AppMeta,
    main_exe: Option<String>,
    protocol: Option<String>,
    create_shortcuts: bool,
    desktop: bool,
    /// Hex Ed25519 public key the package must verify against (if any).
    public_key: Option<String>,
    /// Abort the install if the signature is missing/invalid.
    require_signature: bool,
    /// Prerequisites to verify before installing.
    prereqs: Vec<bpkg_core::config::Prerequisite>,
}

slint::include_modules!();

fn main() -> anyhow::Result<()> {
    // Uninstall mode: triggered from the ARP "Uninstall" button
    // (`uninstall.exe --uninstall`). Reverses the system integration + removes files.
    if std::env::args().any(|a| a == "--uninstall") {
        return run_uninstall();
    }

    let (cfg, package_path) = resolve_sources()?;
    run_gui(cfg, package_path)
}

/// Config + payload come from the embedded self-extracting blob (an exe built
/// with `bpkg build`) when present, else from CLI args (dev mode:
/// `<installer.toml> [package.bpkg]`).
fn resolve_sources() -> anyhow::Result<(InstallerConfig, Option<PathBuf>)> {
    if let Some(emb) = std::env::current_exe()
        .ok()
        .and_then(|e| bpkg_core::embed::read_embedded(&e).ok().flatten())
    {
        let cfg = InstallerConfig::from_toml(&String::from_utf8_lossy(&emb.config))
            .map_err(|e| anyhow::anyhow!("embedded config: {e}"))?;
        // Stage the embedded .bpkg to a temp file so Package::open can read it.
        let tmp = std::env::temp_dir().join(format!("betterinstaller-{}.bpkg", std::process::id()));
        std::fs::write(&tmp, &emb.bpkg)?;
        return Ok((cfg, Some(tmp)));
    }

    let mut args = std::env::args().skip(1);
    let config_path = args
        .next()
        .unwrap_or_else(|| "examples/bmm/installer.toml".to_string());
    let package_path: Option<PathBuf> = args.next().map(PathBuf::from);
    let cfg = InstallerConfig::load(&config_path)
        .map_err(|e| anyhow::anyhow!("loading {config_path}: {e}"))?;
    Ok((cfg, package_path))
}

fn run_gui(cfg: InstallerConfig, package_path: Option<PathBuf>) -> anyhow::Result<()> {
    let plat = platform::current();
    let app_meta = AppMeta {
        id: cfg.app.id.clone(),
        name: cfg.app.name.clone(),
        version: cfg.app.version.clone(),
        publisher: cfg.app.publisher.clone(),
        homepage: cfg.app.homepage.clone(),
        platforms: cfg.app.platforms.clone(),
    };

    let ui = MainWindow::new()?;
    ui.set_app_name(cfg.app.name.clone().into());
    ui.set_app_version(cfg.app.version.clone().into());
    ui.set_publisher(cfg.app.publisher.clone().into());
    ui.set_install_dir(
        plat.default_install_dir(&app_meta)
            .to_string_lossy()
            .to_string()
            .into(),
    );
    if let Some(accent) = cfg.branding.accent.as_deref().and_then(parse_hex) {
        ui.set_accent(accent);
    }

    // UI language: a non-"auto" default on the `language` setup option, else the OS.
    let lang = cfg
        .setup_options
        .iter()
        .find(|o| o.id == "language")
        .and_then(|o| o.default.as_str())
        .filter(|l| *l != "auto")
        .map(str::to_string)
        .unwrap_or_else(bpkg_core::i18n::detect_lang);
    {
        use bpkg_core::i18n::t;
        ui.set_t_next(t(&lang, "next").into());
        ui.set_t_back(t(&lang, "back").into());
        ui.set_t_install(t(&lang, "install").into());
        ui.set_t_finish(t(&lang, "finish").into());
        ui.set_t_config_title(t(&lang, "config_title").into());
        ui.set_t_config_hint(t(&lang, "config_hint").into());
        ui.set_t_install_loc(t(&lang, "install_loc").into());
        ui.set_t_installing(t(&lang, "installing").into());
        ui.set_t_accept(t(&lang, "accept").into());
    }

    // Shared mutable state captured by callbacks.
    let setup_opts = Rc::new(cfg.setup_options.clone());
    let chosen: Rc<RefCell<BTreeMap<String, serde_json::Value>>> =
        Rc::new(RefCell::new(BTreeMap::new()));

    // Build the options model for the Configuration page.
    let rows: Vec<OptionRow> = setup_opts.iter().map(to_row).collect();
    let model = Rc::new(VecModel::from(rows));
    ui.set_options(ModelRc::from(model.clone()));
    ui.set_can_proceed(compute_can_proceed(&setup_opts, &chosen.borrow()));

    // ── Navigation ──────────────────────────────────────────────────────
    {
        let w = ui.as_weak();
        ui.on_go_next(move || {
            if let Some(ui) = w.upgrade() {
                ui.set_page(1);
            }
        });
    }
    {
        let w = ui.as_weak();
        ui.on_go_back(move || {
            if let Some(ui) = w.upgrade() {
                ui.set_page(0);
            }
        });
    }
    {
        let w = ui.as_weak();
        ui.on_finish(move || {
            let _ = w;
            let _ = slint::quit_event_loop();
        });
    }

    // ── Option changes ──────────────────────────────────────────────────
    {
        let w = ui.as_weak();
        let model = model.clone();
        let chosen = chosen.clone();
        let opts = setup_opts.clone();
        ui.on_option_bool_changed(move |id, v| {
            chosen
                .borrow_mut()
                .insert(id.to_string(), serde_json::json!(v));
            set_row(&model, &id, |r| r.bool_value = v);
            if let Some(ui) = w.upgrade() {
                ui.set_can_proceed(compute_can_proceed(&opts, &chosen.borrow()));
            }
        });
    }
    {
        let model = model.clone();
        let chosen = chosen.clone();
        ui.on_option_select_changed(move |id, v| {
            chosen
                .borrow_mut()
                .insert(id.to_string(), serde_json::json!(v.to_string()));
            set_row(&model, &id, |r| r.string_value = v.clone());
        });
    }

    // ── Install ─────────────────────────────────────────────────────────
    // Pre-compute everything the install closure needs (Box<dyn PlatformOps>
    // isn't Clone, so resolve the handoff directory up front).
    let handoff_cfg = cfg.handoff.clone();
    let app_data_dir = plat.app_data_dir(&app_meta);
    let components: Vec<String> = cfg
        .components
        .iter()
        .filter(|c| c.required || c.default)
        .map(|c| c.id.clone())
        .collect();
    let app_version = cfg.app.version.clone();
    let integ = SystemIntegration {
        app: app_meta.clone(),
        main_exe: cfg.install.main_exe.clone(),
        protocol: cfg.install.protocol.clone(),
        create_shortcuts: cfg.install.create_shortcuts,
        desktop: cfg.install.desktop_shortcut,
        public_key: cfg.security.as_ref().and_then(|s| s.public_key.clone()),
        require_signature: cfg
            .security
            .as_ref()
            .map(|s| s.require_signature)
            .unwrap_or(false),
        prereqs: cfg.prerequisites.clone(),
    };

    let prog_timer: Rc<RefCell<Option<Timer>>> = Rc::new(RefCell::new(None));
    {
        let w = ui.as_weak();
        let chosen = chosen.clone();
        let opts = setup_opts.clone();
        let prog_timer = prog_timer.clone();
        ui.on_install(move || {
            let ui = match w.upgrade() {
                Some(u) => u,
                None => return,
            };

            // 1) Write the real handoff file (the headline feature).
            let mut message = String::new();
            let mut ok = true;
            if let Some(h) = handoff_cfg.as_ref().filter(|h| h.enabled) {
                let doc = handoff::build(
                    &opts,
                    &chosen.borrow(),
                    components.clone(),
                    &app_version,
                    bpkg_core::VERSION,
                );
                let dir = match h.location {
                    bpkg_core::config::HandoffLocation::AppData => app_data_dir.clone(),
                    bpkg_core::config::HandoffLocation::InstallDir => {
                        PathBuf::from(ui.get_install_dir().to_string())
                    }
                };
                let path = dir.join(&h.file);
                match doc.write_atomic(&path) {
                    Ok(()) => message = format!("First-run config written to {}", path.display()),
                    Err(e) => {
                        ok = false;
                        message = format!("Could not write first-run config: {e}");
                    }
                }
            }

            // 2) Copy the files.
            ui.set_page(2);
            ui.set_progress(0.0);
            ui.set_progress_label("Preparing…".into());

            match package_path.clone() {
                // Real install: verify + extract the .bpkg on a worker thread,
                // pushing progress back to the UI thread.
                Some(pkg) => {
                    let dest = PathBuf::from(ui.get_install_dir().to_string());
                    let comps = components.clone();
                    let weak = ui.as_weak();
                    let handoff_msg = message.clone();
                    let handoff_ok = ok;
                    let integ = integ.clone();
                    std::thread::spawn(move || {
                        let result = run_real_install(weak.clone(), &pkg, &dest, &comps, &integ);
                        let _ = weak.upgrade_in_event_loop(move |ui| {
                            match result {
                                Ok(n) => {
                                    ui.set_success(handoff_ok);
                                    ui.set_result_message(
                                        format!(
                                            "Installed {n} files to {}\n{}",
                                            dest.display(),
                                            handoff_msg
                                        )
                                        .into(),
                                    );
                                }
                                Err(e) => {
                                    ui.set_success(false);
                                    ui.set_result_message(format!("Install failed: {e}").into());
                                }
                            }
                            ui.set_progress(1.0);
                            ui.set_page(3);
                        });
                    });
                }
                // No package supplied: simulated progress (UI preview).
                None => {
                    let w2 = ui.as_weak();
                    let pt = prog_timer.clone();
                    let progress = Rc::new(RefCell::new(0.0f32));
                    let final_msg = message.clone();
                    let timer = Timer::default();
                    timer.start(TimerMode::Repeated, Duration::from_millis(50), move || {
                        let ui = match w2.upgrade() {
                            Some(u) => u,
                            None => return,
                        };
                        let mut p = progress.borrow_mut();
                        *p += 0.035;
                        if *p >= 1.0 {
                            ui.set_progress(1.0);
                            ui.set_success(ok);
                            ui.set_result_message(final_msg.clone().into());
                            ui.set_page(3);
                            if let Some(t) = pt.borrow().as_ref() {
                                t.stop();
                            }
                        } else {
                            ui.set_progress(*p);
                            ui.set_progress_label(
                                format!("Installing…  {}%", (*p * 100.0) as i32).into(),
                            );
                        }
                    });
                    *prog_timer.borrow_mut() = Some(timer);
                }
            }
        });
    }

    ui.run()?;
    Ok(())
}

/// Verify + extract a package on a worker thread, pushing throttled (per whole
/// percent) progress to the UI. Returns the number of files installed.
fn run_real_install(
    weak: Weak<MainWindow>,
    pkg: &Path,
    dest: &Path,
    comps: &[String],
    integ: &SystemIntegration,
) -> Result<u64, String> {
    // Prerequisites must be satisfied before touching anything.
    let missing: Vec<String> = integ
        .prereqs
        .iter()
        .filter(|p| p.required && !bpkg_core::prereq::check(p))
        .map(|p| p.name.clone())
        .collect();
    if !missing.is_empty() {
        return Err(format!("Missing prerequisites: {}", missing.join(", ")));
    }

    let mut p = Package::open(pkg).map_err(|e| e.to_string())?;

    // Verify the Ed25519 signature before writing anything, when a trust key is set.
    if let Some(pk_hex) = integ.public_key.as_ref() {
        let vk = bpkg_core::sign::parse_public(pk_hex).map_err(|e| e.to_string())?;
        let valid = p.verify_signature(&vk).map_err(|e| e.to_string())?;
        if !valid && integ.require_signature {
            return Err(if p.is_signed() {
                "package signature is INVALID — refusing to install".to_string()
            } else {
                "package is not signed but a signature is required".to_string()
            });
        }
    }

    let comp: Option<&[String]> = if comps.is_empty() { None } else { Some(comps) };
    let mut last_pct = -1i32;
    let written = p
        .install_with_progress(dest, comp, |done, total, file| {
            let pct = if total > 0 {
                (done * 100 / total) as i32
            } else {
                100
            };
            if pct != last_pct {
                last_pct = pct;
                let fname = file.to_string();
                let _ = weak.upgrade_in_event_loop(move |ui| {
                    ui.set_progress(pct as f32 / 100.0);
                    ui.set_progress_label(format!("Installing…  {pct}%  ·  {fname}").into());
                });
            }
        })
        .map_err(|e| e.to_string())?;

    // After files land: shortcuts, protocol, uninstaller (best-effort).
    let _ = weak.upgrade_in_event_loop(|ui| {
        ui.set_progress_label("Finishing up…".into());
    });
    do_system_integration(dest, integ);
    Ok(written)
}

/// Register the app with the OS: shortcuts, custom URL scheme, and the
/// Add/Remove-Programs uninstaller. All steps are best-effort (a failed shortcut
/// never fails the whole install). Also drops `uninstall-info.json` so a later
/// `--uninstall` can reverse exactly what was done.
fn do_system_integration(dest: &Path, integ: &SystemIntegration) {
    let plat = platform::current();

    if let Some(exe_rel) = integ.main_exe.as_ref() {
        let exe = dest.join(exe_rel);
        if integ.create_shortcuts {
            let _ = plat.create_shortcuts(&ShortcutSpec {
                name: integ.app.name.clone(),
                target: exe.clone(),
                icon: None,
                desktop: integ.desktop,
                start_menu: true,
            });
        }
        if let Some(scheme) = integ.protocol.as_ref() {
            let _ = plat.register_protocol(scheme, &exe);
        }
    }

    // Copy ourselves in as the uninstaller and register the ARP entry.
    if let Ok(self_exe) = std::env::current_exe() {
        let uninstaller = dest.join("uninstall.exe");
        let _ = std::fs::copy(&self_exe, &uninstaller);
        let _ = plat.register_uninstaller(&UninstallEntry {
            app: integ.app.clone(),
            install_dir: dest.to_path_buf(),
            uninstaller,
        });
    }

    // Record what to reverse on uninstall.
    let info = serde_json::json!({
        "app_id": integ.app.id,
        "app_name": integ.app.name,
        "protocol": integ.protocol,
        "shortcut_name": integ.app.name,
        "desktop": integ.create_shortcuts && integ.desktop,
        "start_menu": integ.create_shortcuts,
        "install_dir": dest.to_string_lossy(),
    });
    if let Ok(bytes) = serde_json::to_vec_pretty(&info) {
        let _ = std::fs::write(dest.join("uninstall-info.json"), bytes);
    }
}

/// Reverse a previous install: remove shortcuts, unregister the protocol + ARP
/// entry, and delete the install directory (except the running uninstaller).
fn run_uninstall() -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let dir = exe
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let info: serde_json::Value = std::fs::read(dir.join("uninstall-info.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(serde_json::Value::Null);

    let plat = platform::current();
    if let Some(name) = info["shortcut_name"].as_str() {
        let _ = plat.remove_shortcuts(
            name,
            info["desktop"].as_bool().unwrap_or(false),
            info["start_menu"].as_bool().unwrap_or(false),
        );
    }
    if let Some(scheme) = info["protocol"].as_str() {
        let _ = plat.unregister_protocol(scheme);
    }
    if let Some(id) = info["app_id"].as_str() {
        let _ = plat.unregister_uninstaller(id);
    }

    // Remove every install file except the running uninstaller (which Windows
    // locks while it runs — the now-near-empty folder can be removed afterwards).
    remove_dir_except(&dir, &exe);
    Ok(())
}

fn remove_dir_except(dir: &Path, keep: &Path) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p == keep {
                continue;
            }
            if p.is_dir() {
                let _ = std::fs::remove_dir_all(&p);
            } else {
                let _ = std::fs::remove_file(&p);
            }
        }
    }
}

/// Map a [`SetupOption`] to the Slint row struct.
fn to_row(o: &SetupOption) -> OptionRow {
    let kind = match o.kind {
        SetupOptionKind::Bool => "bool",
        SetupOptionKind::Select => "select",
        SetupOptionKind::License => "license",
    };
    let choices: Vec<SharedString> = o.choices.iter().map(|c| c.clone().into()).collect();
    OptionRow {
        id: o.id.clone().into(),
        kind: kind.into(),
        label: humanize(&o.label_key).into(),
        choices: ModelRc::from(Rc::new(VecModel::from(choices))),
        bool_value: o.default.as_bool().unwrap_or(false),
        string_value: o.default.as_str().unwrap_or("").into(),
    }
}

/// Update the model row whose `id` matches.
fn set_row(model: &VecModel<OptionRow>, id: &str, f: impl Fn(&mut OptionRow)) {
    for i in 0..model.row_count() {
        if let Some(mut r) = model.row_data(i) {
            if r.id == id {
                f(&mut r);
                model.set_row_data(i, r);
                break;
            }
        }
    }
}

/// "Next" is gated on every required `license` option being accepted.
fn compute_can_proceed(opts: &[SetupOption], chosen: &BTreeMap<String, serde_json::Value>) -> bool {
    opts.iter().all(|o| {
        if o.required && matches!(o.kind, SetupOptionKind::License) {
            chosen.get(&o.id).and_then(|v| v.as_bool()).unwrap_or(false)
        } else {
            true
        }
    })
}

/// "setup.skip_tutorial" → "Skip tutorial". A stand-in until i18n (Phase 7).
fn humanize(key: &str) -> String {
    let last = key.rsplit('.').next().unwrap_or(key);
    let spaced = last.replace('_', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => spaced,
    }
}

fn parse_hex(s: &str) -> Option<Color> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some(Color::from_rgb_u8(r, g, b))
}
