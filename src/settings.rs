//! Einstellungen, gespeichert als JSON unter `%APPDATA%\WPM\settings.json`.

use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

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
            Self::Default => "Standard (Installer zeigen Fortschritt)",
            Self::Silent => "Still – ohne Installer-Fenster (empfohlen)",
            Self::Interactive => "Interaktiv (mit allen Dialogen)",
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
            Self::Any => "Automatisch",
            Self::User => "Nur aktueller Benutzer",
            Self::Machine => "Alle Benutzer (System)",
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
            Self::All => "Alle Quellen",
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
            Self::System => "Wie Windows",
            Self::Light => "Hell",
            Self::Dark => "Dunkel",
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
    /// Beim Autostart nur in der Taskleiste starten.
    pub start_minimized: bool,
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
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            autostart: false,
            start_minimized: true,
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
        let path =
            Self::path().ok_or_else(|| io::Error::other("Kein Konfigurationsordner gefunden"))?;
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
    }
}
