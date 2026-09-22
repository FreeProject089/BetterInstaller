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

use bpkg_core::config::{InstallerConfig, SetupGroup, SetupOption, SetupOptionKind};
use bpkg_core::handoff;
use bpkg_core::i18n::{Catalog, Direction, Translator};
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
    /// The UI thread's translator, for closures that come back from a worker thread
    /// through `upgrade_in_event_loop` (they must be Send, and an Rc is not).
    static UI_TRANSLATOR: RefCell<Option<Rc<RefCell<Translator>>>> =
        const { RefCell::new(None) };
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    // `--lang=<tag>` opens in that language, whatever the OS says. For testing a
    // translation, and for a support person reproducing what a user sees.
    let lang_arg = args
        .iter()
        .find_map(|a| a.strip_prefix("--lang="))
        .map(str::to_string);
    let (cfg, package_path, config_dir) = resolve_sources()?;

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
    run_gui(
        cfg,
        package_path,
        config_dir,
        uninstall,
        auto_update,
        lang_arg,
    )
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
/// `<installer.toml> [package.bpkg]`). The third value is the config's folder in dev
/// mode, where product catalogues are read from disk when no package is given.
type Sources = (InstallerConfig, Option<PathBuf>, Option<PathBuf>);

fn resolve_sources() -> anyhow::Result<Sources> {
    if let Some(emb) = std::env::current_exe()
        .ok()
        .and_then(|e| bpkg_core::embed::read_embedded(&e).ok().flatten())
    {
        let cfg = InstallerConfig::from_toml(&String::from_utf8_lossy(&emb.config))
            .map_err(|e| anyhow::anyhow!("embedded config: {e}"))?;
        // Stage the embedded .bpkg to a temp file so Package::open can read it.
        let tmp = std::env::temp_dir().join(format!("betterinstaller-{}.bpkg", std::process::id()));
        std::fs::write(&tmp, &emb.bpkg)?;
        return Ok((cfg, Some(tmp), None));
    }

    let mut args = std::env::args().skip(1).filter(|a| !a.starts_with("--"));
    let config_path = args
        .next()
        .unwrap_or_else(|| "examples/bmm/installer.toml".to_string());
    let package_path: Option<PathBuf> = args.next().map(PathBuf::from);
    let cfg = InstallerConfig::load(&config_path)
        .map_err(|e| anyhow::anyhow!("loading {config_path}: {e}"))?;
    let config_dir = Path::new(&config_path).parent().map(Path::to_path_buf);
    Ok((cfg, package_path, config_dir))
}

/// The language the installer opens in, most explicit first: `--lang=`, then
/// `[i18n] default`, then a non-"auto" default on a `language` setup option (how a
/// project pinned the language before `[i18n]` existed), then the OS, then English.
fn initial_language(cfg: &InstallerConfig, lang_arg: Option<&str>) -> String {
    let pinned = |v: Option<&str>| {
        v.filter(|l| *l != "auto")
            .and_then(bpkg_core::i18n::normalize)
    };
    pinned(lang_arg)
        .or_else(|| pinned(cfg.i18n.as_ref().and_then(|i| i.default.as_deref())))
        .or_else(|| {
            pinned(
                cfg.setup_options
                    .iter()
                    .find(|o| o.id == "language")
                    .and_then(|o| o.default.as_str()),
            )
        })
        .or_else(bpkg_core::i18n::detect_os_locale)
        .unwrap_or_else(|| bpkg_core::i18n::FALLBACK.to_string())
}

/// The product's own catalogues: `<locales_dir>/<code>.toml`, from the package, or in
/// dev mode (no package) from beside installer.toml.
///
/// A file that does not parse is skipped, never fatal: the installer still works in the
/// engine's languages, and a broken translation must not stop an install. The i18n tests
/// are where a bad catalogue is caught before it ships.
fn load_product_catalogs(
    cfg: &InstallerConfig,
    pkg: Option<&Path>,
    config_dir: Option<&Path>,
) -> Vec<Catalog> {
    let dir = cfg
        .i18n
        .as_ref()
        .map(|i| i.locales_dir().to_string())
        .unwrap_or_else(|| bpkg_core::config::I18nConfig::DEFAULT_LOCALES_DIR.to_string());
    let dir = dir.trim_end_matches('/').to_string();
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    if let Some(pkg) = pkg {
        if let Ok(mut p) = Package::open(pkg) {
            let prefix = format!("{dir}/");
            let wanted: Vec<String> = p
                .manifest
                .files
                .iter()
                .map(|f| f.path.clone())
                .filter(|f| {
                    f.strip_prefix(&prefix)
                        .is_some_and(|rest| !rest.contains('/') && rest.ends_with(".toml"))
                })
                .collect();
            if let Ok(map) = p.read_files(&wanted) {
                files.extend(map);
            }
        }
    } else if let Some(cd) = config_dir {
        if let Ok(rd) = std::fs::read_dir(cd.join(&dir)) {
            for e in rd.flatten() {
                let path = e.path();
                if path.extension().and_then(|x| x.to_str()) == Some("toml") {
                    if let Ok(b) = std::fs::read(&path) {
                        files.push((path.to_string_lossy().to_string(), b));
                    }
                }
            }
        }
    }
    files
        .into_iter()
        .filter_map(|(name, bytes)| {
            let code = Path::new(&name).file_stem()?.to_str()?.to_string();
            Catalog::parse(&code, &String::from_utf8_lossy(&bytes))
                .map_err(|e| eprintln!("skipping catalogue {name}: {e}"))
                .ok()
        })
        .collect()
}

