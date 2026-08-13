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
        // Titles for the bundled legal documents. The BODY is already localized (the
        // installer prefers TOS_FR.md / PRIVACY_FR.md when the language is French), so an
        // English heading over French text was the last thing left in the wrong language.
        // The theme picker offered once more on the final page. These were hardcoded
        // English in main.slint and never set from Rust, so a French install showed
        // them untranslated next to fully translated buttons.
        "final_apply" => {
            if fr {
                "Appliquer"
            } else {
                "Apply"
            }
        }
        "final_skip" => {
            if fr {
                "Garder l'actuel"
            } else {
                "Keep current"
            }
        }
        "scroll_to_accept" => {
            if fr {
                "Fais défiler jusqu'à la fin pour accepter."
            } else {
                "Scroll to the end to accept."
            }
        }
        "doc_privacy" => {
            if fr {
                "Politique de confidentialité"
            } else {
                "Privacy Policy"
            }
        }
        "doc_tos" => {
            if fr {
                "Conditions d'utilisation"
            } else {
                "Terms of Service"
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

#[cfg(test)]
mod tests {
    use super::{detect_lang, t};

    #[test]
    fn translates_and_falls_back() {
        assert_eq!(t("fr", "next"), "Suivant");
        assert_eq!(t("en", "next"), "Next");
        assert_eq!(t("fr", "install"), "Installer");
        // Non-fr locales collapse to English.
        assert_eq!(t("de", "back"), "Back");
        // A key with no per-language variant is identical in both.
        assert_eq!(t("fr", "config_title"), "Configuration");
        assert_eq!(t("en", "config_title"), "Configuration");
        // Unknown keys return the key itself (never an empty string).
        assert_eq!(t("fr", "totally_unknown_key"), "totally_unknown_key");
        assert_eq!(t("en", ""), "");
    }

    #[test]
    fn detect_lang_is_always_a_known_code() {
        // Reads process env; assert only the invariant (never flaky under parallel tests).
        let l = detect_lang();
        assert!(l == "en" || l == "fr", "unexpected detected lang: {l:?}");
    }
}
