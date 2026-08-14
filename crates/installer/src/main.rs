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

use i_slint_backend_winit::WinitWindowAccessor;
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

/// Pushes a legal document (by index) into the UI and gates the Next button.
type RefreshLegal = Rc<dyn Fn(&MainWindow, usize)>;

thread_local! {
    /// The remote update manifest found by the background check (UI-thread only).
    static REMOTE_MANIFEST: RefCell<Option<bpkg_core::update::UpdateManifest>> =
        const { RefCell::new(None) };
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    let (cfg, package_path) = resolve_sources()?;

    // `--check-update`: headless update check. Prints a JSON result to stdout and exits
    // (10 = update available, 0 = up to date, 2 = error). Lets the installed app ask
    // "is there a newer version?" by spawning the installer and reading its output.
    if has("--check-update") {
        return run_check_update(&cfg);
    }

    // `--uninstall` (from the ARP entry) opens the GUI straight in maintenance mode.
    // `--update` does the same but auto-starts the update once the manifest confirms one.
    let uninstall = has("--uninstall");
    let auto_update = has("--update");
    run_gui(cfg, package_path, uninstall, auto_update)
}

/// Headless update check — see `--check-update` in `main`.
fn run_check_update(cfg: &InstallerConfig) -> anyhow::Result<()> {
    #[cfg(windows)]
    attach_parent_console();

    let plat = platform::current();
    let app_name = cfg.app.name.clone();
    // What's installed (from the OS), else the version bundled in this installer.
    let current = plat
        .installed_version(&cfg.app.id)
        .unwrap_or_else(|| cfg.app.version.clone());

    let result = match cfg.update.as_ref() {
        None => serde_json::json!({
            "app": app_name,
            "current_version": current,
            "update_available": false,
            "error": "no [update] manifest_url configured",
        }),
        Some(uc) => match bpkg_core::update::check_remote_multi(&uc.sources(), &current) {
            Ok(Some(m)) => serde_json::json!({
                "app": app_name,
                "current_version": current,
                "update_available": true,
                "latest_version": m.version,
                "notes": m.notes,
                "url": m.url,
                "has_delta": !m.deltas.is_empty(),
            }),
            Ok(None) => serde_json::json!({
                "app": app_name,
                "current_version": current,
                "update_available": false,
            }),
            Err(e) => serde_json::json!({
                "app": app_name,
                "current_version": current,
                "update_available": false,
                "error": e.to_string(),
            }),
        },
    };

    println!("{}", serde_json::to_string_pretty(&result)?);
    let code = if result["update_available"].as_bool() == Some(true) {
        10
    } else if result.get("error").is_some() {
        2
    } else {
        0
    };
    std::process::exit(code);
}

/// GUI-subsystem app: attach to the caller's console so `println!` is visible when run
/// from a terminal. (When stdout is redirected to a pipe — e.g. the app spawns us and
/// captures output — that pipe is inherited, so this is just a best-effort nicety.)
#[cfg(windows)]
fn attach_parent_console() {
    extern "system" {
        fn AttachConsole(dw_process_id: u32) -> i32;
    }
    const ATTACH_PARENT_PROCESS: u32 = 0xFFFF_FFFF;
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
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

    let mut args = std::env::args().skip(1).filter(|a| !a.starts_with("--"));
    let config_path = args
        .next()
        .unwrap_or_else(|| "examples/bmm/installer.toml".to_string());
    let package_path: Option<PathBuf> = args.next().map(PathBuf::from);
    let cfg = InstallerConfig::load(&config_path)
        .map_err(|e| anyhow::anyhow!("loading {config_path}: {e}"))?;
    Ok((cfg, package_path))
}