fn run_gui(
    cfg: InstallerConfig,
    package_path: Option<PathBuf>,
    config_dir: Option<PathBuf>,
    start_uninstall: bool,
    auto_update: bool,
    lang_arg: Option<String>,
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

    // Signature first: it decides whether the product's catalogues and legal documents
    // are read at all (see `Trust::may_read`).
    let trust = detect_signature(&cfg, package_path.as_deref());

    // ── Language ────────────────────────────────────────────────────────────
    //
    // Runtime catalogues (bpkg-core::i18n) rather than Slint's `@tr()` + gettext, for
    // three reasons:
    //  - most of what is on screen is the PRODUCT's text (option labels, legal
    //    documents) from installer.toml and the package, which `@tr()` never sees;
    //  - a product adds a language by shipping `<code>.toml` in its signed package, with
    //    no engine rebuild; `@tr()` bundles translations at compile time;
    //  - switching language mid-flow is one `rev` bump (the `I18n` global in
    //    main.slint), with no window rebuilt and nothing the user entered lost.
    //
    // Right-to-left: a catalogue may declare `direction = "rtl"`. Text is then
    // right-aligned; the layout is not mirrored (Slint has no layout mirroring), and
    // glyph shaping for RTL scripts is whatever the Slint renderer does, which has not
    // been checked against a real RTL catalogue.
    let mut translator = Translator::builtin();
    translator.set_ctx("app", cfg.app.name.clone());
    translator.set_ctx("publisher", cfg.app.publisher.clone());
    translator.set_ctx("version", cfg.app.version.clone());
    translator.set_ctx(
        "installed_version",
        installed_version.clone().unwrap_or_default(),
    );
    translator.set_ctx("new_version", cfg.app.version.clone());
    if trust.may_read() {
        for c in load_product_catalogs(&cfg, package_path.as_deref(), config_dir.as_deref()) {
            translator.add_catalog(c);
        }
    }
    translator.set_language(&initial_language(&cfg, lang_arg.as_deref()));
    let tr: Rc<RefCell<Translator>> = Rc::new(RefCell::new(translator));
    UI_TRANSLATOR.with(|c| *c.borrow_mut() = Some(tr.clone()));

    let ui = MainWindow::new()?;
    {
        let tr = tr.clone();
        ui.global::<I18n>()
            .on_tr(move |key, _rev| tr.borrow().t(&key).into());
    }
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
    ui.set_signed(trust.is_trusted());
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

    // Shared mutable state captured by callbacks.
    let setup_opts = Rc::new(cfg.setup_options.clone());
    let setup_groups: Rc<Vec<SetupGroup>> = Rc::new(cfg.setup_groups.clone());
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
    let cfg_rc = Rc::new(cfg.clone());
    let legal_docs: Rc<RefCell<Vec<LegalDoc>>> = Rc::new(RefCell::new(load_legal_docs(
        &cfg,
        package_path.as_deref(),
        trust.may_read(),
        &tr.borrow(),
    )));
    let legal_count = legal_docs.borrow().len();
    let legal_index: Rc<RefCell<usize>> = Rc::new(RefCell::new(0));
    // One acceptance flag PER document (separate accept for TOS and Privacy).
    let legal_accepted: Rc<RefCell<Vec<bool>>> = Rc::new(RefCell::new(vec![false; legal_count]));
    ui.set_legal_count(legal_count as i32);

    // Setup rows + the option list used for "can proceed" exclude the legal option.
    // Ordered by group, so each heading is drawn once above its options.
    let visible_opts: Rc<Vec<SetupOption>> = Rc::new(order_by_group(
        cfg.setup_options
            .iter()
            .filter(|o| Some(&o.id) != legal_opt_id.as_ref())
            .cloned()
            .collect(),
        &cfg.setup_groups,
    ));
    let model = Rc::new(VecModel::from(option_rows(
        &visible_opts,
        &setup_groups,
        &chosen.borrow(),
        &tr.borrow(),
    )));
    ui.set_options(ModelRc::from(model.clone()));
    ui.set_can_proceed(true); // Welcome's Next is always enabled

    // Rewrite every option row from the current language and choices, in place (the row
    // count never changes, so the ScrollView keeps its position).
    let refresh_rows: Rc<dyn Fn()> = Rc::new({
        let model = model.clone();
        let visible = visible_opts.clone();
        let groups = setup_groups.clone();
        let chosen = chosen.clone();
        let tr = tr.clone();
        move || {
            let rows = option_rows(&visible, &groups, &chosen.borrow(), &tr.borrow());
            for (i, r) in rows.into_iter().enumerate() {
                model.set_row_data(i, r);
            }
        }
    });

    // Push a legal document into the UI + gate the Next button.
    let refresh_legal: RefreshLegal = Rc::new({
        let docs = legal_docs.clone();
        let acc = legal_accepted.clone();
        let tr = tr.clone();
        move |ui: &MainWindow, idx: usize| {
            let tr = tr.borrow();
            if let Some(d) = docs.borrow().get(idx) {
                ui.set_legal_title(d.title.clone().into());
                ui.set_legal_blocks(ModelRc::from(Rc::new(VecModel::from(d.blocks.clone()))));
                ui.set_legal_accept_text(tr.t_with("legal.accept", &[("doc", &d.title)]).into());
                ui.set_legal_notice(d.notice.clone().into());
            }
            ui.set_legal_counter(
                tr.t_with(
                    "legal.counter",
                    &[
                        ("n", &(idx + 1).to_string()),
                        ("total", &legal_count.to_string()),
                    ],
                )
                .into(),
            );
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
        let tr = tr.clone();
        ui.on_browse_location(move || {
            // Native folder picker (Windows uses the OS dialog).
            if let Some(ui) = w.upgrade() {
                let start = ui.get_install_dir().to_string();
                let title = tr.borrow().t("ui.choose_folder");
                let mut dlg = rfd::FileDialog::new().set_title(title);
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
        let chosen = chosen.clone();
        let opts = visible_opts.clone();
        let refresh = refresh_rows.clone();
        ui.on_option_bool_changed(move |id, v| {
            chosen
                .borrow_mut()
                .insert(id.to_string(), serde_json::json!(v));
            refresh();
            if let Some(ui) = w.upgrade() {
                ui.set_can_proceed(compute_can_proceed(&opts, &chosen.borrow()));
            }
        });
    }
    {
        let chosen = chosen.clone();
        let refresh = refresh_rows.clone();
        ui.on_option_select_changed(move |id, v| {
            chosen
                .borrow_mut()
                .insert(id.to_string(), serde_json::json!(v.to_string()));
            refresh();
        });
    }
    {
        // A dropdown reports the INDEX of the label picked; the value comes from
        // `choices`, so a translated label can never end up in the handoff.
        let chosen = chosen.clone();
        let opts = visible_opts.clone();
        let refresh = refresh_rows.clone();
        ui.on_option_choice_picked(move |id, idx| {
            let value = opts
                .iter()
                .find(|o| o.id == id.as_str())
                .and_then(|o| o.choices.get(usize::try_from(idx).ok()?).cloned());
            if let Some(v) = value {
                chosen
                    .borrow_mut()
                    .insert(id.to_string(), serde_json::json!(v));
                refresh();
            }
        });
    }
    {
        let w = ui.as_weak();
        let chosen = chosen.clone();
        let opts = visible_opts.clone();
        let refresh = refresh_rows.clone();
        let legal_id = legal_opt_id.clone();
        ui.on_reset_options(move || {
            // Everything on the Setup page back to its declared default. Legal
            // acceptance is not on this page and is kept.
            chosen
                .borrow_mut()
                .retain(|k, _| Some(k) == legal_id.as_ref());
            refresh();
            if let Some(ui) = w.upgrade() {
                ui.set_can_proceed(compute_can_proceed(&opts, &chosen.borrow()));
            }
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
        ui.set_final_value(o.default.as_str().unwrap_or("").into());
    }
    let label_final: Rc<dyn Fn(&MainWindow)> = Rc::new({
        let opt = final_opt.clone();
        let tr = tr.clone();
        move |ui: &MainWindow| {
            if let Some(o) = opt.as_ref() {
                let tr = tr.borrow();
                ui.set_final_previews(ModelRc::from(Rc::new(VecModel::from(swatch_rows(o, &tr)))));
                ui.set_final_title(option_label(o, &tr).into());
                ui.set_final_hint(option_description(o, &tr).into());
            }
        }
    });
    label_final(&ui);
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
        let refresh = refresh_rows.clone();
        let state = written_handoff.clone();
        let opt = final_opt.clone();
        let tr = tr.clone();
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
                refresh();
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
                                "{}\n{}",
                                ui.get_result_message(),
                                tr.borrow().t("done.final_save_failed")
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
    // Optional prerequisites appear as ordinary component rows.
    //
    // Reusing this list rather than adding a second one: it already has the model, the
    // toggle handler and the layout, and to the person installing there is no
    // difference worth a separate screen between "an optional part of the app" and "an
    // optional thing the app needs". The `prereq:` prefix keeps the ids from ever
    // colliding with a real component's, and is what run_real_install matches on.
    //
    // Only the MISSING ones — offering to install a Python that is already there is
    // noise, and ticking it would download 10 MB to no effect. Detected once: the check
    // runs commands, and a language switch must not run them again.
    let missing_prereqs: Rc<Vec<bpkg_core::config::Prerequisite>> = Rc::new(
        bpkg_core::prereq::optional_missing(&cfg.prerequisites)
            .into_iter()
            .cloned()
            .collect(),
    );
    let label_components: Rc<dyn Fn(&MainWindow)> = Rc::new({
        let cfg = cfg_rc.clone();
        let missing = missing_prereqs.clone();
        let chosen = chosen_components.clone();
        let tr = tr.clone();
        move |ui: &MainWindow| {
            let rows = component_rows(&cfg, &missing, &chosen.borrow(), &tr.borrow());
            ui.set_components(ModelRc::from(Rc::new(VecModel::from(rows))));
        }
    });
    label_components(&ui);
    {
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

    // ── Language switch ─────────────────────────────────────────────────────
    //
    // Everything worded by Rust is re-worded here; everything worded by the .slint file
    // follows the `rev` bump on its own.
    let apply_language: Rc<dyn Fn(&MainWindow)> = Rc::new({
        let tr = tr.clone();
        let refresh_rows = refresh_rows.clone();
        let label_components = label_components.clone();
        let label_final = label_final.clone();
        let docs = legal_docs.clone();
        let acc = legal_accepted.clone();
        let chosen = chosen.clone();
        let legal_id = legal_opt_id.clone();
        let refresh_legal = refresh_legal.clone();
        let li = legal_index.clone();
        let cfg = cfg_rc.clone();
        let pkg = package_path.clone();
        let may_read = trust.may_read();
        let trust = trust.clone();
        move |ui: &MainWindow| {
            let (names, index, rtl, sig) = {
                let t = tr.borrow();
                let avail = t.available();
                let index = avail
                    .iter()
                    .position(|(c, _)| c == t.language())
                    .unwrap_or(0);
                let names: Vec<SharedString> =
                    avail.into_iter().map(|(_, name)| name.into()).collect();
                (
                    names,
                    index,
                    t.direction() == Direction::Rtl,
                    trust.text(&t),
                )
            };
            ui.set_language_names(ModelRc::from(Rc::new(VecModel::from(names))));
            ui.set_language_index(index as i32);
            ui.set_signature_status(sig.into());
            let g = ui.global::<I18n>();
            g.set_rtl(rtl);
            g.set_rev(g.get_rev() + 1);

            refresh_rows();
            label_components(ui);
            label_final(ui);

            // The documents follow the language too. One whose text changed must be
            // accepted again: acceptance is of the text that was read.
            let fresh = load_legal_docs(&cfg, pkg.as_deref(), may_read, &tr.borrow());
            if fresh.len() == docs.borrow().len() {
                let mut a = acc.borrow_mut();
                for (i, (old, new)) in docs.borrow().iter().zip(&fresh).enumerate() {
                    if old.file != new.file {
                        a[i] = false;
                    }
                }
                let all = a.iter().all(|x| *x);
                if let Some(id) = &legal_id {
                    if chosen.borrow().contains_key(id) {
                        chosen
                            .borrow_mut()
                            .insert(id.clone(), serde_json::json!(all));
                    }
                }
                drop(a);
                *docs.borrow_mut() = fresh;
                if ui.get_page() == 1 {
                    refresh_legal(ui, *li.borrow());
                }
            }
        }
    });
    apply_language(&ui);
    {
        let w = ui.as_weak();
        let tr = tr.clone();
        let apply = apply_language.clone();
        ui.on_language_picked(move |idx| {
            let code = tr
                .borrow()
                .available()
                .get(usize::try_from(idx).unwrap_or(0))
                .map(|(c, _)| c.clone());
            if let (Some(code), Some(ui)) = (code, w.upgrade()) {
                tr.borrow_mut().set_language(&code);
                apply(&ui);
            }
        });
    }

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
                        UI_TRANSLATOR.with(|c| {
                            if let Some(tr) = c.borrow().as_ref() {
                                tr.borrow_mut().set_ctx("new_version", newv.clone());
                            }
                        });
                        let g = ui.global::<I18n>();
                        g.set_rev(g.get_rev() + 1);
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
        let tr = tr.clone();
        // Only a fresh install offers the picker — Repair and Update do not
        // rewrite the handoff, so there would be nothing for it to change.
        let show_final = final_opt.is_some();
        ui.on_install(move || {
            let ui = match w.upgrade() {
                Some(u) => u,
                None => return,
            };
            // A snapshot: the worker thread words its progress in the language the
            // install started in.
            let trs: Translator = tr.borrow().clone();

            // 1) Write the real handoff file (the headline feature).
            let mut message = String::new();
            let mut ok = true;
            if let Some(h) = handoff_cfg.as_ref().filter(|h| h.enabled) {
                // Resolve a still-"auto" select (e.g. language) to a concrete choice.
                // Without this, leaving the default "auto" means the app never receives
                // a concrete language/select choice (the "selects not applied" bug).
                //
                // "auto" follows the language the installer is SHOWN in, which is the
                // OS language unless the user picked another one here: someone who
                // switched the installer to French expects the app in French too. The
                // first entry of that language's fallback chain among the option's own
                // choices wins, so `fr-CH` resolves to `fr` and an unsupported language
                // to `en`.
                {
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
                                if let Some(v) = trs.resolve_among(&opt.choices) {
                                    ch.insert(opt.id.clone(), serde_json::json!(v));
                                }
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
                        message = trs.t_with(
                            "done.handoff_written",
                            &[("path", &path.display().to_string())],
                        );
                        // Kept so the Done-page picker can amend this exact file.
                        *written_handoff.borrow_mut() = Some((path.clone(), doc.clone()));
                    }
                    Err(e) => {
                        ok = false;
                        message = trs.t_with("done.handoff_failed", &[("error", &e.to_string())]);
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
            ui.set_progress_label(trs.t("progress.preparing").into());

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
                    let lrows = launch_rows(&launch_cfg, &launch_checked.borrow(), &comps, &trs);
                    std::thread::spawn(move || {
                        let result =
                            run_real_install(weak.clone(), &pkg, &dest, &comps, &integ, &trs);
                        let _ = weak.upgrade_in_event_loop(move |ui| {
                            match result {
                                Ok(n) => {
                                    ui.set_success(handoff_ok);
                                    ui.set_result_title(trs.t("done.installed_title").into());
                                    ui.set_result_message(
                                        format!(
                                            "{}\n{}",
                                            trs.t_with(
                                                "done.installed_message",
                                                &[
                                                    ("n", &n.to_string()),
                                                    ("dir", &dest.display().to_string())
                                                ]
                                            ),
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
                                    ui.set_result_title(trs.t("done.install_failed_title").into());
                                    ui.set_result_message(
                                        trs.t_with("done.install_failed_message", &[("error", &e)])
                                            .into(),
                                    );
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
                            ui.set_result_title(trs.t("done.installed_title").into());
                            ui.set_result_message(final_msg.clone().into());
                            ui.set_page(4);
                            ui.set_final_visible(show_final && ok);
                            if let Some(t) = pt.borrow().as_ref() {
                                t.stop();
                            }
                        } else {
                            ui.set_progress(*p);
                            ui.set_progress_label(
                                trs.t_with(
                                    "progress.installing_pct",
                                    &[("pct", &((*p * 100.0) as i32).to_string())],
                                )
                                .into(),
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
        let tr = tr.clone();
        ui.on_repair(move || {
            let ui = match w.upgrade() {
                Some(u) => u,
                None => return,
            };
            let trs = tr.borrow().clone();
            ui.set_maintenance_verb(trs.t("steps.repair").into());
            let pkg = match &pkg {
                Some(p) => p.clone(),
                None => {
                    ui.set_success(false);
                    ui.set_result_title(trs.t("done.repair_failed_title").into());
                    ui.set_result_message(trs.t("done.repair_nothing").into());
                    ui.set_page(4);
                    return;
                }
            };
            let comps_v = comps.borrow().clone();
            let lrows = launch_rows(&launch_cfg, &launch_checked.borrow(), &comps_v, &trs);
            spawn_reinstall(
                &ui,
                pkg,
                loc.clone(),
                comps_v,
                integ.clone(),
                lrows,
                Reinstall::Repair,
                trs,
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
        let tr = tr.clone();
        // `[update] allow_delta` (default true). The delta path in `download_and_apply` is
        // reached only when a current .bpkg is passed, so withholding it IS the off switch —
        // set it here, once, rather than re-reading config on the worker thread.
        let allow_delta = cfg.update.as_ref().is_none_or(|u| u.allow_delta);
        ui.on_update_app(move || {
            let ui = match w.upgrade() {
                Some(u) => u,
                None => return,
            };
            let trs = tr.borrow().clone();
            ui.set_maintenance_verb(trs.t("steps.update").into());
            let comps_v = comps.borrow().clone();

            // Preferred path: a configured remote update was found.
            if let Some(m) = REMOTE_MANIFEST.with(|c| c.borrow().clone()) {
                let lrows = launch_rows(&launch_cfg, &launch_checked.borrow(), &comps_v, &trs);
                ui.set_page(3);
                ui.set_progress(0.2);
                ui.set_progress_label(
                    trs.t_with("progress.downloading", &[("new_version", &m.version)])
                        .into(),
                );
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
                        let v: &[(&str, &str)] = &[("new_version", &m.version)];
                        match res {
                            Ok(n) => {
                                ui.set_success(true);
                                ui.set_result_title(trs.t_with("done.updated_to_title", v).into());
                                ui.set_result_message(
                                    trs.t_with(
                                        "done.updated_to_message",
                                        &[
                                            ("new_version", &m.version),
                                            ("n", &n.to_string()),
                                            ("dir", &dir.display().to_string()),
                                        ],
                                    )
                                    .into(),
                                );
                                apply_launch_rows(&ui, lrows);
                            }
                            Err(e) => {
                                ui.set_success(false);
                                ui.set_result_title(trs.t("done.update_failed_title").into());
                                ui.set_result_message(e.to_string().into());
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
                    ui.set_result_title(trs.t("done.update_failed_title").into());
                    ui.set_result_message(trs.t("done.update_nothing").into());
                    ui.set_page(4);
                    return;
                }
            };
            let lrows = launch_rows(&launch_cfg, &launch_checked.borrow(), &comps_v, &trs);
            spawn_reinstall(
                &ui,
                pkg,
                loc.clone(),
                comps_v,
                integ.clone(),
                lrows,
                Reinstall::Update,
                trs,
            );
        });
    }

    // ── Maintenance: Uninstall ──────────────────────────────────────────
    {
        let w = ui.as_weak();
        let loc = loc_uninstall;
        let tr = tr.clone();
        ui.on_uninstall_app(move || {
            let ui = match w.upgrade() {
                Some(u) => u,
                None => return,
            };
            let trs = tr.borrow().clone();
            ui.set_maintenance_verb(trs.t("steps.uninstall").into());
            ui.set_page(3);
            ui.set_progress(0.4);
            ui.set_progress_label(trs.t("progress.uninstalling").into());
            let dir = loc.clone();
            let weak = ui.as_weak();
            std::thread::spawn(move || {
                let result = do_uninstall_full(&dir);
                let _ = weak.upgrade_in_event_loop(move |ui| {
                    match result {
                        Ok(()) => {
                            ui.set_success(true);
                            ui.set_result_title(trs.t("done.uninstalled_title").into());
                            ui.set_result_message(trs.t("done.uninstalled_message").into());
                        }
                        Err(e) => {
                            ui.set_success(false);
                            ui.set_result_title(trs.t("done.uninstall_failed_title").into());
                            ui.set_result_message(e.into());
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
    tr: &Translator,
) -> Result<u64, String> {
    // Prerequisites: auto-download/-install the missing required ones (those with a
    // download_url), error on any still missing. Done before touching the install.
    {
        let weak = weak.clone();
        // The selection list carries both kinds; `prereq:` marks the ones that are not
        // package components. Split here rather than earlier so there is one selection
        // model in the UI — but the package extraction below must never be handed a
        // `prereq:` id, because it would silently match no files and install nothing.
        let opted_in: Vec<String> = comps
            .iter()
            .filter_map(|c| c.strip_prefix("prereq:").map(str::to_string))
            .collect();
        // `dest` so a zip prerequisite (a downloaded runtime) unpacks under the install
        // directory the user chose, not somewhere fixed.
        bpkg_core::prereq::ensure_required(&integ.prereqs, dest, &opted_in, |name| {
            let label = tr.t_with("progress.prerequisite", &[("name", name)]);
            let _ = weak.upgrade_in_event_loop(move |ui| {
                ui.set_progress_label(label.into());
            });
        })
        .map_err(|e| e.to_string())?;
    }

    // If a previous version is running, close it first — otherwise its locked .exe /
    // resources make the overwrite (install / repair / update) fail. No-op on a fresh
    // install where the dir doesn't exist yet.
    if dest.exists() {
        let label = tr.t("progress.closing");
        let _ = weak.upgrade_in_event_loop(move |ui| {
            ui.set_progress_label(label.into());
        });
        kill_running_apps(dest);
        std::thread::sleep(std::time::Duration::from_millis(400));
    }

    // Writability preflight — fail with a clear message instead of a cryptic I/O
    // error if the chosen folder needs administrator rights (e.g. Program Files).
    if let Err(e) = std::fs::create_dir_all(dest) {
        return Err(tr.t_with(
            "errors.not_writable",
            &[
                ("dir", &dest.display().to_string()),
                ("error", &e.to_string()),
            ],
        ));
    }

    let mut p = Package::open(pkg).map_err(|e| e.to_string())?;

    // Verify the Ed25519 signature before writing anything, when a trust key is set.
    if let Some(pk_hex) = integ.public_key.as_ref() {
        let vk = bpkg_core::sign::parse_public(pk_hex).map_err(|e| e.to_string())?;
        let valid = p.verify_signature(&vk).map_err(|e| e.to_string())?;
        signature_verdict(valid, p.is_signed(), integ.require_signature).map_err(|k| tr.t(k))?;
    }

    // Package components only. A `prereq:` id here would match no files, and the empty
    // case below means "install everything" — so a run where the user ticked only a
    // prerequisite would have looked like a full install rather than a component-filtered
    // one. Filtering keeps the two selections from meaning anything to each other.
    let pkg_comps: Vec<String> = comps
        .iter()
        .filter(|c| !c.starts_with("prereq:"))
        .cloned()
        .collect();
    let comp: Option<&[String]> = if pkg_comps.is_empty() {
        None
    } else {
        Some(&pkg_comps)
    };
    let mut last_pct = -1i32;
    let written = p
        .install_with_progress(dest, comp, |done, total, file| {
            let pct = (done * 100)
                .checked_div(total)
                .map(|p| p as i32)
                .unwrap_or(100);
            if pct != last_pct {
                last_pct = pct;
                let label = tr.t_with(
                    "progress.installing_file",
                    &[("pct", &pct.to_string()), ("file", file)],
                );
                let _ = weak.upgrade_in_event_loop(move |ui| {
                    ui.set_progress(pct as f32 / 100.0);
                    ui.set_progress_label(label.into());
                });
            }
        })
        .map_err(|e| e.to_string())?;

    // After files land: shortcuts, protocol, uninstaller (best-effort).
    let label = tr.t("progress.finishing");
    let _ = weak.upgrade_in_event_loop(move |ui| {
        ui.set_progress_label(label.into());
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

/// What the Welcome page can say about the package, decided once at startup.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Trust {
    /// No package: running the dev binary on a bare installer.toml.
    Preview,
    Unreadable,
    Unsigned,
    /// Signed, and verified against the pinned `[security].public_key`.
    Verified,
    /// Signed, but the signature does not verify against the pinned key.
    Invalid,
    /// Signed, with no key configured to check it against.
    Signed,
}

impl Trust {
    fn is_trusted(&self) -> bool {
        matches!(self, Trust::Verified | Trust::Signed)
    }

    /// May text be read from this package before the install (product catalogues, legal
    /// documents)? Not when its signature is known to be broken: the install is refused
    /// anyway (`signature_verdict`), and a tampered package should not get to word the
    /// screens that lead up to that refusal. An UNSIGNED package is still read: whether it
    /// may install is `require_signature`'s decision, made at install time.
    fn may_read(&self) -> bool {
        !matches!(self, Trust::Invalid | Trust::Unreadable)
    }

    fn text(&self, tr: &Translator) -> String {
        tr.t(match self {
            Trust::Preview => "signature.preview",
            Trust::Unreadable => "signature.unreadable",
            Trust::Unsigned => "signature.unsigned",
            Trust::Verified => "signature.verified",
            Trust::Invalid => "signature.invalid",
            Trust::Signed => "signature.signed",
        })
    }
}

/// Verify the package against the pinned key (if any), for the Welcome-page badge and
/// for [`Trust::may_read`]. Install-time verification is separate and unchanged.
fn detect_signature(cfg: &InstallerConfig, pkg: Option<&Path>) -> Trust {
    let pkg = match pkg {
        Some(p) => p,
        None => return Trust::Preview,
    };
    let mut p = match Package::open(pkg) {
        Ok(p) => p,
        Err(_) => return Trust::Unreadable,
    };
    if !p.is_signed() {
        return Trust::Unsigned;
    }
    if let Some(pk) = cfg.security.as_ref().and_then(|s| s.public_key.as_ref()) {
        return match bpkg_core::sign::parse_public(pk).and_then(|vk| p.verify_signature(&vk)) {
            Ok(true) => Trust::Verified,
            _ => Trust::Invalid,
        };
    }
    Trust::Signed
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
///
/// The error is a catalogue key, worded by the caller in the installer's language.
fn signature_verdict(
    valid: bool,
    is_signed: bool,
    require_signature: bool,
) -> Result<(), &'static str> {
    if valid {
        return Ok(());
    }
    if is_signed {
        return Err("errors.signature_invalid");
    }
    if require_signature {
        return Err("errors.signature_missing");
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
    tr: &Translator,
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
                product_text(tr, &format!("launch.{}.label", l.id), Some(&l.label))
                    .unwrap_or_else(|| l.id.clone()),
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

/// Which maintenance action a re-extraction is, for its wording.
#[derive(Clone, Copy)]
enum Reinstall {
    Repair,
    Update,
}

impl Reinstall {
    /// (progress, done title, done message, failed title)
    fn keys(self) -> (&'static str, &'static str, &'static str, &'static str) {
        match self {
            Reinstall::Repair => (
                "progress.repairing",
                "done.repaired_title",
                "done.repaired_message",
                "done.repair_failed_title",
            ),
            Reinstall::Update => (
                "progress.updating",
                "done.updated_title",
                "done.updated_message",
                "done.update_failed_title",
            ),
        }
    }
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
    kind: Reinstall,
    tr: Translator,
) {
    let (progress_key, title_key, message_key, failed_key) = kind.keys();
    ui.set_page(3);
    ui.set_progress(0.0);
    ui.set_progress_label(tr.t(progress_key).into());
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let result = run_real_install(weak.clone(), &pkg, &dest, &comps, &integ, &tr);
        let _ = weak.upgrade_in_event_loop(move |ui| {
            match result {
                Ok(n) => {
                    ui.set_success(true);
                    ui.set_result_title(tr.t(title_key).into());
                    ui.set_result_message(
                        tr.t_with(
                            message_key,
                            &[("n", &n.to_string()), ("dir", &dest.display().to_string())],
                        )
                        .into(),
                    );
                    apply_launch_rows(&ui, lrows);
                }
                Err(e) => {
                    ui.set_success(false);
                    ui.set_result_title(tr.t(failed_key).into());
                    ui.set_result_message(e.into());
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
    /// The package path actually shown. A language switch that changes it asks for
    /// acceptance again.
    file: String,
    /// Non-empty when the document is not in the language the installer is shown in.
    notice: String,
}

/// The `_<LANG>` sibling of a document name: `TOS.md` + `fr` → `TOS_FR.md`,
/// `TOS.md` + `pt-BR` → `TOS_PT-BR.md`. `None` for English, which is the file itself.
///
/// Kept from before `localized_documents` existed: a package that ships `TOS_FR.md` next
/// to `TOS.md` is translated with no config change.
fn localized_doc_name(doc: &str, lang: &str) -> Option<String> {
    let l = lang.to_uppercase();
    if l.is_empty() || l == "EN" {
        return None;
    }
    let (stem, ext) = doc.rsplit_once('.')?;
    Some(format!("{stem}_{l}.{ext}"))
}

/// For each document of the license option, the package paths to try, best first, each
/// with the language it is in.
///
/// Walks the language chain (`pt-BR` → `pt` → `en`); at each step the explicit
/// `localized_documents` entry comes first, then the `_<LANG>` sibling. English is the
/// option's own `documents`. Terms and a privacy policy shown in a language the reader
/// may not have, in an installer that is otherwise translated, is a consent problem before
/// it is a polish one; this keeps the fallback explicit and ordered.
fn legal_candidates(opt: &SetupOption, chain: &[String]) -> Vec<Vec<(String, String)>> {
    opt.documents
        .iter()
        .enumerate()
        .map(|(i, doc)| {
            let mut c: Vec<(String, String)> = Vec::new();
            for code in chain {
                if code == bpkg_core::i18n::FALLBACK {
                    c.push((doc.clone(), code.clone()));
                    continue;
                }
                for ld in &opt.localized_documents {
                    if bpkg_core::i18n::normalize(&ld.lang).as_deref() == Some(code.as_str()) {
                        if let Some(d) = ld.documents.get(i) {
                            c.push((d.clone(), code.clone()));
                        }
                    }
                }
                if let Some(sib) = localized_doc_name(doc, code) {
                    c.push((sib, code.clone()));
                }
            }
            // The explicit entry and the sibling spelling are often the same file.
            let mut seen = std::collections::HashSet::new();
            c.retain(|(f, _)| seen.insert(f.clone()));
            c
        })
        .collect()
}

fn load_legal_docs(
    cfg: &InstallerConfig,
    pkg: Option<&Path>,
    may_read: bool,
    tr: &Translator,
) -> Vec<LegalDoc> {
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
        Some(p) if may_read => p,
        _ => return out,
    };
    let mut p = match Package::open(pkg) {
        Ok(p) => p,
        Err(_) => return out,
    };
    let candidates = legal_candidates(lo, tr.chain());
    // Every candidate in one read: a single pass over the archive.
    let wanted: Vec<String> = candidates
        .iter()
        .flatten()
        .map(|(f, _)| f.clone())
        .collect();
    let map = match p.read_files(&wanted) {
        Ok(m) => m,
        Err(_) => return out,
    };
    for list in &candidates {
        let picked = list
            .iter()
            .find_map(|(f, code)| map.get(f).map(|b| (f, code, b)));
        if let Some((name, code, bytes)) = picked {
            out.push(LegalDoc {
                title: doc_title(name, tr),
                blocks: parse_md(&String::from_utf8_lossy(bytes)),
                file: name.clone(),
                notice: fallback_notice(code, tr),
            });
        }
    }
    out
}

/// Said only when a document fell back to ANOTHER language than the one on screen; `fr`
/// for an `fr-CH` reader is the same language, and an English screen needs no notice.
fn fallback_notice(doc_lang: &str, tr: &Translator) -> String {
    let shown = tr.language();
    if doc_lang == shown || shown == bpkg_core::i18n::FALLBACK {
        return String::new();
    }
    tr.t_with(
        "legal.fallback_notice",
        &[("language", &tr.language_name(shown))],
    )
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
        let tok = tok.trim_end_matches(['.', ',', ';', ':', '!', '?', '"']);
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

/// Friendly title for a legal document filename, in the installer's language.
///
/// A product may name any document in its catalogue as `docs.<stem>` (lower-case, without
/// the language suffix); otherwise the engine recognises terms, privacy and licence files.
fn doc_title(file: &str, tr: &Translator) -> String {
    let base = file.rsplit('/').next().unwrap_or(file);
    let stem = base.rsplit_once('.').map(|(s, _)| s).unwrap_or(base);
    // Drop a `_FR` / `_PT-BR` language suffix: how the file was picked is not a title.
    let bare = match stem.rsplit_once('_') {
        Some((head, tail))
            if !head.is_empty()
                && bpkg_core::i18n::normalize(tail).is_some()
                && !tail.chars().any(|c| c.is_ascii_lowercase()) =>
        {
            head
        }
        _ => stem,
    };
    let low = bare.to_lowercase();
    if let Some(t) = tr.lookup(&format!("docs.{low}")) {
        return tr.fill(t, &[]);
    }
    let key = if low.contains("privacy") {
        "docs.privacy"
    } else if low.contains("eula") {
        "docs.eula"
    } else if low.contains("tos") || low.contains("terms") {
        "docs.tos"
    } else if low.contains("licen") {
        "docs.license"
    } else {
        return bare.to_string();
    };
    tr.t(key)
}

/// Options in display order: ungrouped first, then each `[[setup_group]]` in the order
/// the config declares them. Stable, so options keep their config order inside a group.
fn order_by_group(mut opts: Vec<SetupOption>, groups: &[SetupGroup]) -> Vec<SetupOption> {
    let rank = |o: &SetupOption| {
        o.group
            .as_ref()
            .and_then(|g| groups.iter().position(|sg| &sg.id == g))
            .map(|i| i + 1)
            .unwrap_or(0)
    };
    opts.sort_by_key(|o| rank(o));
    opts
}

/// A product string: its catalogue key along the language chain, else the config's
/// English text.
fn product_text(tr: &Translator, key: &str, english: Option<&str>) -> Option<String> {
    tr.lookup(key)
        .or(english)
        .filter(|s| !s.is_empty())
        .map(|s| tr.fill(s, &[]))
}

fn option_label(o: &SetupOption, tr: &Translator) -> String {
    product_text(tr, &format!("options.{}.label", o.id), o.label.as_deref())
        .unwrap_or_else(|| humanize(&o.label_key))
}

fn option_description(o: &SetupOption, tr: &Translator) -> String {
    product_text(
        tr,
        &format!("options.{}.description", o.id),
        o.description.as_deref(),
    )
    .unwrap_or_default()
}

/// What the user reads for one choice value. The value itself only as a last resort.
fn choice_label(o: &SetupOption, value: &str, tr: &Translator) -> String {
    let preview = o
        .previews
        .iter()
        .find(|p| p.value == value)
        .and_then(|p| p.label.as_deref());
    product_text(tr, &format!("options.{}.choices.{value}", o.id), preview)
        .unwrap_or_else(|| value.to_string())
}

/// Build the Setup page rows: labels in the current language, current values, the
/// default spelled out, and a heading on the first row of each group.
fn option_rows(
    opts: &[SetupOption],
    groups: &[SetupGroup],
    chosen: &BTreeMap<String, serde_json::Value>,
    tr: &Translator,
) -> Vec<OptionRow> {
    let mut prev_group: Option<&str> = None;
    opts.iter()
        .map(|o| {
            let kind = match o.kind {
                SetupOptionKind::Bool => "bool",
                SetupOptionKind::Select => "select",
                SetupOptionKind::License => "license",
                SetupOptionKind::Swatch => "swatch",
            };
            let current = chosen
                .get(&o.id)
                .cloned()
                .unwrap_or_else(|| o.default.clone());
            let is_default = current == o.default;
            let default_text = match o.kind {
                SetupOptionKind::Bool => tr.t(if o.default.as_bool().unwrap_or(false) {
                    "setup.on"
                } else {
                    "setup.off"
                }),
                SetupOptionKind::Select | SetupOptionKind::Swatch => {
                    choice_label(o, o.default.as_str().unwrap_or(""), tr)
                }
                SetupOptionKind::License => String::new(),
            };
            let default_label = if default_text.is_empty() {
                String::new()
            } else {
                tr.t_with("setup.default", &[("value", &default_text)])
            };
            let value = current.as_str().unwrap_or("").to_string();
            let labels: Vec<SharedString> = o
                .choices
                .iter()
                .map(|c| choice_label(o, c, tr).into())
                .collect();
            let choices: Vec<SharedString> = o.choices.iter().map(|c| c.clone().into()).collect();
            let choice_index = o.choices.iter().position(|c| *c == value).unwrap_or(0) as i32;

            let group = o.group.as_deref();
            let (header, header_description) = match group {
                Some(g) if prev_group != Some(g) => {
                    let sg = groups.iter().find(|sg| sg.id == g);
                    (
                        product_text(
                            tr,
                            &format!("groups.{g}.label"),
                            sg.map(|s| s.label.as_str()),
                        )
                        .unwrap_or_else(|| humanize(g)),
                        product_text(
                            tr,
                            &format!("groups.{g}.description"),
                            sg.and_then(|s| s.description.as_deref()),
                        )
                        .unwrap_or_default(),
                    )
                }
                _ => (String::new(), String::new()),
            };
            prev_group = group;

            OptionRow {
                id: o.id.clone().into(),
                kind: kind.into(),
                label: option_label(o, tr).into(),
                description: option_description(o, tr).into(),
                choices: ModelRc::from(Rc::new(VecModel::from(choices))),
                choice_labels: ModelRc::from(Rc::new(VecModel::from(labels))),
                choice_index,
                previews: ModelRc::from(Rc::new(VecModel::from(swatch_rows(o, tr)))),
                bool_value: current.as_bool().unwrap_or(false),
                string_value: value.into(),
                default_label: default_label.into(),
                is_default,
                sends_data: o.sends_data,
                header: header.into(),
                header_description: header_description.into(),
            }
        })
        .collect()
}

/// Component rows for the Welcome page, then the optional prerequisites that are missing.
fn component_rows(
    cfg: &InstallerConfig,
    missing: &[bpkg_core::config::Prerequisite],
    chosen: &[String],
    tr: &Translator,
) -> Vec<CompRow> {
    let mut rows: Vec<CompRow> = cfg
        .components
        .iter()
        .map(|c| CompRow {
            id: c.id.clone().into(),
            name: product_text(tr, &format!("components.{}.name", c.id), Some(&c.name))
                .unwrap_or_else(|| c.id.clone())
                .into(),
            description: product_text(
                tr,
                &format!("components.{}.description", c.id),
                Some(&c.description),
            )
            .unwrap_or_default()
            .into(),
            size: if c.size_mb > 0 {
                format!("{} MB", c.size_mb).into()
            } else {
                SharedString::new()
            },
            required: c.required,
            checked: c.required || chosen.contains(&c.id),
        })
        .collect();
    for p in missing {
        let id = format!("prereq:{}", p.id);
        rows.push(CompRow {
            name: product_text(tr, &format!("prereqs.{}.name", p.id), Some(&p.name))
                .unwrap_or_else(|| p.id.clone())
                .into(),
            description: tr
                .t(if p.check_command.is_some() {
                    "prereq.missing_install"
                } else {
                    "prereq.missing_download"
                })
                .into(),
            size: SharedString::new(),
            required: false,
            // Unticked unless the user ticked it. A download nobody asked for is not a
            // default, and the app works without it.
            checked: chosen.contains(&id),
            id: id.into(),
        });
    }
    rows
}

/// Build the tile models for a `swatch` option.
fn swatch_rows(o: &SetupOption, tr: &Translator) -> Vec<SwatchRow> {
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
                label: choice_label(o, &p.value, tr).into(),
                bg: at(0, Color::from_rgb_u8(0x0d, 0x11, 0x17)),
                surface: at(1, Color::from_rgb_u8(0x16, 0x1b, 0x22)),
                accent: at(2, Color::from_rgb_u8(0x3b, 0x82, 0xf6)),
                ink: at(3, Color::from_rgb_u8(0xe6, 0xed, 0xf3)),
            }
        })
        .collect()
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

/// "setup.skip_tutorial" → "Skip tutorial". The last resort for an option with no label
/// in the config and none in any catalogue.
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
    use super::{
        doc_title, fallback_notice, is_web_url, legal_candidates, option_rows, order_by_group,
        parse_md, signature_verdict,
    };
    use bpkg_core::config::InstallerConfig;
    use bpkg_core::i18n::Translator;
    use slint::Model;

    fn tr(lang: &str) -> Translator {
        let mut t = Translator::builtin();
        t.set_language(lang);
        t
    }

    const BMM: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/bmm/installer.toml"
    );

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
        assert_eq!(
            doc_title("TOS_FR.md", &tr("fr")),
            "Conditions d'utilisation"
        );
        assert_eq!(doc_title("TOS.md", &tr("en")), "Terms of Service");
        assert_eq!(
            doc_title("PRIVACY_FR.md", &tr("fr")),
            "Politique de confidentialit\u{e9}"
        );
        assert_eq!(doc_title("PRIVACY.md", &tr("en")), "Privacy Policy");
        assert_eq!(doc_title("LICENSE.md", &tr("fr")), "Licence");
        // Any other bundled document falls back to its name without the language suffix.
        assert_eq!(doc_title("CONTRIBUTING_FR.md", &tr("fr")), "CONTRIBUTING");
        assert_eq!(
            doc_title("docs/CONTRIBUTING_PT-BR.md", &tr("en")),
            "CONTRIBUTING"
        );
    }

    #[test]
    fn legal_documents_walk_the_language_chain_then_english() {
        let cfg = InstallerConfig::load(BMM).unwrap();
        let legal = cfg.setup_options.iter().find(|o| o.id == "legal").unwrap();

        // fr-CH: a Swiss-French sibling if the package has one, then the French entry,
        // then English. Each file once.
        let c = legal_candidates(legal, tr("fr-CH").chain());
        let tos: Vec<&str> = c[0].iter().map(|(f, _)| f.as_str()).collect();
        assert_eq!(tos, ["TOS_FR-CH.md", "TOS_FR.md", "TOS.md"]);
        assert_eq!(c[1].last().map(|(f, _)| f.as_str()), Some("PRIVACY.md"));

        // A language BMM does not translate: `_DE` siblings are tried (a package may
        // ship them with no config change), then English, and nothing else.
        let c = legal_candidates(legal, tr("de").chain());
        let tos: Vec<&str> = c[0].iter().map(|(f, _)| f.as_str()).collect();
        assert_eq!(tos, ["TOS_DE.md", "TOS.md"]);
    }

    #[test]
    fn a_document_in_another_language_says_so() {
        // French screen, English document: told.
        let fr = tr("fr");
        let n = fallback_notice("en", &fr);
        assert!(n.contains("anglaise") && n.contains("Fran"), "{n}");
        // Same language, or a regional reader getting the base language: silent.
        assert_eq!(fallback_notice("fr", &tr("fr-CH")), "");
        // English screen: nothing to explain.
        assert_eq!(fallback_notice("en", &tr("en")), "");
    }

    #[test]
    fn setup_rows_are_grouped_translated_and_show_their_default() {
        let cfg = InstallerConfig::load(BMM).unwrap();
        let opts = order_by_group(
            cfg.setup_options
                .iter()
                .filter(|o| o.id != "legal")
                .cloned()
                .collect(),
            &cfg.setup_groups,
        );
        let mut t = tr("fr");
        let dir = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/bmm/installer-locales"
        );
        for code in ["en", "fr"] {
            let text = std::fs::read_to_string(format!("{dir}/{code}.toml")).unwrap();
            t.add_catalog(bpkg_core::i18n::Catalog::parse(code, &text).unwrap());
        }
        let rows = option_rows(&opts, &cfg.setup_groups, &Default::default(), &t);

        // One heading per group, on the first row of it, and every grouped row has one
        // above it somewhere.
        let headers: Vec<String> = rows
            .iter()
            .filter(|r| !r.header.is_empty())
            .map(|r| r.header.to_string())
            .collect();
        assert_eq!(headers.len(), cfg.setup_groups.len(), "{headers:?}");

        // Nothing is changed yet, and every row says what its default is.
        for r in &rows {
            assert!(r.is_default, "{} starts changed", r.id);
            assert!(!r.default_label.is_empty(), "{} shows no default", r.id);
        }

        // Telemetry is in French, marked as sending data, and its default is On.
        let tel = rows.iter().find(|r| r.id == "telemetry").unwrap();
        assert!(tel.sends_data);
        assert!(tel.default_label.contains("Activ"), "{}", tel.default_label);
        assert!(!tel.label.contains("telemetry"), "{}", tel.label);

        // A select shows labels and keeps values: the dropdown reports an index into
        // `choices`, so a label can never become the handoff value.
        let lang = rows.iter().find(|r| r.id == "language").unwrap();
        assert_eq!(lang.choices.row_count(), lang.choice_labels.row_count());
        assert_eq!(lang.choices.row_data(0).unwrap(), "auto");
        assert_ne!(lang.choice_labels.row_data(0).unwrap(), "auto");
        assert_eq!(lang.choice_index, 0);
    }

    #[test]
    fn a_tampered_package_is_refused_even_when_signatures_are_optional() {
        // The regression this guards: signed, does not verify, require_signature = false
        // (the DEFAULT). This must fail closed — it used to install silently.
        let err = signature_verdict(false, true, false)
            .expect_err("a package with a broken signature must never install");
        assert_eq!(err, "errors.signature_invalid");
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
        assert_eq!(err, "errors.signature_missing");
    }

    #[test]
    fn a_good_signature_always_installs() {
        for require in [false, true] {
            assert!(signature_verdict(true, true, require).is_ok());
        }
    }
}
