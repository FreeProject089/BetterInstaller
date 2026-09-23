//! Runtime translation catalogues for the installer.
//!
//! A catalogue is one TOML file per language: a `[meta]` table (native name, text
//! direction) and any number of tables of strings, flattened to dotted keys
//! (`[ui] next = "…"` is the key `ui.next`). Two layers are merged:
//!
//! 1. **Engine** catalogues, `crates/bpkg-core/locales/<code>.toml`, embedded at build time
//!    by `build.rs`. Adding a file adds a language; no code changes.
//! 2. **Product** catalogues, `<code>.toml` files shipped inside the product's signed
//!    package (`[i18n] locales_dir`). They add the product's own strings (option labels,
//!    component names…) and may override engine strings or add a language the engine
//!    does not have, with no engine rebuild at all — everything except the handful of keys
//!    in [`ENGINE_ONLY`], where the installer speaks ABOUT the package rather than for it.
//!
//! Lookups walk a fallback chain: the requested tag, its shorter prefixes, then English
//! (`pt-BR` → `pt` → `en`). A key missing everywhere comes back as the key itself, so a
//! gap is visible rather than blank.
//!
//! Why not Slint's `@tr()` + gettext: bundled translations are compiled into the binary
//! (a language added by a product would need a rebuilt engine), and most of the text on
//! screen comes from the product's installer.toml, which `@tr()` never sees. A runtime
//! catalogue covers both and switches language mid-flow without restarting.

use std::collections::BTreeMap;

include!(concat!(env!("OUT_DIR"), "/builtin_locales.rs"));

/// The language every chain ends with, and the one whose catalogue is complete.
pub const FALLBACK: &str = "en";

/// Where a catalogue came from, and therefore how far its strings are trusted.
///
/// A product catalogue is read out of a package the installer has not necessarily
/// verified — `Trust::may_read` lets an UNSIGNED package supply one, because refusing to
/// read it would leave the screen that has to say "unsigned" untranslated. Its strings are
/// therefore attacker-controlled in the "someone hands you a package" model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Compiled into this binary from `crates/bpkg-core/locales/`.
    Engine,
    /// Read at runtime from a product's package (or, in dev mode, from beside
    /// `installer.toml`). Untrusted.
    Product,
}

/// Keys only the ENGINE may word: the ones in which the installer speaks about the
/// package rather than for it.
///
/// Every other string is fair game — overriding `ui.next` or `done.installed_title` is the
/// documented point of product catalogues. These are not text about the product; they are
/// the installer's own verdict ON the product, plus the badge that warns an option sends
/// data off the machine. A package that can word them can show "Signed & verified" while
/// being unsigned, or blank the "Sends data" badge on a telemetry toggle, and the reader
/// has no way to tell the engine's voice from the package's.
const ENGINE_ONLY: &[&str] = &[
    "signature.",
    "errors.signature",
    "setup.sends_data",
    "legal.fallback_notice",
];

fn engine_only(key: &str) -> bool {
    ENGINE_ONLY.iter().any(|p| key.starts_with(p))
}

