//! Sprache der Oberfläche: Deutsch oder Englisch.
//!
//! Texte stehen direkt im Code, jeweils auf Deutsch und Englisch:
//! `tr!("Suchen", "Search")` für feste Texte, `trf!("{n} Pakete", "{n} packages")`
//! für Texte mit Platzhaltern (wie `format!`). Die Sprache gilt für das ganze
//! Programm, auch für Threads; ein Wechsel in den Einstellungen wirkt sofort.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    De,
    En,
}

/// Bis zum Aufruf von [`set`] (beim Start) gilt Deutsch – auch in Tests.
static ENGLISH: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
thread_local! {
    static OVERRIDE: std::cell::Cell<Option<Lang>> = const { std::cell::Cell::new(None) };
}

pub fn set(lang: Lang) {
    ENGLISH.store(lang == Lang::En, Ordering::Relaxed);
}

pub fn current() -> Lang {
    #[cfg(test)]
    if let Some(lang) = OVERRIDE.with(std::cell::Cell::get) {
        return lang;
    }
    if ENGLISH.load(Ordering::Relaxed) {
        Lang::En
    } else {
        Lang::De
    }
}

pub fn english() -> bool {
    current() == Lang::En
}

/// Führt `f` im aktuellen Thread mit der angegebenen Sprache aus (für Tests,
/// die parallel laufen und sich die globale Einstellung nicht teilen sollen).
#[cfg(test)]
pub fn with<R>(lang: Lang, f: impl FnOnce() -> R) -> R {
    let before = OVERRIDE.with(|o| o.replace(Some(lang)));
    let result = f();
    OVERRIDE.with(|o| o.set(before));
    result
}

/// Anzeigesprache von Windows: Deutsch bei deutscher Oberfläche (auch
/// Österreich, Schweiz …), sonst Englisch.
pub fn system() -> Lang {
    static SYSTEM: OnceLock<Lang> = OnceLock::new();
    *SYSTEM.get_or_init(detect)
}

#[cfg(windows)]
fn detect() -> Lang {
    use windows_sys::Win32::Globalization::GetUserDefaultUILanguage;
    /// Primärsprache in den unteren 10 Bit der LANGID.
    const LANG_GERMAN: u16 = 0x07;
    // SAFETY: Ohne Parameter, liefert nur einen Wert.
    let id = unsafe { GetUserDefaultUILanguage() };
    if id & 0x3FF == LANG_GERMAN {
        Lang::De
    } else {
        Lang::En
    }
}

#[cfg(not(windows))]
fn detect() -> Lang {
    let tag = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|v| !v.is_empty()))
        .unwrap_or_default();
    from_tag(&tag)
}

/// Sprache aus Angaben wie `de_DE.UTF-8` oder `en-US`.
#[cfg_attr(windows, allow(dead_code))]
fn from_tag(tag: &str) -> Lang {
    if tag.to_lowercase().starts_with("de") {
        Lang::De
    } else {
        Lang::En
    }
}

/// Für [`tr!`]: Feste Texte dürfen keine Platzhalter enthalten, sonst stünde
/// `{name}` wörtlich in der Oberfläche.
pub const fn has_braces(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' || bytes[i] == b'}' {
            return true;
        }
        i += 1;
    }
    false
}

/// Fester Text auf Deutsch und Englisch, ergibt `&'static str`.
macro_rules! tr {
    ($de:literal, $en:literal $(,)?) => {{
        const _: () = assert!(
            !crate::i18n::has_braces($de) && !crate::i18n::has_braces($en),
            "tr! ohne Platzhalter – für Texte mit Platzhaltern trf! verwenden"
        );
        if crate::i18n::english() { $en } else { $de }
    }};
}

/// Text mit Platzhaltern auf Deutsch und Englisch, wie `format!`; ergibt
/// `String`. Beide Fassungen müssen dieselben Argumente verwenden.
macro_rules! trf {
    ($de:literal, $en:literal $(, $($args:tt)*)?) => {
        if crate::i18n::english() {
            format!($en $(, $($args)*)?)
        } else {
            format!($de $(, $($args)*)?)
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn german_until_set_and_override_per_thread() {
        assert_eq!(current(), Lang::De);
        assert_eq!(tr!("Suchen", "Search"), "Suchen");
        with(Lang::En, || {
            assert!(english());
            assert_eq!(tr!("Suchen", "Search"), "Search");
            let n = 3;
            assert_eq!(trf!("{n} Pakete", "{n} packages"), "3 packages");
            assert_eq!(trf!("{} von {}", "{} of {}", 1, n), "1 of 3");
        });
        assert_eq!(current(), Lang::De);
        let n = 2;
        assert_eq!(trf!("{n} Pakete", "{n} packages"), "2 Pakete");
    }

    #[test]
    fn language_tags() {
        assert_eq!(from_tag("de_DE.UTF-8"), Lang::De);
        assert_eq!(from_tag("de-AT"), Lang::De);
        assert_eq!(from_tag("en_US.UTF-8"), Lang::En);
        assert_eq!(from_tag("fr_FR"), Lang::En);
        assert_eq!(from_tag(""), Lang::En);
        assert_eq!(from_tag("C"), Lang::En);
    }

    #[test]
    fn braces_are_detected() {
        assert!(has_braces("{n} Pakete"));
        assert!(!has_braces("Suchen"));
    }
}
