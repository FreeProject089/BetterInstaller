//! Minimal compiled-in i18n for the installer chrome. `t(lang, key)` returns the
//! string for `lang` (currently "en"/"fr") with English fallback. The richer
//! plan (JSON translation files, more languages) builds on this.

/// Best-effort OS UI language → "en" | "fr".
pub fn detect_lang() -> String {
    #[cfg(windows)]
    {
        // GetUserDefaultUILanguage would be more precise; the LANG env var is a
        // good-enough, dependency-free heuristic and matches the non-Windows path.
    }
    let raw = std::env::var("LANG")
        .or_else(|_| std::env::var("LC_ALL"))
        .or_else(|_| std::env::var("LANGUAGE"))
        .unwrap_or_default()
        .to_lowercase();
    if raw.starts_with("fr") {
        "fr".to_string()
    } else {
        "en".to_string()
    }
}

/// Translate a UI key. Unknown keys return the key itself.
pub fn t(lang: &str, key: &str) -> String {
    let fr = lang == "fr";
    let s = match key {
        "next" => {
            if fr {
                "Suivant"
            } else {
                "Next"
            }
        }
        "back" => {
            if fr {
                "Retour"
            } else {
                "Back"
            }
        }
        "install" => {
            if fr {
                "Installer"
            } else {
                "Install"
            }
        }
        "finish" => {
            if fr {
                "Terminer"
            } else {
                "Finish"
            }
        }
        "config_title" => "Configuration", // same in en/fr
        "config_hint" => {
            if fr {
                "Ces choix sont appliqués au premier lancement — aucune configuration dans l'application."
            } else {
                "These choices are applied on first launch — no setup needed inside the app."
            }
        }
        "install_loc" => {
            if fr {
                "Dossier d'installation"
            } else {
                "Install location"
            }
        }
        "installing" => {
            if fr {
                "Installation…"
            } else {
                "Installing…"
            }
        }
        "accept" => {
            if fr {
                "J'accepte"
            } else {
                "I accept"
            }
        }
        _ => key,
    };
    s.to_string()
}