/// Strip the characters that let a string lie about its own shape.
///
/// Explicit bidirectional formatting (`U+202A`..`U+202E`, `U+2066`..`U+2069`) reverses
/// the visual order of what follows it: `https://\u{202e}moc.live` renders as
/// `https://evil.com` backwards, and a filename or a URL next to a legal document then
/// reads as something it is not. The RTL *marks* (`U+200E`/`U+200F`) are left alone — real
/// Arabic and Hebrew text needs them, and they cannot reorder ASCII on their own.
///
/// C0 controls go too (except tab and newline, which the markdown renderer uses): a
/// carriage return in a Slint `Text` run, or a NUL in a path shown mid-progress, is never
/// something a translator wrote.
pub fn sanitize_text(s: &str) -> String {
    s.chars()
        .filter(|c| {
            !matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
                && (!c.is_control() || *c == '\n' || *c == '\t')
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Ltr,
    Rtl,
}

/// One language's strings.
#[derive(Debug, Clone)]
pub struct Catalog {
    pub code: String,
    /// The language's own name for itself, shown in the picker ("Français").
    pub name: Option<String>,
    pub direction: Option<Direction>,
    pub origin: Origin,
    strings: BTreeMap<String, String>,
}

impl Catalog {
    /// Parse a catalogue that came from a package: [`Origin::Product`], and so subject to
    /// [`ENGINE_ONLY`] when it is layered in.
    ///
    /// Untrusted is the DEFAULT on purpose. The trusted constructor is the one that has to
    /// be asked for by name, because there are two catalogues compiled into this binary and
    /// an unbounded number that arrive from outside it; a default that has to be remembered
    /// is the one that gets forgotten by whoever adds the third source.
    pub fn parse(code: &str, text: &str) -> Result<Catalog, String> {
        Self::parse_with(code, text, Origin::Product)
    }

    /// A catalogue that ships inside this binary. Nothing is filtered from it.
    pub fn parse_engine(code: &str, text: &str) -> Result<Catalog, String> {
        Self::parse_with(code, text, Origin::Engine)
    }

    /// Parse a catalogue. `code` comes from the file name, not the contents, so a file
    /// cannot claim to be another language than the one it is stored as.
    fn parse_with(code: &str, text: &str, origin: Origin) -> Result<Catalog, String> {
        let code = normalize(code).ok_or_else(|| format!("{code:?} is not a language code"))?;
        let root: toml::Table = toml::from_str(text).map_err(|e| format!("{code}.toml: {e}"))?;
        let mut cat = Catalog {
            code: code.clone(),
            name: None,
            direction: None,
            origin,
            strings: BTreeMap::new(),
        };
        for (k, v) in root {
            if k == "meta" {
                let meta = v
                    .as_table()
                    .ok_or_else(|| format!("{code}.toml: [meta] must be a table"))?;
                cat.name = meta.get("name").and_then(|n| n.as_str()).map(sanitize_text);
                cat.direction = match meta.get("direction").and_then(|d| d.as_str()) {
                    None => None,
                    Some("ltr") => Some(Direction::Ltr),
                    Some("rtl") => Some(Direction::Rtl),
                    Some(o) => return Err(format!("{code}.toml: direction {o:?} (ltr or rtl)")),
                };
                continue;
            }
            flatten(&code, &k, &v, &mut cat.strings)?;
        }
        Ok(cat)
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.strings.get(key).map(String::as_str)
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.strings.keys().map(String::as_str)
    }

    /// Layer `other` over `self`: its strings win, and so does its meta when it sets one.
    fn merge(&mut self, other: Catalog) {
        if other.name.is_some() {
            self.name = other.name;
        }
        if other.direction.is_some() {
            self.direction = other.direction;
        }
        self.strings.extend(other.strings);
    }
}

fn flatten(
    code: &str,
    prefix: &str,
    v: &toml::Value,
    out: &mut BTreeMap<String, String>,
) -> Result<(), String> {
    match v {
        toml::Value::String(s) => {
            // Sanitised here, at the one door every catalogue string comes through, rather
            // than at each of the dozen places one is handed to Slint.
            out.insert(prefix.to_string(), sanitize_text(s));
            Ok(())
        }
        toml::Value::Table(t) => {
            for (k, v) in t {
                flatten(code, &format!("{prefix}.{k}"), v, out)?;
            }
            Ok(())
        }
        // A number or a list where a sentence was expected is a translator's typo, and
        // dropping it silently would show English with no clue why.
        _ => Err(format!("{code}.toml: {prefix} must be a string")),
    }
}

/// The engine's embedded catalogues.
pub fn builtin_catalogs() -> Vec<Catalog> {
    BUILTIN
        .iter()
        .map(|(code, text)| {
            // build.rs guarantees the name; a parse error here is a broken file in this
            // repository, which the tests catch before anyone ships it.
            Catalog::parse_engine(code, text).unwrap_or_else(|e| panic!("engine catalogue {e}"))
        })
        .collect()
}

/// `fr_FR.UTF-8@euro` → `fr-FR`, `EN` → `en`, `zh-hant-tw` → `zh-Hant-TW`.
/// `None` for the POSIX non-locales (`C`, `POSIX`) and anything that is not a tag.
pub fn normalize(raw: &str) -> Option<String> {
    let s = raw.trim();
    let s = s.split(['.', '@']).next().unwrap_or("");
    if s.is_empty() || s.eq_ignore_ascii_case("c") || s.eq_ignore_ascii_case("posix") {
        return None;
    }
    let mut parts = s.split(['-', '_']);
    let lang = parts.next()?;
    if !(2..=3).contains(&lang.len()) || !lang.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let mut out = lang.to_ascii_lowercase();
    for p in parts {
        if !(2..=8).contains(&p.len()) || !p.chars().all(|c| c.is_ascii_alphanumeric()) {
            return None;
        }
        out.push('-');
        // Conventional casing: 4-letter script Title-case, 2-letter region upper-case.
        if p.len() == 4 && p.chars().all(|c| c.is_ascii_alphabetic()) {
            let mut c = p.chars();
            out.push(c.next().unwrap().to_ascii_uppercase());
            out.push_str(&c.as_str().to_ascii_lowercase());
        } else if p.len() == 2 {
            out.push_str(&p.to_ascii_uppercase());
        } else {
            out.push_str(p);
        }
    }
    Some(out)
}

/// `pt-BR` → `[pt-BR, pt, en]`. Always ends with [`FALLBACK`], never repeats a tag.
pub fn fallback_chain(tag: &str) -> Vec<String> {
    let mut chain: Vec<String> = Vec::new();
    if let Some(t) = normalize(tag) {
        let mut cur = t.as_str();
        loop {
            chain.push(cur.to_string());
            match cur.rfind('-') {
                Some(i) => cur = &cur[..i],
                None => break,
            }
        }
    }
    if !chain.iter().any(|c| c == FALLBACK) {
        chain.push(FALLBACK.to_string());
    }
    chain
}

/// The user's interface language as the OS reports it, normalized. `None` when nothing
/// usable is set, and the caller falls back to English.
pub fn detect_os_locale() -> Option<String> {
    #[cfg(windows)]
    if let Some(l) = windows_ui_locale() {
        return Some(l);
    }
    // POSIX precedence for messages. LANGUAGE is a colon list; its first entry counts.
    ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .filter_map(|v| v.split(':').next().and_then(normalize))
        .next()
}

/// The Windows UI language (what the user reads menus in), not the regional format:
/// someone in Switzerland with an English UI and French date formats wants English.
#[cfg(windows)]
fn windows_ui_locale() -> Option<String> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetUserDefaultUILanguage() -> u16;
        fn LCIDToLocaleName(locale: u32, name: *mut u16, cch_name: i32, flags: u32) -> i32;
    }
    let mut buf = [0u16; 85]; // LOCALE_NAME_MAX_LENGTH
                              // SAFETY: the buffer and its length are passed together; the call writes at most
                              // cch_name UTF-16 units, including the terminator, and returns how many it wrote.
    let n = unsafe {
        let lcid = GetUserDefaultUILanguage() as u32;
        LCIDToLocaleName(lcid, buf.as_mut_ptr(), buf.len() as i32, 0)
    };
    if n <= 1 {
        return None;
    }
    normalize(&String::from_utf16_lossy(&buf[..(n - 1) as usize]))
}

