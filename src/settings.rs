//! Einstellungen, gespeichert als JSON unter `%APPDATA%\WPM\settings.json`.

use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::i18n::{self, Lang};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum InstallMode {
    /// winget entscheidet (meist mit Fortschrittsfenster des Installers).
    Default,
    /// `--silent`: keine Installer-Fenster.
    #[default]
    Silent,
    /// `--interactive`: Installer-Dialoge anzeigen.
    Interactive,
}

impl InstallMode {
    pub const ALL: [Self; 3] = [Self::Silent, Self::Default, Self::Interactive];

    pub fn label(self) -> &'static str {
        match self {
            Self::Default => tr!(
                "Standard (Installer zeigen Fortschritt)",
                "Default (installers show progress)"
            ),
            Self::Silent => tr!(
                "Still – ohne Installer-Fenster (empfohlen)",
                "Silent – no installer windows (recommended)"
            ),
            Self::Interactive => tr!(
                "Interaktiv (mit allen Dialogen)",
                "Interactive (with all dialogs)"
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scope {
    #[default]
    Any,
    User,
    Machine,
}

impl Scope {
    pub const ALL: [Self; 3] = [Self::Any, Self::User, Self::Machine];

    pub fn label(self) -> &'static str {
        match self {
            Self::Any => tr!("Automatisch", "Automatic"),
            Self::User => tr!("Nur aktueller Benutzer", "Current user only"),
            Self::Machine => tr!("Alle Benutzer (System)", "All users (machine)"),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceFilter {
    #[default]
    All,
    Winget,
    MsStore,
}

impl SourceFilter {
    pub const ALL: [Self; 3] = [Self::All, Self::Winget, Self::MsStore];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => tr!("Alle Quellen", "All sources"),
            Self::Winget => "winget",
            Self::MsStore => "Microsoft Store (msstore)",
        }
    }

    pub fn source_name(self) -> Option<&'static str> {
        match self {
            Self::All => None,
            Self::Winget => Some("winget"),
            Self::MsStore => Some("msstore"),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

impl Theme {
    pub const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    pub fn label(self) -> &'static str {
        match self {
            Self::System => tr!("Wie Windows", "Same as Windows"),
            Self::Light => tr!("Hell", "Light"),
            Self::Dark => tr!("Dunkel", "Dark"),
        }
    }

    pub fn to_egui(self) -> eframe::egui::ThemePreference {
        use eframe::egui::ThemePreference;
        match self {
            Self::System => ThemePreference::System,
            Self::Light => ThemePreference::Light,
            Self::Dark => ThemePreference::Dark,
        }
    }
}

/// Sprache der Oberfläche.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    /// Wie die Anzeigesprache von Windows (Deutsch oder sonst Englisch).
    #[default]
    System,
    German,
    English,
}

impl Language {
    pub const ALL: [Self; 3] = [Self::System, Self::German, Self::English];

    pub fn label(self) -> &'static str {
        match (self, i18n::system()) {
            (Self::System, Lang::De) => tr!("Wie Windows (Deutsch)", "Same as Windows (German)"),
            (Self::System, Lang::En) => tr!("Wie Windows (Englisch)", "Same as Windows (English)"),
            // Jede Sprache in ihrer eigenen Schreibweise, damit man sie findet.
            (Self::German, _) => "Deutsch",
            (Self::English, _) => "English",
        }
    }

    pub fn resolve(self) -> Lang {
        match self {
            Self::System => i18n::system(),
            Self::German => Lang::De,
            Self::English => Lang::En,
        }
    }
}

/// Version des Einstellungsformats, für einmalige Anpassungen alter Dateien.
const SETTINGS_VERSION: u32 = 2;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Fehlt in Dateien von WPM 0.1 und wird dann als 0 gelesen.
    #[serde(default)]
    pub version: u32,
    /// Mit Windows starten (Eintrag unter `HKCU\…\Run`).
    pub autostart: bool,
    /// Verknüpfung „WPM“ im Startmenü. Wie `autostart` gibt das den
    /// tatsächlichen Zustand wieder, nicht nur einen Wunsch.
    pub start_menu: bool,
    /// Über winget installiert wurde die Verknüpfung schon einmal automatisch
    /// angelegt; danach entscheidet nur noch die Einstellung.
    pub start_menu_offered: bool,
    /// Beim Autostart nur im Infobereich bzw. minimiert in der Taskleiste starten.
    pub start_minimized: bool,
    /// Symbol im Infobereich der Taskleiste anzeigen.
    pub tray_icon: bool,
    /// Beim Schließen des Fensters im Infobereich weiterlaufen.
    pub close_to_tray: bool,
    /// Der Hinweis „WPM läuft im Infobereich weiter“ wurde schon gezeigt.
    pub background_hint_shown: bool,
    /// WPM mit Adminrechten starten, damit Installer nicht einzeln nachfragen.
    pub run_as_admin: bool,
    pub check_on_startup: bool,
    pub auto_check: bool,
    pub check_interval_hours: u32,
    /// Windows-Benachrichtigung, wenn die automatische Prüfung neue Updates findet.
    pub notify: bool,
    /// `--include-unknown`: auch Pakete ohne erkennbare Version anzeigen.
    pub include_unknown: bool,
    pub install_mode: InstallMode,
    pub scope: Scope,
    pub search_source: SourceFilter,
    pub confirm_uninstall: bool,
    /// Programme ohne winget-Quelle (z. B. Treiber, Systemkomponenten) ausblenden.
    pub hide_unmanaged: bool,
    pub theme: Theme,
    pub language: Language,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            autostart: false,
            start_menu: false,
            start_menu_offered: false,
            start_minimized: true,
            tray_icon: true,
            close_to_tray: true,
            background_hint_shown: false,
            run_as_admin: false,
            check_on_startup: true,
            auto_check: true,
            check_interval_hours: 6,
            notify: true,
            include_unknown: false,
            install_mode: InstallMode::Silent,
            scope: Scope::Any,
            search_source: SourceFilter::All,
            confirm_uninstall: true,
            hide_unmanaged: false,
            theme: Theme::System,
            language: Language::System,
        }
    }
}