fn run_gui(
    cfg: InstallerConfig,
    package_path: Option<PathBuf>,
    start_uninstall: bool,
    auto_update: bool,
) -> anyhow::Result<()> {
    // Select the winit backend so the custom (frameless) title bar can drive the
    // window (drag / minimize / maximize). Must run before any Slint window.
    if let Ok(backend) = i_slint_backend_winit::Backend::new() {
        let _ = slint::platform::set_platform(Box::new(backend));
    }

    let plat = platform::current();
    let app_meta = AppMeta {
        id: cfg.app.id.clone(),
        name: cfg.app.name.clone(),
        version: cfg.app.version.clone(),
        publisher: cfg.app.publisher.clone(),
        homepage: cfg.app.homepage.clone(),
        platforms: cfg.app.platforms.clone(),
    };

    // Maintenance mode: if this app is already installed (or `--uninstall`), show
    // Repair / Uninstall instead of the install flow.
    let installed = plat.installed_dir(&app_meta.id);
    let maintenance = start_uninstall || installed.is_some();
    let install_location: PathBuf = installed
        .clone()
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|e| e.parent().map(Path::to_path_buf))
        })
        .unwrap_or_default();

    let installed_version = plat.installed_version(&app_meta.id);
    let update_available = installed_version
        .as_deref()
        .map(|iv| version_gt(&app_meta.version, iv))
        .unwrap_or(false);

    let ui = MainWindow::new()?;
    if maintenance {
        ui.set_maintenance(true);
        let loc = install_location.to_string_lossy().to_string();
        ui.set_install_location(loc.clone().into());
        ui.set_install_dir(loc.into()); // so post-action launch can resolve exes
        if let Some(iv) = &installed_version {
            ui.set_installed_version(iv.clone().into());
        }
        ui.set_update_available(update_available);
    }

    // Signature / publisher trust badge (shown on the Welcome page).
    let (signed, sig_status) = detect_signature(&cfg, package_path.as_deref());
    ui.set_signed(signed);
    ui.set_signature_status(sig_status.into());
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
    // Theme: override any palette colour from `[theme]` in installer.toml.
    {
        let pal = ui.global::<Pal>();
        let th = &cfg.theme;
        if let Some(c) = th.bg.as_deref().and_then(parse_hex) {
            pal.set_bg(c);
        }
        if let Some(c) = th.panel.as_deref().and_then(parse_hex) {
            pal.set_panel(c);
        }
        if let Some(c) = th.panel2.as_deref().and_then(parse_hex) {
            pal.set_panel2(c);
        }
        if let Some(c) = th.border.as_deref().and_then(parse_hex) {
            pal.set_border(c);
        }
        if let Some(c) = th.accent.as_deref().and_then(parse_hex) {
            pal.set_accent(c);
        }
        if let Some(c) = th.accent_dark.as_deref().and_then(parse_hex) {
            pal.set_accent_dark(c);
        }
        if let Some(c) = th.accent_hover.as_deref().and_then(parse_hex) {
            pal.set_accent_hover(c);
        }
        if let Some(c) = th.text.as_deref().and_then(parse_hex) {
            pal.set_text(c);
        }
        if let Some(c) = th.dim.as_deref().and_then(parse_hex) {
            pal.set_dim(c);
        }
        if let Some(c) = th.danger.as_deref().and_then(parse_hex) {
            pal.set_danger(c);
        }
        if let Some(c) = th.shadow.as_deref().and_then(parse_hex) {
            pal.set_shadow(c);
        }
    }
    // App branding logo: read `[branding].logo` from the package and show it in the
    // sidebar. Falls back to the BetterInstaller mark when absent.
    if let (Some(logo_rel), Some(pkg)) = (cfg.branding.logo.as_deref(), package_path.as_deref()) {
        if let Ok(mut p) = Package::open(pkg) {
            if let Ok(map) = p.read_files(&[logo_rel.to_string()]) {
                if let Some(bytes) = map.get(logo_rel) {
                    let ext = std::path::Path::new(logo_rel)
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("png");
                    let tmp =
                        std::env::temp_dir().join(format!("bi-logo-{}.{ext}", std::process::id()));
                    if std::fs::write(&tmp, bytes).is_ok() {
                        if let Ok(img) = slint::Image::load_from_path(&tmp) {
                            ui.set_app_logo(img);
                        }
                    }
                }
            }
        }
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
        ui.set_t_final_apply(t(&lang, "final_apply").into());
        ui.set_t_final_skip(t(&lang, "final_skip").into());
    }

    // Shared mutable state captured by callbacks.
    let setup_opts = Rc::new(cfg.setup_options.clone());
    let chosen: Rc<RefCell<BTreeMap<String, serde_json::Value>>> =
        Rc::new(RefCell::new(BTreeMap::new()));

    // Post-install "launch now" items (opt-in checkboxes on the Done page).
    let launch_cfg: Rc<Vec<bpkg_core::config::LaunchItem>> = Rc::new(cfg.launch.clone());
    let launch_checked: Rc<RefCell<std::collections::HashMap<String, bool>>> =
        Rc::new(RefCell::new(
            cfg.launch
                .iter()
                .map(|l| (l.id.clone(), l.default))
                .collect(),
        ));

    // Legal: a license option with `documents` becomes the Terms step (text read
    // from the package). It's hidden from the Setup page; its acceptance is stored
    // in `chosen` so the handoff still records privacy/tos acceptance.
    let legal_opt = cfg
        .setup_options
        .iter()
        .find(|o| matches!(o.kind, SetupOptionKind::License) && !o.documents.is_empty());
    let legal_opt_id: Option<String> = legal_opt.map(|o| o.id.clone());
    // Opt-in per project: forcing every installer to make people scroll would be a
    // behaviour change nobody asked for.
    ui.set_legal_require_scroll(legal_opt.map(|o| o.require_scroll).unwrap_or(false));
    ui.set_legal_scroll_hint(bpkg_core::i18n::t(&lang, "scroll_to_accept").into());
    let legal_docs: Rc<Vec<LegalDoc>> =
        Rc::new(load_legal_docs(&cfg, package_path.as_deref(), &lang));
    let legal_count = legal_docs.len();
    let legal_index: Rc<RefCell<usize>> = Rc::new(RefCell::new(0));
    // One acceptance flag PER document (separate accept for TOS and Privacy).
    let legal_accepted: Rc<RefCell<Vec<bool>>> = Rc::new(RefCell::new(vec![false; legal_count]));
    ui.set_legal_count(legal_count as i32);

    // Setup rows + the option list used for "can proceed" exclude the legal option.
    let visible_opts: Rc<Vec<SetupOption>> = Rc::new(
        cfg.setup_options
            .iter()
            .filter(|o| Some(&o.id) != legal_opt_id.as_ref())
            .cloned()
            .collect(),
    );
    let rows: Vec<OptionRow> = visible_opts.iter().map(to_row).collect();
    let model = Rc::new(VecModel::from(rows));
    ui.set_options(ModelRc::from(model.clone()));
    ui.set_can_proceed(true); // Welcome's Next is always enabled

    // Push a legal document into the UI + gate the Next button.
    let refresh_legal: RefreshLegal = Rc::new({
        let docs = legal_docs.clone();
        let acc = legal_accepted.clone();
        move |ui: &MainWindow, idx: usize| {
            if let Some(d) = docs.get(idx) {
                ui.set_legal_title(d.title.clone().into());
                ui.set_legal_blocks(ModelRc::from(Rc::new(VecModel::from(d.blocks.clone()))));
                ui.set_legal_accept_text(format!("I have read and accept the {}.", d.title).into());
            }
            let accepted = acc.borrow().get(idx).copied().unwrap_or(false);
            ui.set_legal_index(idx as i32);
            ui.set_legal_accepted(accepted);
            // Each document must be accepted before its Next is enabled.
            ui.set_can_proceed(accepted);
        }
    });

    // ── Navigation (page-aware: Welcome → Terms* → Setup → Install → Done) ──
    {
        let w = ui.as_weak();
        let visible = visible_opts.clone();
        let chosen = chosen.clone();
        let refresh = refresh_legal.clone();
        let li = legal_index.clone();
        ui.on_go_next(move || {
            let ui = match w.upgrade() {
                Some(u) => u,
                None => return,
            };
            match ui.get_page() {
                0 => {
                    if legal_count > 0 {
                        *li.borrow_mut() = 0;
                        refresh(&ui, 0);
                        ui.set_page(1);
                    } else {
                        ui.set_can_proceed(compute_can_proceed(&visible, &chosen.borrow()));
                        ui.set_page(2);
                    }
                }
                1 => {
                    let next = *li.borrow() + 1;
                    if next < legal_count {
                        *li.borrow_mut() = next;
                        refresh(&ui, next);
                    } else {
                        ui.set_can_proceed(compute_can_proceed(&visible, &chosen.borrow()));
                        ui.set_page(2);
                    }
                }
                _ => {}
            }
        });
    }
    {
        let w = ui.as_weak();
        let refresh = refresh_legal.clone();
        let li = legal_index.clone();
        ui.on_go_back(move || {
            let ui = match w.upgrade() {
                Some(u) => u,
                None => return,
            };
            match ui.get_page() {
                1 => {
                    if *li.borrow() > 0 {
                        let prev = *li.borrow() - 1;
                        *li.borrow_mut() = prev;
                        refresh(&ui, prev);
                    } else {
                        ui.set_can_proceed(true);
                        ui.set_page(0);
                    }
                }
                2 => {
                    if legal_count > 0 {
                        let last = legal_count - 1;
                        *li.borrow_mut() = last;
                        refresh(&ui, last);
                        ui.set_page(1);
                    } else {
                        ui.set_can_proceed(true);
                        ui.set_page(0);
                    }
                }
                _ => {}
            }
        });
    }
    {
        let w = ui.as_weak();
        let acc = legal_accepted.clone();
        let chosen = chosen.clone();
        let li = legal_index.clone();
        let legal_id = legal_opt_id.clone();
        ui.on_legal_accept_toggled(move |v| {
            let idx = *li.borrow();
            {
                let mut a = acc.borrow_mut();
                if idx < a.len() {
                    a[idx] = v;
                }
            }
            // The handoff records overall acceptance (all docs accepted).
            let all = acc.borrow().iter().all(|x| *x);
            if let Some(id) = &legal_id {
                chosen
                    .borrow_mut()
                    .insert(id.clone(), serde_json::json!(all));
            }
            if let Some(ui) = w.upgrade() {
                // Drive the checkbox from state (the checkbox is "controlled", so
                // it never self-toggles — this keeps TOS/Privacy independent).
                ui.set_legal_accepted(v);
                ui.set_can_proceed(v); // current doc must be accepted to proceed
            }
        });
    }
    {
        let w = ui.as_weak();
        let launch_cfg = launch_cfg.clone();
        let launch_checked = launch_checked.clone();
        ui.on_finish(move || {
            // Launch whatever the user opted into, then close.
            if let Some(ui) = w.upgrade() {
                let dir = PathBuf::from(ui.get_install_dir().to_string());
                let checked = launch_checked.borrow();
                for it in launch_cfg.iter() {
                    if *checked.get(&it.id).unwrap_or(&false) {
                        launch_detached(&dir.join(&it.exe));
                    }
                }
            }
            let _ = slint::quit_event_loop();
        });
    }
    {
        let launch_checked = launch_checked.clone();
        ui.on_launch_toggled(move |id, v| {
            launch_checked.borrow_mut().insert(id.to_string(), v);
        });
    }
    ui.on_open_url(move |url| {
        open_web_url(&url);
    });
    {
        let w = ui.as_weak();
        ui.on_browse_location(move || {
            // Native folder picker (Windows uses the OS dialog).
            if let Some(ui) = w.upgrade() {
                let start = ui.get_install_dir().to_string();
                let mut dlg = rfd::FileDialog::new().set_title("Choose install location");
                let p = std::path::Path::new(&start);
                if let Some(parent) = p.parent() {
                    if parent.exists() {
                        dlg = dlg.set_directory(parent);
                    }
                }
                if let Some(dir) = dlg.pick_folder() {
                    ui.set_install_dir(dir.to_string_lossy().to_string().into());
                }
            }
        });
    }

    // ── Custom title bar (frameless window controls + drag) ─────────────
    {
        let w = ui.as_weak();
        ui.on_start_drag(move || {
            if let Some(ui) = w.upgrade() {
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
                ui.window().with_winit_window(|win| win.set_minimized(true));
            }
        });
    }
    {
        let w = ui.as_weak();
        ui.on_toggle_maximize(move || {
            if let Some(ui) = w.upgrade() {
                ui.window()
                    .with_winit_window(|win| win.set_maximized(!win.is_maximized()));
            }
        });
    }
    ui.on_close_window(|| {
        let _ = slint::quit_event_loop();
    });

    // ── Option changes ──────────────────────────────────────────────────
    {
        let w = ui.as_weak();
        let model = model.clone();
        let chosen = chosen.clone();
        let opts = visible_opts.clone();
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

    // ── The one-time picker on the Done page ────────────────────────────────
    //
    // A `swatch` option may ask (`show_at_end`) to be offered a second time once
    // the install has succeeded — the first moment the user can judge the choice
    // with nothing else on their mind. It is never the only chance to answer: the
    // value picked during setup is already written, and skipping keeps it.
    let final_opt: Option<SetupOption> = visible_opts
        .iter()
        .find(|o| matches!(o.kind, SetupOptionKind::Swatch) && o.show_at_end)
        .cloned();
    // The handoff written at the start of the install, kept so answering the
    // picker can rewrite exactly that file rather than guess at its path again.
    let written_handoff: Rc<RefCell<Option<(PathBuf, handoff::HandoffDoc)>>> =
        Rc::new(RefCell::new(None));
    if let Some(o) = final_opt.as_ref() {
        ui.set_final_previews(ModelRc::from(Rc::new(VecModel::from(swatch_rows(o)))));
        ui.set_final_title(
            o.label
                .clone()
                .unwrap_or_else(|| humanize(&o.label_key))
                .into(),
        );
        ui.set_final_hint(o.description.clone().unwrap_or_default().into());
        ui.set_final_value(o.default.as_str().unwrap_or("").into());
    }
    {
        let w = ui.as_weak();
        ui.on_final_picked(move |v| {
            // Highlighted immediately, committed only by Apply — so clicking
            // through the tiles to look at them changes nothing.
            if let Some(ui) = w.upgrade() {
                ui.set_final_value(v);
            }
        });
    }
    {
        let w = ui.as_weak();
        let chosen = chosen.clone();
        let model = model.clone();
        let state = written_handoff.clone();
        let opt = final_opt.clone();
        ui.on_final_apply(move || {
            let ui = match w.upgrade() {
                Some(u) => u,
                None => return,
            };
            let value = ui.get_final_value().to_string();
            if let Some(o) = opt.as_ref() {
                chosen
                    .borrow_mut()
                    .insert(o.id.clone(), serde_json::json!(value.clone()));
                set_row(&model, &o.id, |r| r.string_value = value.clone().into());
                // Rewrite the handoff in place. Same prefix rule as handoff::build,
                // or the app would look for `settings.active_theme` and find
                // `active_theme` sitting next to it.
                if let Some((path, doc)) = state.borrow_mut().as_mut() {
                    for key in o.maps_to.keys() {
                        let flat = key.strip_prefix("settings.").unwrap_or(key);
                        doc.set(flat, serde_json::json!(value.clone()));
                    }
                    if doc.write_atomic(&*path).is_err() {
                        // The install itself is fine; only this last choice failed
                        // to persist, and saying so beats a silent no-op.
                        ui.set_result_message(
                            format!(
                                "{}\nCould not save that choice — the one picked during setup stands.",
                                ui.get_result_message()
                            )
                            .into(),
                        );
                    }
                }
            }
            ui.set_final_visible(false);
        });
    }
    {
        let w = ui.as_weak();
        ui.on_final_skip(move || {
            if let Some(ui) = w.upgrade() {
                ui.set_final_visible(false);
            }
        });
    }

    // ── Install ─────────────────────────────────────────────────────────
    // Pre-compute everything the install closure needs (Box<dyn PlatformOps>
    // isn't Clone, so resolve the handoff directory up front).
    let handoff_cfg = cfg.handoff.clone();
    let app_data_dir = plat.app_data_dir(&app_meta);
    // Live component selection (the user toggles optional ones on the Welcome page).
    let chosen_components: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(
        cfg.components
            .iter()
            .filter(|c| c.required || c.default)
            .map(|c| c.id.clone())
            .collect(),
    ));
    {
        let comp_rows: Vec<CompRow> = cfg
            .components
            .iter()
            .map(|c| CompRow {
                id: c.id.clone().into(),
                name: c.name.clone().into(),
                description: c.description.clone().into(),
                size: if c.size_mb > 0 {
                    format!("{} MB", c.size_mb).into()
                } else {
                    SharedString::new()
                },
                required: c.required,
                checked: c.required || c.default,
            })
            .collect();
        ui.set_components(ModelRc::from(Rc::new(VecModel::from(comp_rows))));
        let chosen = chosen_components.clone();
        ui.on_component_toggled(move |id, checked| {
            let id = id.to_string();
            let mut v = chosen.borrow_mut();
            if checked {
                if !v.contains(&id) {
                    v.push(id);
                }
            } else {
                v.retain(|x| *x != id);
            }
        });
    }
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

    // Clones for the maintenance callbacks (the install closure below moves the
    // originals).
    let pkg_maint = package_path.clone();
    let integ_maint = integ.clone();
    let comps_maint = chosen_components.clone();
    let loc_repair = install_location.clone();
    let loc_update = install_location.clone();
    let loc_uninstall = install_location.clone();
    // The version to compare/patch from (what's installed, else the bundled one).
    let current_version = installed_version
        .clone()
        .unwrap_or_else(|| app_meta.version.clone());

    // Remote update: if configured, check the manifest in the background and flip
    // the maintenance "Update" button on when a newer version is published online.
    if maintenance {
        if let Some(uc) = cfg.update.as_ref().filter(|u| u.auto_check) {
            let urls = uc.sources();
            let cur = current_version.clone();
            let weak = ui.as_weak();
            std::thread::spawn(move || {
                if let Ok(Some(m)) = bpkg_core::update::check_remote_multi(&urls, &cur) {
                    let newv = m.version.clone();
                    let _ = weak.upgrade_in_event_loop(move |ui| {
                        ui.set_update_available(true);
                        ui.set_new_version(newv.into());
                        REMOTE_MANIFEST.with(|c| *c.borrow_mut() = Some(m));
                        // `--update`: a newer version is confirmed → start it immediately.
                        if auto_update {
                            ui.invoke_update_app();
                        }
                    });
                }
            });
        }
    }

    let prog_timer: Rc<RefCell<Option<Timer>>> = Rc::new(RefCell::new(None));
    {
        let w = ui.as_weak();
        let chosen = chosen.clone();
        let chosen_components = chosen_components.clone();
        let opts = setup_opts.clone();
        let prog_timer = prog_timer.clone();
        let launch_cfg = launch_cfg.clone();
        let launch_checked = launch_checked.clone();
        let written_handoff = written_handoff.clone();
        let final_opt_id: Option<String> = final_opt.as_ref().map(|o| o.id.clone());
        // Only a fresh install offers the picker — Repair and Update do not
        // rewrite the handoff, so there would be nothing for it to change.
        let show_final = final_opt.is_some();
        ui.on_install(move || {
            let ui = match w.upgrade() {
                Some(u) => u,
                None => return,
            };

            // 1) Write the real handoff file (the headline feature).
            let mut message = String::new();
            let mut ok = true;
            if let Some(h) = handoff_cfg.as_ref().filter(|h| h.enabled) {
                // Resolve a still-"auto" select (e.g. language) to the detected OS
                // value. Without this, leaving the default "auto" means the app never
                // receives a concrete language/select choice (the "selects not applied"
                // bug). "auto" is the documented language sentinel.
                {
                    let detected = bpkg_core::i18n::detect_lang();
                    let mut ch = chosen.borrow_mut();
                    for opt in opts.iter() {
                        if matches!(opt.kind, SetupOptionKind::Select) {
                            let eff = ch
                                .get(&opt.id)
                                .and_then(|v| v.as_str())
                                .map(str::to_string)
                                .unwrap_or_else(|| {
                                    opt.default.as_str().unwrap_or_default().to_string()
                                });
                            if eff == "auto" {
                                ch.insert(opt.id.clone(), serde_json::json!(detected.clone()));
                            }
                        }
                    }
                }
                let mut doc = handoff::build(
                    &opts,
                    &chosen.borrow(),
                    chosen_components.borrow().clone(),
                    &app_version,
                    bpkg_core::VERSION,
                );
                doc.install_dir = ui.get_install_dir().to_string();
                let dir = match h.location {
                    bpkg_core::config::HandoffLocation::AppData => app_data_dir.clone(),
                    bpkg_core::config::HandoffLocation::InstallDir => {
                        PathBuf::from(ui.get_install_dir().to_string())
                    }
                };
                let path = dir.join(&h.file);
                match doc.write_atomic(&path) {
                    Ok(()) => {
                        message = format!("First-run config written to {}", path.display());
                        // Kept so the Done-page picker can amend this exact file.
                        *written_handoff.borrow_mut() = Some((path.clone(), doc.clone()));
                    }
                    Err(e) => {
                        ok = false;
                        message = format!("Could not write first-run config: {e}");
                    }
                }
            }

            // The picker opens on whatever setup already settled on, so its
            // highlighted tile matches what the app will actually start with.
            if let Some(o) = final_opt_id.as_ref() {
                if let Some(v) = chosen.borrow().get(o).and_then(|v| v.as_str()) {
                    ui.set_final_value(v.into());
                }
            }

            // 2) Copy the files.
            ui.set_page(3);
            ui.set_progress(0.0);
            ui.set_progress_label("Preparing…".into());

            match package_path.clone() {
                // Real install: verify + extract the .bpkg on a worker thread,
                // pushing progress back to the UI thread.
                Some(pkg) => {
                    let dest = PathBuf::from(ui.get_install_dir().to_string());
                    let comps = chosen_components.borrow().clone();
                    let weak = ui.as_weak();
                    let handoff_msg = message.clone();
                    let handoff_ok = ok;
                    let integ = integ.clone();
                    // Build the launch rows on the UI thread (Rc isn't Send).
                    let lrows = launch_rows(&launch_cfg, &launch_checked.borrow(), &comps);
                    std::thread::spawn(move || {
                        let result = run_real_install(weak.clone(), &pkg, &dest, &comps, &integ);
                        let _ = weak.upgrade_in_event_loop(move |ui| {
                            let name = ui.get_app_name().to_string();
                            match result {
                                Ok(n) => {
                                    ui.set_success(handoff_ok);
                                    ui.set_result_title(format!("{name} was installed").into());
                                    ui.set_result_message(
                                        format!(
                                            "Installed {n} files to {}\n{}",
                                            dest.display(),
                                            handoff_msg
                                        )
                                        .into(),
                                    );
                                    apply_launch_rows(&ui, lrows);
                                    // Only after a real success: a failed install
                                    // has nothing to pick a look for.
                                    ui.set_final_visible(show_final && handoff_ok);
                                }
                                Err(e) => {
                                    ui.set_success(false);
                                    ui.set_result_title("Installation failed".into());
                                    ui.set_result_message(format!("Install failed: {e}").into());
                                }
                            }
                            ui.set_progress(1.0);
                            ui.set_page(4);
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
                            ui.set_result_title(
                                format!("{} was installed", ui.get_app_name()).into(),
                            );
                            ui.set_result_message(final_msg.clone().into());
                            ui.set_page(4);
                            ui.set_final_visible(show_final && ok);
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

    // ── Maintenance: Repair (re-verify + restore the same version) ──────
    {
        let w = ui.as_weak();
        let pkg = pkg_maint.clone();
        let integ = integ_maint.clone();
        let comps = comps_maint.clone();
        let loc = loc_repair;
        let launch_cfg = launch_cfg.clone();
        let launch_checked = launch_checked.clone();
        ui.on_repair(move || {
            let ui = match w.upgrade() {
                Some(u) => u,
                None => return,
            };
            ui.set_maintenance_verb("Repair".into());
            let pkg = match &pkg {
                Some(p) => p.clone(),
                None => {
                    ui.set_success(false);
                    ui.set_result_title("Repair failed".into());
                    ui.set_result_message("Nothing to repair: no package embedded.".into());
                    ui.set_page(4);
                    return;
                }
            };
            let comps_v = comps.borrow().clone();
            let lrows = launch_rows(&launch_cfg, &launch_checked.borrow(), &comps_v);
            spawn_reinstall(
                &ui,
                pkg,
                loc.clone(),
                comps_v,
                integ.clone(),
                lrows,
                "Repairing",
                "Repaired",
            );
        });
    }

    // ── Maintenance: Update — remote (download + delta + rollback) when a
    //    manifest is configured & newer, else re-extract the bundled package. ──
    {
        let w = ui.as_weak();
        let pkg = pkg_maint;
        let integ = integ_maint;
        let comps = comps_maint;
        let loc = loc_update;
        let cur = current_version.clone();
        let launch_cfg = launch_cfg.clone();
        let launch_checked = launch_checked.clone();
        // `[update] allow_delta` (default true). The delta path in `download_and_apply` is
        // reached only when a current .bpkg is passed, so withholding it IS the off switch —
        // set it here, once, rather than re-reading config on the worker thread.
        let allow_delta = cfg.update.as_ref().is_none_or(|u| u.allow_delta);
        ui.on_update_app(move || {
            let ui = match w.upgrade() {
                Some(u) => u,
                None => return,
            };
            ui.set_maintenance_verb("Update".into());
            let comps_v = comps.borrow().clone();

            // Preferred path: a configured remote update was found.
            if let Some(m) = REMOTE_MANIFEST.with(|c| c.borrow().clone()) {
                let lrows = launch_rows(&launch_cfg, &launch_checked.borrow(), &comps_v);
                ui.set_page(3);
                ui.set_progress(0.2);
                ui.set_progress_label(format!("Downloading v{}…", m.version).into());
                let dir = loc.clone();
                let cur = cur.clone();
                let cur_bpkg = if allow_delta { pkg.clone() } else { None };
                let weak = ui.as_weak();
                // Pin the publisher key so a tampered/unsigned update from a hostile
                // mirror is refused before it's applied (fail closed).
                let update_vk = integ
                    .public_key
                    .as_ref()
                    .and_then(|pk| bpkg_core::sign::parse_public(pk).ok());
                std::thread::spawn(move || {
                    let res = bpkg_core::update::download_and_apply(
                        &m,
                        &cur,
                        cur_bpkg.as_deref(),
                        &dir,
                        update_vk.as_ref(),
                    );
                    let _ = weak.upgrade_in_event_loop(move |ui| {
                        match res {
                            Ok(n) => {
                                ui.set_success(true);
                                ui.set_result_title(
                                    format!("{} was updated to v{}", ui.get_app_name(), m.version)
                                        .into(),
                                );
                                ui.set_result_message(
                                    format!(
                                        "Updated to v{}: {n} files in {}",
                                        m.version,
                                        dir.display()
                                    )
                                    .into(),
                                );
                                apply_launch_rows(&ui, lrows);
                            }
                            Err(e) => {
                                ui.set_success(false);
                                ui.set_result_title("Update failed".into());
                                ui.set_result_message(format!("Update failed: {e}").into());
                            }
                        }
                        ui.set_progress(1.0);
                        ui.set_page(4);
                    });
                });
                return;
            }

            // Fallback: re-extract the (newer) bundled package.
            let pkg = match &pkg {
                Some(p) => p.clone(),
                None => {
                    ui.set_success(false);
                    ui.set_result_title("Update failed".into());
                    ui.set_result_message("No update package available.".into());
                    ui.set_page(4);
                    return;
                }
            };
            let lrows = launch_rows(&launch_cfg, &launch_checked.borrow(), &comps_v);
            spawn_reinstall(
                &ui,
                pkg,
                loc.clone(),
                comps_v,
                integ.clone(),
                lrows,
                "Updating",
                "Updated",
            );
        });
    }

    // ── Maintenance: Uninstall ──────────────────────────────────────────
    {
        let w = ui.as_weak();
        let loc = loc_uninstall;
        ui.on_uninstall_app(move || {
            let ui = match w.upgrade() {
                Some(u) => u,
                None => return,
            };
            ui.set_maintenance_verb("Uninstall".into());
            ui.set_page(3);
            ui.set_progress(0.4);
            ui.set_progress_label("Uninstalling…".into());
            let dir = loc.clone();
            let name = ui.get_app_name().to_string();
            let weak = ui.as_weak();
            std::thread::spawn(move || {
                let result = do_uninstall_full(&dir);
                let _ = weak.upgrade_in_event_loop(move |ui| {
                    match result {
                        Ok(()) => {
                            ui.set_success(true);
                            ui.set_result_title(format!("{name} was uninstalled").into());
                            ui.set_result_message(
                                format!("{name} and its files were removed.").into(),
                            );
                        }
                        Err(e) => {
                            ui.set_success(false);
                            ui.set_result_title("Uninstall failed".into());
                            ui.set_result_message(format!("Uninstall failed: {e}").into());
                        }
                    }
                    ui.set_progress(1.0);
                    ui.set_page(4);
                });
            });
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
    // Prerequisites: auto-download/-install the missing required ones (those with a
    // download_url), error on any still missing. Done before touching the install.
    {
        let weak = weak.clone();
        // `dest` so a zip prerequisite (a downloaded runtime) unpacks under the install
        // directory the user chose, not somewhere fixed.
        bpkg_core::prereq::ensure_required(&integ.prereqs, dest, |name| {
            let name = name.to_string();
            let _ = weak.upgrade_in_event_loop(move |ui| {
                ui.set_progress_label(format!("Installing prerequisite: {name}…").into());
            });
        })
        .map_err(|e| e.to_string())?;
    }

    // If a previous version is running, close it first — otherwise its locked .exe /
    // resources make the overwrite (install / repair / update) fail. No-op on a fresh
    // install where the dir doesn't exist yet.
    if dest.exists() {
        let _ = weak.upgrade_in_event_loop(|ui| {
            ui.set_progress_label("Closing the running app…".into());
        });
        kill_running_apps(dest);
        std::thread::sleep(std::time::Duration::from_millis(400));
    }

    // Writability preflight — fail with a clear message instead of a cryptic I/O
    // error if the chosen folder needs administrator rights (e.g. Program Files).
    if let Err(e) = std::fs::create_dir_all(dest) {
        return Err(format!(
            "Can't write to {}: {e}\nPick a folder you can write to (the default is under your user profile). Installing into Program Files needs an administrator (elevated) installer.",
            dest.display()
        ));
    }

    let mut p = Package::open(pkg).map_err(|e| e.to_string())?;

    // Verify the Ed25519 signature before writing anything, when a trust key is set.
    if let Some(pk_hex) = integ.public_key.as_ref() {
        let vk = bpkg_core::sign::parse_public(pk_hex).map_err(|e| e.to_string())?;
        let valid = p.verify_signature(&vk).map_err(|e| e.to_string())?;
        signature_verdict(valid, p.is_signed(), integ.require_signature)?;
    }

    let comp: Option<&[String]> = if comps.is_empty() { None } else { Some(comps) };
    let mut last_pct = -1i32;
    let written = p
        .install_with_progress(dest, comp, |done, total, file| {
            let pct = (done * 100)
                .checked_div(total)
                .map(|p| p as i32)
                .unwrap_or(100);
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
/// Reverse the system integration recorded in `dir/uninstall-info.json`, then
/// remove the install directory. If we're running from *inside* `dir` (the ARP
/// uninstaller, which Windows locks), keep the running exe and schedule a detached
/// self-delete; otherwise remove everything immediately.
fn do_uninstall_full(dir: &Path) -> Result<(), String> {
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

    // Close the app if it's running, so its files aren't locked and the uninstall
    // doesn't get blocked.
    kill_running_apps(dir);

    let exe = std::env::current_exe().unwrap_or_default();
    if exe.starts_with(dir) {
        // Locked uninstaller: remove all but the running exe, then schedule a
        // detached self-delete that also removes the uninstaller + the folder.
        remove_dir_except(dir, &exe);
        schedule_self_delete(&exe, dir);
        Ok(())
    } else {
        // Give the killed processes a moment to release their file handles.
        std::thread::sleep(std::time::Duration::from_millis(400));
        std::fs::remove_dir_all(dir).map_err(|e| e.to_string())
    }
}

/// Force-close any app executable living in the install dir (e.g.
/// better-mods-manager.exe, bmm-mcp-server.exe) before removing files. Never
/// touches the running uninstaller itself.
#[cfg(windows)]
fn kill_running_apps(dir: &Path) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let self_exe = std::env::current_exe().unwrap_or_default();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p == self_exe {
                continue;
            }
            let is_exe = p
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("exe"))
                .unwrap_or(false);
            if is_exe {
                if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                    let _ = std::process::Command::new("taskkill")
                        .args(["/F", "/IM", name])
                        .creation_flags(CREATE_NO_WINDOW)
                        .output();
                }
            }
        }
    }
}

#[cfg(not(windows))]
fn kill_running_apps(_dir: &Path) {}

#[cfg(windows)]
fn schedule_self_delete(exe: &Path, dir: &Path) {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // Wait ~1s (let us exit + release the lock), delete the exe, then force-remove
    // the whole folder (incl. the uninstaller + anything left behind).
    let script = format!(
        "ping 127.0.0.1 -n 2 >nul & del /F /Q \"{}\" & rmdir /S /Q \"{}\"",
        exe.display(),
        dir.display()
    );
    let _ = std::process::Command::new("cmd")
        .args(["/C", &script])
        .creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW)
        .spawn();
}

#[cfg(not(windows))]
fn schedule_self_delete(exe: &Path, _dir: &Path) {
    // Unix doesn't lock running executables — just remove it.
    let _ = std::fs::remove_file(exe);
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

/// Whether package signature `status` should show the package as trusted, and the
/// human label for the Welcome-page badge.
fn detect_signature(cfg: &InstallerConfig, pkg: Option<&Path>) -> (bool, String) {
    let publisher = cfg.app.publisher.as_str();
    let pkg = match pkg {
        Some(p) => p,
        None => return (false, "Unsigned (developer preview)".into()),
    };
    let mut p = match Package::open(pkg) {
        Ok(p) => p,
        Err(_) => return (false, "Package unreadable".into()),
    };
    if !p.is_signed() {
        return (
            false,
            format!("Unsigned package  ·  publisher: {publisher}"),
        );
    }
    if let Some(pk) = cfg.security.as_ref().and_then(|s| s.public_key.as_ref()) {
        match bpkg_core::sign::parse_public(pk).and_then(|vk| p.verify_signature(&vk)) {
            Ok(true) => return (true, format!("Signed & verified  ·  {publisher}")),
            _ => {
                return (
                    false,
                    "Signature INVALID — do not trust this package".into(),
                )
            }
        }
    }
    (true, format!("Signed  ·  {publisher}"))
}

/// Decide whether a package may install, given the outcome of verifying it against the
/// publisher's pinned key.
///
/// A BAD signature and a MISSING one are different failures, and only one of them is
/// `require_signature`'s business.
///
/// `require_signature` answers "may an UNSIGNED package install?" — a deployment choice,
/// and it defaults to false. A signature that is present but does not verify is not a
/// policy question: the bytes were altered after the publisher signed them, or they came
/// from someone else entirely. That is refused unconditionally.
///
/// Conflating the two (`if !valid && require_signature`) meant the DEFAULT configuration —
/// trust key set, `require_signature` unset — installed a tampered package without a word,
/// which is the exact attack that pinning the key was meant to stop.
fn signature_verdict(valid: bool, is_signed: bool, require_signature: bool) -> Result<(), String> {
    if valid {
        return Ok(());
    }
    if is_signed {
        return Err("package signature is INVALID — refusing to install".to_string());
    }
    if require_signature {
        return Err("package is not signed but a signature is required".to_string());
    }
    Ok(())
}

/// Compare dotted version strings numerically: is `a` newer than `b`?
fn version_gt(a: &str, b: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.split(['.', '-', '+'])
            .map(|p| p.parse::<u64>().unwrap_or(0))
            .collect()
    };
    let (va, vb) = (parse(a), parse(b));
    for i in 0..va.len().max(vb.len()) {
        let x = va.get(i).copied().unwrap_or(0);
        let y = vb.get(i).copied().unwrap_or(0);
        if x != y {
            return x > y;
        }
    }
    false
}

/// Spawn `exe` detached (no console window, survives the installer closing).
fn launch_detached(exe: &Path) {
    if !exe.exists() {
        return;
    }
    let dir = exe.parent().unwrap_or_else(|| Path::new("."));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        let _ = std::process::Command::new(exe)
            .current_dir(dir)
            .creation_flags(DETACHED_PROCESS)
            .spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new(exe).current_dir(dir).spawn();
    }
}

/// Filter the configured launch items to those whose component was installed, and
/// resolve each one's checked state. Returns plain (id, label, checked) — `Send`,
/// so it can cross into the worker thread.
fn launch_rows(
    cfg: &[bpkg_core::config::LaunchItem],
    checked: &std::collections::HashMap<String, bool>,
    installed: &[String],
) -> Vec<(String, String, bool)> {
    cfg.iter()
        .filter(|l| {
            l.component
                .as_ref()
                .map(|c| installed.iter().any(|ic| ic == c))
                .unwrap_or(true)
        })
        .map(|l| {
            (
                l.id.clone(),
                l.label.clone(),
                *checked.get(&l.id).unwrap_or(&l.default),
            )
        })
        .collect()
}

/// Push pre-computed launch rows into the Done-page model.
fn apply_launch_rows(ui: &MainWindow, rows: Vec<(String, String, bool)>) {
    let model: Vec<LaunchRow> = rows
        .into_iter()
        .map(|(id, label, checked)| LaunchRow {
            id: id.into(),
            label: label.into(),
            checked,
        })
        .collect();
    ui.set_launch_items(ModelRc::from(Rc::new(VecModel::from(model))));
}

/// Re-extract the package into `dest` on a worker thread (Repair / Update), then
/// surface the result + launch options on the Done page.
#[allow(clippy::too_many_arguments)]
fn spawn_reinstall(
    ui: &MainWindow,
    pkg: PathBuf,
    dest: PathBuf,
    comps: Vec<String>,
    integ: SystemIntegration,
    lrows: Vec<(String, String, bool)>,
    progress_verb: &'static str,
    done_verb: &'static str,
) {
    ui.set_page(3);
    ui.set_progress(0.0);
    ui.set_progress_label(format!("{progress_verb}…").into());
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let result = run_real_install(weak.clone(), &pkg, &dest, &comps, &integ);
        let _ = weak.upgrade_in_event_loop(move |ui| {
            let name = ui.get_app_name().to_string();
            match result {
                Ok(n) => {
                    ui.set_success(true);
                    ui.set_result_title(format!("{name} was {}", done_verb.to_lowercase()).into());
                    ui.set_result_message(
                        format!("{done_verb}: {n} files in {}", dest.display()).into(),
                    );
                    apply_launch_rows(&ui, lrows);
                }
                Err(e) => {
                    ui.set_success(false);
                    ui.set_result_title(format!("{done_verb} failed").into());
                    ui.set_result_message(format!("{done_verb} failed: {e}").into());
                }
            }
            ui.set_progress(1.0);
            ui.set_page(4);
        });
    });
}

/// One legal document for the Terms step (title + rendered markdown blocks).
struct LegalDoc {
    title: String,
    blocks: Vec<MdBlock>,
}

/// Read the license documents from the package and render them to markdown blocks.
/// Terms and a privacy policy shown in a language the reader may not have, in an
/// installer that is otherwise translated, is a consent problem before it is a
/// polish one — someone clicks "I accept" on text they cannot read.
///
/// The config names the documents once (`TOS.md`, `PRIVACY.md`) and this looks for a
/// `_<lang>` sibling of each inside the package: `TOS_FR.md` for `fr`. Found, it is
/// used; absent, the original is — so a package that ships only English behaves
/// exactly as before and no config has to change to gain this.
fn localized_doc_name(doc: &str, lang: &str) -> Option<String> {
    let l = lang.split(['-', '_']).next().unwrap_or(lang).to_uppercase();
    if l.is_empty() || l == "EN" {
        return None;
    }
    let (stem, ext) = match doc.rsplit_once('.') {
        Some((s, e)) => (s, e),
        None => return None,
    };
    Some(format!("{stem}_{l}.{ext}"))
}

fn load_legal_docs(cfg: &InstallerConfig, pkg: Option<&Path>, lang: &str) -> Vec<LegalDoc> {
    let mut out = Vec::new();
    let lo = match cfg
        .setup_options
        .iter()
        .find(|o| matches!(o.kind, SetupOptionKind::License) && !o.documents.is_empty())
    {
        Some(o) => o,
        None => return out,
    };
    let pkg = match pkg {
        Some(p) => p,
        None => return out,
    };
    let mut p = match Package::open(pkg) {
        Ok(p) => p,
        Err(_) => return out,
    };
    // Ask for both spellings in one read, then prefer the localized one per document.
    // Requesting them together keeps this to a single pass over the archive.
    let mut wanted: Vec<String> = Vec::new();
    for doc in &lo.documents {
        if let Some(loc) = localized_doc_name(doc, lang) {
            wanted.push(loc);
        }
        wanted.push(doc.clone());
    }
    let map = match p.read_files(&wanted) {
        Ok(m) => m,
        Err(_) => return out,
    };
    for doc in &lo.documents {
        let picked = localized_doc_name(doc, lang)
            .and_then(|loc| map.get(&loc).map(|b| (loc, b)))
            .or_else(|| map.get(doc).map(|b| (doc.clone(), b)));
        if let Some((name, bytes)) = picked {
            out.push(LegalDoc {
                title: doc_title(&name, lang),
                blocks: parse_md(&String::from_utf8_lossy(bytes)),
            });
        }
    }
    out
}

/// Render markdown to display blocks: headings (level 1-3), bullets (level 4),
/// paragraphs (level 0). Inline emphasis/code markers are stripped; links become
/// `text (url)`.
fn parse_md(text: &str) -> Vec<MdBlock> {
    let mut out = Vec::new();
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    // Header cells of the table currently being read, if any. A table ends at the first
    // line that is not a row.
    let mut headers: Vec<String> = Vec::new();
    let mut i = 0usize;

    while i < lines.len() {
        let line = lines[i];

        // -- Tables --------------------------------------------------------------
        // There is no table layout in the Slint view, and the renderer used to fall
        // through to the paragraph arm: PRIVACY's "what leaves your PC" summary -- the
        // one table a privacy policy most needs read -- reached the user as raw
        // pipe-delimited lines. Rather than build a grid, each row is flattened into one
        // bullet carrying its own column labels, so it stays readable at any width.
        if is_table_row(line) {
            if headers.is_empty() {
                headers = split_row(line);
                i += 1;
                if i < lines.len() && is_table_divider(lines[i]) {
                    i += 1;
                }
                continue;
            }
            if is_table_divider(line) {
                i += 1;
                continue;
            }
            let cells = split_row(line);
            if let Some(t) = flatten_row(&headers, &cells) {
                out.push(MdBlock {
                    text: t.into(),
                    level: 4,
                    link: String::new().into(),
                });
            }
            i += 1;
            continue;
        }
        headers.clear();

        let (level, content): (i32, &str) = if let Some(s) = line.strip_prefix("### ") {
            (3, s)
        } else if let Some(s) = line.strip_prefix("## ") {
            (2, s)
        } else if let Some(s) = line.strip_prefix("# ") {
            (1, s)
        } else if let Some(s) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            (4, s)
        } else {
            (0, line)
        };
        let (text, links) = inline_md(content);
        out.push(MdBlock {
            text: text.into(),
            level,
            link: String::new().into(),
        });
        // Each link becomes its own row beneath the prose: level 5, carrying the URL both
        // as its text and as its target. The sentence above reads as a sentence, and the
        // address is on screen exactly once — clickable, and still copyable by eye for
        // anyone who does not trust an installer to open their browser.
        for url in links {
            out.push(MdBlock {
                text: url.clone().into(),
                level: 5,
                link: url.into(),
            });
        }
        i += 1;
    }
    out
}

fn is_table_row(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('|') && t.len() > 1
}

/// A `|---|:--:|` separator carries no content and must not become a bullet.
fn is_table_divider(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('|') && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ')) && t.contains('-')
}

fn split_row(line: &str) -> Vec<String> {
    line.trim()
        .trim_start_matches('|')
        .trim_end_matches('|')
        .split('|')
        .map(|c| strip_inline_md(c.trim()))
        .collect()
}

/// One table row -> "first cell - Header: value - Header: value".
///
/// Cells holding an em dash or nothing are dropped: in these documents that is how
/// "nothing is sent" is written, and repeating "Data sent: -" on every such row buries
/// the rows that DO say something.
fn flatten_row(headers: &[String], cells: &[String]) -> Option<String> {
    let subject = cells.first()?.trim();
    if subject.is_empty() {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    for (idx, cell) in cells.iter().enumerate().skip(1) {
        let v = cell.trim();
        if v.is_empty() || v == "\u{2014}" || v == "-" {
            continue;
        }
        match headers.get(idx).map(|h| h.trim()).filter(|h| !h.is_empty()) {
            Some(h) => parts.push(format!("{h}: {v}")),
            None => parts.push(v.to_string()),
        }
    }
    if parts.is_empty() {
        Some(subject.to_string())
    } else {
        Some(format!("{subject} \u{2014} {}", parts.join(" \u{b7} ")))
    }
}

/// Raw HTML tags a legal document may carry. Markdown allows inline HTML and these
/// documents are rendered by three different things — BMM (marked, which passes HTML
/// through), the BCWEB site, and this installer, which draws Text runs and has no notion
/// of a tag at all. An `<a href=…>` at the end of PRIVACY.md therefore reached the reader
/// as literal angle brackets.
///
/// The document was rewritten to use a markdown link, but stripping tags here is what
/// stops the NEXT one: nobody editing a policy is thinking about a Slint renderer, and a
/// tag that leaks through is visible to every user of the installer.
///
/// Deliberately crude — it removes tags and keeps their inner text, which is the right
/// outcome for `<a>`, `<b>`, `<span>` and friends. It is not an HTML parser and does not
/// need to be: the goal is that markup never renders as prose, not that HTML is supported.
fn strip_html_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0usize;
    for ch in s.chars() {
        match ch {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            c if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

fn strip_inline_md(s: &str) -> String {
    inline_md(s).0
}

/// Inline markdown -> (prose, links found in reading order).
///
/// `[text](url)` used to flatten to `text (url)`, which is why a policy's one useful
/// line read as `BetterModsManager (https://github.com/FreeProject089/BetterModsManager)`
/// — a URL printed mid-sentence that the reader then had to retype into a browser,
/// because a `Text` run is not clickable.
///
/// The prose now keeps only the LABEL, and the URL comes back separately so the caller
/// can render it as something you can actually click. A bare `https://…` sitting in the
/// text counts as a link too: policies write them both ways, and the reader does not care
/// which syntax the author used.
fn inline_md(s: &str) -> (String, Vec<String>) {
    let mut r = strip_html_tags(s).replace("**", "").replace('`', "");
    let mut links: Vec<String> = Vec::new();
    while let (Some(lb), Some(rb)) = (r.find('['), r.find("](")) {
        if rb <= lb {
            break;
        }
        let close = match r[rb..].find(')') {
            Some(c) => rb + c,
            None => break,
        };
        let txt = r[lb + 1..rb].to_string();
        let url = r[rb + 2..close].trim().to_string();
        if is_web_url(&url) {
            links.push(url);
        }
        // The label alone. An empty label (`[](url)`) would leave a hole in the sentence,
        // so it falls back to the URL — visible, if ugly, beats invisible.
        let repl = if txt.trim().is_empty() {
            r[rb + 2..close].to_string()
        } else {
            txt
        };
        r.replace_range(lb..close + 1, &repl);
    }
    let text = r
        .replace('*', "")
        .trim_start_matches('>')
        .trim()
        .to_string();
    // Bare URLs, after the markdown pass so a link's own URL is not collected twice.
    for tok in
        text.split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == '<' || c == '>')
    {
        // Trailing sentence punctuation is not part of the address.
        let tok = tok.trim_end_matches(|c| matches!(c, '.' | ',' | ';' | ':' | '!' | '?' | '"'));
        if is_web_url(tok) && !links.iter().any(|l| l == tok) {
            links.push(tok.to_string());
        }
    }
    (text, links)
}

/// Only `http(s)` is ever handed to the OS opener.
///
/// The documents are bundled, not fetched, so this is not a defence against a hostile
/// policy file — it is a defence against handing the shell something that is not a web
/// page at all. `file:`, `javascript:` and bare paths open *something* on Windows, and an
/// installer must not be the thing that launches it.
fn is_web_url(u: &str) -> bool {
    let u = u.trim();
    (u.starts_with("https://") || u.starts_with("http://"))
        && u.len() > 8
        && !u.contains(char::is_whitespace)
}

/// Hand a web address to the OS browser.
///
/// Re-checks the scheme even though the parser only ever produces `http(s)` links: this is
/// the function that reaches the shell, and a guard that lives at the call site is a guard
/// that the next call site forgets.
///
/// Windows goes through `explorer.exe`, which is a GUI process — `cmd /c start` would flash
/// a console window over the installer. Failure is silent by design: a browser that will
/// not open is not a reason to interrupt an install, and the URL is on screen to be typed.
fn open_web_url(url: &str) {
    if !is_web_url(url) {
        return;
    }
    let _ = {
        #[cfg(target_os = "windows")]
        {
            std::process::Command::new("explorer.exe").arg(url).spawn()
        }
        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("open").arg(url).spawn()
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            std::process::Command::new("xdg-open").arg(url).spawn()
        }
    };
}

/// Friendly title for a legal document filename.
fn doc_title(file: &str, lang: &str) -> String {
    let low = file.to_lowercase();
    if low.contains("privacy") {
        bpkg_core::i18n::t(lang, "doc_privacy")
    } else if low.contains("tos") || low.contains("terms") || low.contains("eula") {
        bpkg_core::i18n::t(lang, "doc_tos")
    } else {
        // Fallback for any other bundled document: show the bare file name, minus the
        // language suffix. Without the trim a French reader gets "CONTRIBUTING_FR" as a
        // heading -- the localized spelling is an implementation detail of how the file
        // was picked, not something to put on screen.
        let stem = file
            .rsplit('/')
            .next()
            .unwrap_or(file)
            .trim_end_matches(".md");
        stem.strip_suffix("_FR")
            .or_else(|| stem.strip_suffix("_EN"))
            .unwrap_or(stem)
            .to_string()
    }
}

/// Map a [`SetupOption`] to the Slint row struct.
fn to_row(o: &SetupOption) -> OptionRow {
    let kind = match o.kind {
        SetupOptionKind::Bool => "bool",
        SetupOptionKind::Select => "select",
        SetupOptionKind::License => "license",
        SetupOptionKind::Swatch => "swatch",
    };
    let choices: Vec<SharedString> = o.choices.iter().map(|c| c.clone().into()).collect();
    let label = o.label.clone().unwrap_or_else(|| humanize(&o.label_key));
    OptionRow {
        id: o.id.clone().into(),
        kind: kind.into(),
        label: label.into(),
        description: o.description.clone().unwrap_or_default().into(),
        choices: ModelRc::from(Rc::new(VecModel::from(choices))),
        previews: ModelRc::from(Rc::new(VecModel::from(swatch_rows(o)))),
        bool_value: o.default.as_bool().unwrap_or(false),
        string_value: o.default.as_str().unwrap_or("").into(),
    }
}

/// Build the tile models for a `swatch` option.
fn swatch_rows(o: &SetupOption) -> Vec<SwatchRow> {
    o.previews
        .iter()
        .map(|p| {
            // Falls back to the installer's own palette, so a preview declared with
            // two colors renders as a dull tile instead of an invisible one.
            let at = |i: usize, fallback: Color| {
                p.colors
                    .get(i)
                    .and_then(|c| parse_hex(c))
                    .unwrap_or(fallback)
            };
            SwatchRow {
                value: p.value.clone().into(),
                label: p.label.clone().unwrap_or_else(|| p.value.clone()).into(),
                bg: at(0, Color::from_rgb_u8(0x0d, 0x11, 0x17)),
                surface: at(1, Color::from_rgb_u8(0x16, 0x1b, 0x22)),
                accent: at(2, Color::from_rgb_u8(0x3b, 0x82, 0xf6)),
                ink: at(3, Color::from_rgb_u8(0xe6, 0xed, 0xf3)),
            }
        })
        .collect()
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
    match s.len() {
        6 => {
            let r = u8::from_str_radix(&s[0..2], 16).ok()?;
            let g = u8::from_str_radix(&s[2..4], 16).ok()?;
            let b = u8::from_str_radix(&s[4..6], 16).ok()?;
            Some(Color::from_rgb_u8(r, g, b))
        }
        // #abc is #aabbcc.
        3 => {
            let n = |i: usize| u8::from_str_radix(&s[i..i + 1], 16).ok().map(|v| v * 17);
            Some(Color::from_rgb_u8(n(0)?, n(1)?, n(2)?))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{doc_title, is_web_url, parse_md, signature_verdict};

    // The exact shape of PRIVACY.md's "what leaves your PC" summary. Before table
    // support this reached the user as raw pipe-delimited lines.
    const TABLE: &str = "\
## Summary
| Action | Leaves your PC? | Data sent | Recipient |
|---|---|---|---|
| Browsing/managing mods | No | \u{2014} | \u{2014} |
| **Telemetry ON** | Yes | Anonymous usage | BMM dashboard |

After the table.";

    #[test]
    fn raw_html_never_reaches_the_reader_as_text() {
        // The exact three-line anchor that used to sit at the end of PRIVACY.md, plus the
        // markdown link that replaced it — both must read as prose.
        let blocks = parse_md(
            "Questions? Open an issue:\n\
             <a href=\"https://example.com/repo\" target=\"_blank\" rel=\"noopener noreferrer\">\n\
             BetterModsManager\n\
             </a>\n\
             \n\
             Or use [BetterModsManager](https://example.com/repo).",
        );
        let text: Vec<String> = blocks.iter().map(|b| b.text.to_string()).collect();
        let joined = text.join("\n");

        // No angle brackets, no attribute names, nowhere.
        assert!(!joined.contains('<') && !joined.contains('>'), "{joined}");
        assert!(!joined.contains("href"), "{joined}");
        assert!(!joined.contains("noopener"), "{joined}");

        // The link TEXT survives — stripping a tag must not delete what it wrapped.
        assert!(
            text.iter().any(|l| l.contains("BetterModsManager")),
            "{text:?}"
        );
        // And a real markdown link still puts its address on screen.
        assert!(joined.contains("https://example.com/repo"), "{joined}");
    }

    #[test]
    fn a_link_becomes_a_clickable_row_and_leaves_the_sentence_alone() {
        let blocks =
            parse_md("Questions? Open an issue on [BetterModsManager](https://example.com/repo).");

        // The prose keeps the LABEL and drops the address: the old renderer inlined
        // "text (url)", so a policy's one useful line arrived with a URL wedged into the
        // middle of it and no way to follow it.
        let prose = &blocks[0];
        assert_eq!(prose.level, 0);
        assert!(prose.text.contains("BetterModsManager"), "{}", prose.text);
        assert!(!prose.text.contains("https://"), "{}", prose.text);
        assert_eq!(prose.link.to_string(), "");

        // The address follows as its own row, carrying the URL as a target.
        let link = &blocks[1];
        assert_eq!(link.level, 5);
        assert_eq!(link.link.to_string(), "https://example.com/repo");
        assert_eq!(link.text.to_string(), "https://example.com/repo");
    }

    #[test]
    fn a_bare_url_is_clickable_too_and_is_never_collected_twice() {
        // Policies write links both ways; the reader does not care which.
        let blocks = parse_md("Write to https://example.com/contact for anything else.");
        let links: Vec<String> = blocks
            .iter()
            .filter(|b| b.level == 5)
            .map(|b| b.link.to_string())
            .collect();
        assert_eq!(links, vec!["https://example.com/contact".to_string()]);

        // A markdown link's own URL must not ALSO be picked up by the bare-URL scan —
        // it is not in the prose to be found, and a doubled row would look like a bug.
        let blocks = parse_md("See [the policy](https://example.com/p).");
        assert_eq!(blocks.iter().filter(|b| b.level == 5).count(), 1);
    }

    #[test]
    fn only_web_urls_are_ever_handed_to_the_shell() {
        // This is the guard on what open_web_url will launch. `file:` and `javascript:`
        // open *something* on Windows, and an installer must not be what launches it.
        assert!(is_web_url("https://example.com"));
        assert!(is_web_url("http://example.com"));
        assert!(!is_web_url("file:///C:/Windows/System32/cmd.exe"));
        assert!(!is_web_url("javascript:alert(1)"));
        assert!(!is_web_url(r"C:\Windows\System32\cmd.exe"));
        assert!(!is_web_url("https://"));
        assert!(!is_web_url("https://exa mple.com"));

        // And a non-web target never becomes a clickable row in the first place.
        let blocks = parse_md("[Open](file:///C:/Windows/System32/cmd.exe)");
        assert_eq!(blocks.iter().filter(|b| b.level == 5).count(), 0);
    }

    #[test]
    fn a_markdown_table_becomes_readable_bullets() {
        let blocks = parse_md(TABLE);
        let texts: Vec<String> = blocks.iter().map(|b| b.text.to_string()).collect();

        // No pipes survive anywhere: that was the whole defect.
        assert!(
            !texts.iter().any(|t| t.contains('|')),
            "a raw table line reached the view: {texts:?}"
        );
        // The header row is consumed, not printed as a bullet of its own.
        assert!(!texts.iter().any(|t| t.starts_with("Action")));
        // The separator never becomes a block.
        assert!(!texts.iter().any(|t| t.contains("---")));

        // A row whose only real cell is the subject keeps just the subject: the em dashes
        // mean "nothing is sent", and echoing "Data sent: -" would bury the rows that
        // actually say something.
        assert!(
            texts
                .iter()
                .any(|t| t == "Browsing/managing mods \u{2014} Leaves your PC?: No"),
            "{texts:?}"
        );
        // A row with real values keeps every one of them, each with its column label.
        let telemetry = texts
            .iter()
            .find(|t| t.starts_with("Telemetry ON"))
            .expect("the telemetry row survives");
        assert!(telemetry.contains("Leaves your PC?: Yes"), "{telemetry}");
        assert!(
            telemetry.contains("Data sent: Anonymous usage"),
            "{telemetry}"
        );
        assert!(
            telemetry.contains("Recipient: BMM dashboard"),
            "{telemetry}"
        );

        // Rows are bullets, and the surrounding document is untouched.
        assert_eq!(
            blocks
                .iter()
                .find(|b| b.text.starts_with("Telemetry ON"))
                .unwrap()
                .level,
            4
        );
        assert!(blocks.iter().any(|b| b.level == 2 && b.text == "Summary"));
        assert!(texts.iter().any(|t| t == "After the table."));
    }

    #[test]
    fn legal_document_titles_follow_the_language() {
        assert_eq!(doc_title("TOS_FR.md", "fr"), "Conditions d'utilisation");
        assert_eq!(doc_title("TOS.md", "en"), "Terms of Service");
        assert_eq!(
            doc_title("PRIVACY_FR.md", "fr"),
            "Politique de confidentialit\u{e9}"
        );
        assert_eq!(doc_title("PRIVACY.md", "en"), "Privacy Policy");
        // Any other bundled document falls back to its name without the language suffix.
        assert_eq!(doc_title("CONTRIBUTING_FR.md", "fr"), "CONTRIBUTING");
    }

    #[test]
    fn a_tampered_package_is_refused_even_when_signatures_are_optional() {
        // The regression this guards: signed, does not verify, require_signature = false
        // (the DEFAULT). This must fail closed — it used to install silently.
        let err = signature_verdict(false, true, false)
            .expect_err("a package with a broken signature must never install");
        assert!(err.contains("INVALID"), "unexpected message: {err}");
        // …and it stays refused when signatures are mandatory, for the same reason.
        assert!(signature_verdict(false, true, true).is_err());
    }

    #[test]
    fn an_unsigned_package_is_a_policy_question() {
        // Not signed at all: this one IS require_signature's call.
        assert!(
            signature_verdict(false, false, false).is_ok(),
            "opting out of signatures must still allow an unsigned package"
        );
        let err = signature_verdict(false, false, true).expect_err("required means required");
        assert!(err.contains("not signed"), "unexpected message: {err}");
    }

    #[test]
    fn a_good_signature_always_installs() {
        for require in [false, true] {
            assert!(signature_verdict(true, true, require).is_ok());
        }
    }
}