/// A set of merged catalogues plus the current language.
///
/// Plain data (Clone + Send): the UI thread owns one, and a worker thread gets a copy to
/// word its progress messages in the same language.
#[derive(Debug, Clone)]
pub struct Translator {
    catalogs: BTreeMap<String, Catalog>,
    requested: String,
    chain: Vec<String>,
    ctx: BTreeMap<String, String>,
}

impl Default for Translator {
    fn default() -> Self {
        Self::builtin()
    }
}

impl Translator {
    /// The engine catalogues only, in English.
    pub fn builtin() -> Self {
        let mut t = Translator {
            catalogs: BTreeMap::new(),
            requested: FALLBACK.to_string(),
            chain: vec![FALLBACK.to_string()],
            ctx: BTreeMap::new(),
        };
        for c in builtin_catalogs() {
            t.add_catalog(c);
        }
        t
    }

    /// Layer a catalogue over whatever is already loaded for its language.
    ///
    /// A [`Origin::Product`] catalogue loses its [`ENGINE_ONLY`] keys on the way in: it may
    /// translate the installer, it may not use it to make a claim about itself.
    pub fn add_catalog(&mut self, mut c: Catalog) {
        if c.origin == Origin::Product {
            let refused: Vec<String> = c
                .strings
                .keys()
                .filter(|k| engine_only(k))
                .cloned()
                .collect();
            for k in &refused {
                c.strings.remove(k);
                eprintln!("{}.toml: {k} is the engine's to word; ignored", c.code);
            }
        }
        match self.catalogs.get_mut(&c.code) {
            Some(existing) => existing.merge(c),
            None => {
                self.catalogs.insert(c.code.clone(), c);
            }
        }
    }