impl Settings {
    pub fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|dir| dir.join("WPM").join("settings.json"))
    }

    /// Lädt die Einstellungen; fehlende oder kaputte Dateien ergeben die Standardwerte.
    pub fn load() -> Self {
        Self::path()
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|text| Self::from_json(&text))
            .unwrap_or_default()
    }

    fn from_json(text: &str) -> Option<Self> {
        let mut settings: Self = serde_json::from_str(text).ok()?;
        settings.migrate();
        Some(settings)
    }

    /// Passt Einstellungen älterer Versionen an.
    fn migrate(&mut self) {
        if self.version < 2 {
            // WPM 0.1 hatte „Standard“ als Vorgabe; „Still“ vermeidet die meisten
            // Installer-Fenster und ist jetzt die Vorgabe.
            if self.install_mode == InstallMode::Default {
                self.install_mode = InstallMode::Silent;
            }
        }
        self.version = SETTINGS_VERSION;
    }

    pub fn save(&self) -> io::Result<()> {
        let path = Self::path().ok_or_else(|| {
            io::Error::other(tr!(
                "Kein Konfigurationsordner gefunden",
                "No configuration folder found"
            ))
        })?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        fs::write(path, json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_from_version_0_1_switch_to_silent_installs() {
        let old = Settings::from_json(r#"{"autostart": true, "install_mode": "Default"}"#).unwrap();
        assert_eq!(old.install_mode, InstallMode::Silent);
        assert_eq!(old.version, SETTINGS_VERSION);

        // Eine bewusste Wahl in einer neuen Datei bleibt erhalten.
        let chosen = Settings {
            install_mode: InstallMode::Default,
            ..Settings::default()
        };
        let json = serde_json::to_string(&chosen).unwrap();
        assert_eq!(
            Settings::from_json(&json).unwrap().install_mode,
            InstallMode::Default
        );
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let settings: Settings = serde_json::from_str(r#"{"autostart": true}"#).unwrap();
        assert!(settings.autostart);
        assert_eq!(settings.check_interval_hours, 6);
        // Ältere Dateien kennen den Infobereich noch nicht.
        assert!(settings.tray_icon && settings.close_to_tray);
        assert!(!settings.background_hint_shown);
        // Ältere Dateien kennen die Sprache nicht: wie Windows.
        assert_eq!(settings.language, Language::System);
    }

    #[test]
    fn language_choice_is_saved_and_resolved() {
        let settings = Settings {
            language: Language::English,
            ..Settings::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        assert!(json.contains(r#""language":"English""#));
        assert_eq!(
            Settings::from_json(&json).unwrap().language.resolve(),
            Lang::En
        );
        assert_eq!(Language::German.resolve(), Lang::De);
        assert_eq!(Language::System.resolve(), i18n::system());
    }
}