    /// Switch language. Unknown or unusable tags fall back through the chain, so this
    /// never fails; [`Translator::language`] says what was actually picked.
    pub fn set_language(&mut self, requested: &str) {
        self.requested = normalize(requested).unwrap_or_else(|| FALLBACK.to_string());
        self.chain = fallback_chain(&self.requested);
    }

    /// What the user asked for (may have no catalogue of its own).
    pub fn requested(&self) -> &str {
        &self.requested
    }

    /// The best language that HAS a catalogue: the first of the chain that is loaded.
    pub fn language(&self) -> &str {
        self.chain
            .iter()
            .find(|c| self.catalogs.contains_key(*c))
            .map(String::as_str)
            .unwrap_or(FALLBACK)
    }

    /// Codes to look up, most specific first.
    pub fn chain(&self) -> &[String] {
        &self.chain
    }

    pub fn direction(&self) -> Direction {
        self.chain
            .iter()
            .filter_map(|c| self.catalogs.get(c))
            .find_map(|c| c.direction)
            .unwrap_or(Direction::Ltr)
    }

    /// `(code, native name)` for every loaded language, sorted by code.
    pub fn available(&self) -> Vec<(String, String)> {
        self.catalogs
            .values()
            .map(|c| {
                (
                    c.code.clone(),
                    c.name.clone().unwrap_or_else(|| c.code.clone()),
                )
            })
            .collect()
    }

    /// The native name of a language, or its code when no catalogue names it.
    pub fn language_name(&self, code: &str) -> String {
        fallback_chain(code)
            .iter()
            .filter_map(|c| self.catalogs.get(c))
            .find_map(|c| c.name.clone())
            .unwrap_or_else(|| code.to_string())
    }

    /// A value available to every string as `{name}` (app name, versions…).
    pub fn set_ctx(&mut self, name: &str, value: impl Into<String>) {
        self.ctx.insert(name.to_string(), value.into());
    }

    /// The raw string for `key` along the chain, unformatted. `None` when no language has it.
    pub fn lookup(&self, key: &str) -> Option<&str> {
        self.chain
            .iter()
            .filter_map(|c| self.catalogs.get(c))
            .find_map(|c| c.get(key))
    }

    /// Like [`lookup`](Self::lookup), but says which language answered.
    pub fn lookup_with_lang(&self, key: &str) -> Option<(&str, &str)> {
        self.chain
            .iter()
            .filter_map(|c| self.catalogs.get(c))
            .find_map(|c| c.get(key).map(|s| (s, c.code.as_str())))
    }

    /// Translate and fill the shared context. A missing key returns the key.
    pub fn t(&self, key: &str) -> String {
        self.t_with(key, &[])
    }

    /// Translate, filling `args` first and then the shared context.
    pub fn t_with(&self, key: &str, args: &[(&str, &str)]) -> String {
        let raw = self.lookup(key).unwrap_or(key);
        self.fill(raw, args)
    }

    /// Fill `{name}` placeholders in an already-chosen string. Unknown ones stay as
    /// written, so a typo in a catalogue shows up on screen instead of vanishing.
    pub fn fill(&self, raw: &str, args: &[(&str, &str)]) -> String {
        let mut s = raw.to_string();
        for (k, v) in args {
            s = s.replace(&format!("{{{k}}}"), v);
        }
        for (k, v) in &self.ctx {
            s = s.replace(&format!("{{{k}}}"), v);
        }
        s
    }

    /// The first entry of the chain found among `choices` (case-insensitive), e.g. to
    /// resolve a product's "auto" language setting to the one the installer is shown in.
    pub fn resolve_among(&self, choices: &[String]) -> Option<String> {
        self.chain.iter().find_map(|c| {
            choices
                .iter()
                .find(|ch| ch.eq_ignore_ascii_case(c))
                .cloned()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_normalized() {
        assert_eq!(normalize("fr_FR.UTF-8").as_deref(), Some("fr-FR"));
        assert_eq!(normalize("de_CH.utf8@euro").as_deref(), Some("de-CH"));
        assert_eq!(normalize("EN").as_deref(), Some("en"));
        assert_eq!(normalize("zh-hant-tw").as_deref(), Some("zh-Hant-TW"));
        assert_eq!(normalize("C"), None);
        assert_eq!(normalize("POSIX"), None);
        assert_eq!(normalize(""), None);
        assert_eq!(normalize("../../etc"), None);
        assert_eq!(normalize("x"), None);
    }

    #[test]
    fn the_chain_narrows_then_ends_in_english() {
        assert_eq!(fallback_chain("pt-BR"), ["pt-BR", "pt", "en"]);
        assert_eq!(
            fallback_chain("zh-Hant-TW"),
            ["zh-Hant-TW", "zh-Hant", "zh", "en"]
        );
        assert_eq!(fallback_chain("en-GB"), ["en-GB", "en"]);
        assert_eq!(fallback_chain("en"), ["en"]);
        assert_eq!(fallback_chain("nonsense tag"), ["en"]);
    }

    #[test]
    fn a_regional_request_uses_the_base_language() {
        let mut t = Translator::builtin();
        t.set_language("fr-CH");
        assert_eq!(t.language(), "fr");
        assert_eq!(t.t("ui.next"), "Suivant");
        // No catalogue at all for German: English, and the request is remembered.
        t.set_language("de-DE");
        assert_eq!(t.language(), "en");
        assert_eq!(t.requested(), "de-DE");
        assert_eq!(t.t("ui.next"), "Next");
    }

    #[test]
    fn a_missing_key_falls_back_to_english_then_to_itself() {
        let mut t = Translator::builtin();
        t.add_catalog(Catalog::parse("fr", "[ui]\nnext = \"Suivant\"").unwrap());
        t.add_catalog(Catalog::parse("en", "[only]\nenglish = \"E\"").unwrap());
        t.set_language("fr");
        assert_eq!(t.t("only.english"), "E");
        assert_eq!(t.t("totally.unknown"), "totally.unknown");
    }

    #[test]
    fn a_product_catalogue_adds_a_language_and_overrides_engine_strings() {
        let mut t = Translator::builtin();
        t.add_catalog(
            Catalog::parse(
                "de",
                "[meta]\nname = \"Deutsch\"\n[ui]\nnext = \"Weiter\"\n[options.x]\nlabel = \"X\"",
            )
            .unwrap(),
        );
        t.add_catalog(Catalog::parse("en", "[ui]\nnext = \"Continue\"").unwrap());
        t.set_language("de-AT");
        assert_eq!(t.language(), "de");
        assert_eq!(t.t("ui.next"), "Weiter");
        // Not in German: the PRODUCT's English override, not the engine's.
        t.set_language("de");
        assert_eq!(t.t("ui.back"), "Back");
        t.set_language("en");
        assert_eq!(t.t("ui.next"), "Continue");
        assert!(t
            .available()
            .iter()
            .any(|(c, n)| c == "de" && n == "Deutsch"));
        // Merging keeps the engine's name for a language the product did not rename.
        assert_eq!(t.language_name("fr"), "Français");
    }

    #[test]
    fn placeholders_take_arguments_then_context() {
        let mut t = Translator::builtin();
        t.set_ctx("app", "BMM");
        assert_eq!(t.t("welcome.title"), "Install BMM");
        assert_eq!(
            t.t_with("done.installed_message", &[("n", "3"), ("dir", "C:/x")]),
            "Installed 3 files to C:/x"
        );
        assert_eq!(t.fill("{unknown} {app}", &[]), "{unknown} BMM");
    }

    #[test]
    fn rtl_is_read_from_the_catalogue() {
        let mut t = Translator::builtin();
        t.add_catalog(
            Catalog::parse("ar", "[meta]\nname = \"العربية\"\ndirection = \"rtl\"").unwrap(),
        );
        t.set_language("ar-EG");
        assert_eq!(t.direction(), Direction::Rtl);
        t.set_language("fr");
        assert_eq!(t.direction(), Direction::Ltr);
    }

    #[test]
    fn a_bad_catalogue_is_refused_with_a_reason() {
        assert!(Catalog::parse("fr", "[ui]\nnext = 3").is_err());
        assert!(Catalog::parse("fr", "[meta]\ndirection = \"up\"").is_err());
        assert!(Catalog::parse("not a code", "").is_err());
    }

    #[test]
    fn resolving_a_product_choice_follows_the_chain() {
        let mut t = Translator::builtin();
        let choices = vec!["auto".to_string(), "en".to_string(), "fr".to_string()];
        t.set_language("fr-BE");
        assert_eq!(t.resolve_among(&choices).as_deref(), Some("fr"));
        t.set_language("ja");
        assert_eq!(t.resolve_among(&choices).as_deref(), Some("en"));
    }

    /// Every engine catalogue parses, names itself, and uses only keys English has.
    /// Missing keys are allowed (they fall back) but printed, so a partial translation
    /// is a visible, deliberate state.
    #[test]
    fn engine_catalogues_are_well_formed() {
        let cats = builtin_catalogs();
        let en = cats.iter().find(|c| c.code == FALLBACK).expect("en.toml");
        for c in &cats {
            assert!(c.name.is_some(), "{}.toml has no [meta] name", c.code);
            for k in c.keys() {
                assert!(
                    en.get(k).is_some(),
                    "{}.toml: {k} is not an English key",
                    c.code
                );
            }
            let missing: Vec<_> = en.keys().filter(|k| c.get(k).is_none()).collect();
            if !missing.is_empty() {
                println!("{}.toml falls back to English for: {missing:?}", c.code);
            }
        }
    }

    /// French ships complete: the BMM installer is released in English and French.
    #[test]
    fn french_is_complete() {
        let cats = builtin_catalogs();
        let en = cats.iter().find(|c| c.code == "en").unwrap();
        let fr = cats.iter().find(|c| c.code == "fr").expect("fr.toml");
        let missing: Vec<_> = en.keys().filter(|k| fr.get(k).is_none()).collect();
        assert!(missing.is_empty(), "fr.toml is missing {missing:?}");
    }

    /// A package's own catalogue must not be able to word the badge that says whether
    /// that package can be trusted. `Trust::may_read` hands an UNSIGNED package's
    /// catalogues to the translator — deliberately, so the screen that has to say
    /// "unsigned" is not the one screen left in English — and every one of these keys was
    /// then the package's to rewrite. The Welcome page read "Signed & verified" for a
    /// package carrying no signature at all.
    #[test]
    fn a_product_catalogue_cannot_reword_the_engine_verdict() {
        let mut t = Translator::builtin();
        t.add_catalog(
            Catalog::parse(
                "en",
                "[signature]\nunsigned = \"Signed & verified\"\ninvalid = \"All good\"\n\
                 [errors]\nsignature_invalid = \"Installed successfully.\"\n\
                 [setup]\nsends_data = \"\"\n\
                 [legal]\nfallback_notice = \"\"\n\
                 [ui]\nnext = \"Continue\"\n",
            )
            .unwrap(),
        );
        t.set_language("en");
        assert!(t.t("signature.unsigned").starts_with("Unsigned package"));
        assert!(t.t("signature.invalid").contains("INVALID"));
        assert!(t.t("errors.signature_invalid").contains("INVALID"));
        assert_eq!(t.t("setup.sends_data"), "Sends data");
        assert!(t.t("legal.fallback_notice").contains("{language}"));
        // …and everything else still translates, which is the point of the layer.
        assert_eq!(t.t("ui.next"), "Continue");
    }

    /// The same keys in a language the engine does not speak: the chain must reach the
    /// ENGINE's English rather than stop at the product's wording of the verdict.
    #[test]
    fn the_verdict_survives_a_language_the_engine_does_not_speak() {
        let mut t = Translator::builtin();
        t.add_catalog(
            Catalog::parse(
                "de",
                "[meta]\nname = \"Deutsch\"\n[signature]\nunsigned = \"Signiert und geprueft\"\n",
            )
            .unwrap(),
        );
        t.set_language("de");
        assert_eq!(t.language(), "de");
        assert!(t.t("signature.unsigned").starts_with("Unsigned package"));
    }

    /// An engine catalogue is not filtered: the restriction is about where a string came
    /// from, not about the key being untouchable.
    #[test]
    fn an_engine_catalogue_may_word_its_own_verdict() {
        let mut t = Translator::builtin();
        t.add_catalog(
            Catalog::parse_engine("en", "[signature]\nunsigned = \"Not signed\"").unwrap(),
        );
        t.set_language("en");
        assert_eq!(t.t("signature.unsigned"), "Not signed");
    }

    /// A right-to-left OVERRIDE inside a string reverses what follows it on screen, so a
    /// URL or a filename can be made to read as another one. Real RTL languages need the
    /// MARKS (200E/200F), not the overrides, so only the overrides go.
    #[test]
    fn bidi_overrides_and_controls_are_stripped_from_catalogue_strings() {
        let c = Catalog::parse(
            "ar",
            "[meta]\nname = \"\u{202e}Arabic\"\ndirection = \"rtl\"\n\
             [ui]\nnext = \"a\u{202e}b\u{2069}c\\u0007d\"\n\
             [legal]\naccept = \"\u{200f}\u{627}\"\n",
        )
        .unwrap();
        assert_eq!(c.get("ui.next"), Some("abcd"));
        assert_eq!(c.name.as_deref(), Some("Arabic"));
        // The mark a real Arabic string needs is untouched.
        assert_eq!(c.get("legal.accept"), Some("\u{200f}\u{627}"));
        assert_eq!(c.direction, Some(Direction::Rtl));
    }

    #[test]
    fn detection_never_returns_an_unusable_tag() {
        if let Some(l) = detect_os_locale() {
            assert_eq!(normalize(&l).as_deref(), Some(l.as_str()));
        }
    }
}
