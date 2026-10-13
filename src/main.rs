// Im Release-Build ohne zusätzliches Konsolenfenster starten.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod icon;
mod runner;
mod scheduler;
mod settings;
mod system;
mod tray;
mod winget;
mod worker;

use std::time::Duration;

use eframe::egui;

use app::{StartupFlags, WpmApp};
use settings::Settings;

/// Markiert den mit Adminrechten neu gestarteten Prozess.
const ELEVATED_FLAG: &str = "--elevated";
/// Konto, das den Neustart angestoßen hat (zum Erkennen fremder Admin-Konten).
const USER_SID_FLAG: &str = "--user-sid=";
/// Paket, dessen Update nach dem Neustart mit Adminrechten fortgesetzt wird.
const UPGRADE_FLAG: &str = "--upgrade=";
/// Unsichtbar im Infobereich starten, nachdem die Instanz mit Adminrechten
/// geschlossen wurde.
const BACKGROUND_FLAG: &str = "--background";

fn main() -> eframe::Result {
    system::harden_dll_search();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    let settings = Settings::load();
    let demo = has("--demo");
    let autostart = has("--autostart");
    let background = has(BACKGROUND_FLAG);
    let relaunched = has(ELEVATED_FLAG) || background;

    // Nur eine Instanz: Autostart und manueller Start sollen sich nicht doppeln.
    // Nach einem Neustart kurz warten, bis die alte Instanz beendet ist.
    let _instance = if demo {
        None
    } else {
        let wait = if relaunched {
            Duration::from_secs(15)
        } else {
            Duration::ZERO
        };
        match system::SingleInstance::acquire(wait) {
            Some(guard) => Some(guard),
            None => {
                // Ein erneuter Start holt das Fenster der laufenden Instanz
                // nach vorn – auch aus dem Infobereich.
                if !autostart && !background && !system::signal_running_instance() {
                    system::message_box(
                        "WPM",
                        "WPM läuft bereits – das Fenster ist eventuell in der Taskleiste \
                         minimiert oder im Infobereich (unten rechts neben der Uhr).",
                    );
                }
                return Ok(());
            }
        }
    };

    // Läuft WPM nach dem Neustart unter einem anderen Konto als vorher? Das
    // passiert, wenn bei der Nachfrage andere Zugangsdaten eingegeben werden
    // oder der Windows-Administratorschutz ein eigenes Admin-Konto verwendet.
    let other_account = args
        .iter()
        .find_map(|a| a.strip_prefix(USER_SID_FLAG))
        .is_some_and(|expected| system::user_sid().is_some_and(|sid| sid != expected));

    let flags = StartupFlags {
        demo,
        minimized: has("--minimized") || background || (autostart && settings.start_minimized),
        background,
        elevated: system::is_elevated(),
        other_account,
        resume_upgrades: args
            .iter()
            .filter_map(|a| a.strip_prefix(UPGRADE_FLAG))
            .map(str::to_string)
            .collect(),
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("WPM – Windows-Paketmanager")
            .with_app_id("wpm")
            .with_inner_size([1100.0, 700.0])
            .with_min_inner_size([760.0, 420.0])
            .with_icon(icon::app_icon())
            // eframe zeigt das Fenster nach dem ersten Bild kurz an, bevor WPM es
            // versteckt bzw. minimiert – dabei soll es keinen Fokus stehlen.
            .with_active(!flags.minimized),
        ..Default::default()
    };
    eframe::run_native(
        "WPM",
        options,
        Box::new(move |cc| Ok(Box::new(WpmApp::new(cc, settings, flags)))),
    )
}

/// Argumente für den Neustart mit Adminrechten, der die Updates der
/// angegebenen Pakete fortsetzt.
pub fn relaunch_args(demo: bool, upgrade_ids: &[String]) -> Vec<String> {
    let mut args = vec![ELEVATED_FLAG.to_string()];
    if let Some(sid) = system::user_sid() {
        args.push(format!("{USER_SID_FLAG}{sid}"));
    }
    if demo {
        args.push("--demo".into());
    }
    args.extend(upgrade_ids.iter().map(|id| format!("{UPGRADE_FLAG}{id}")));
    args
}

/// Argumente für den Start ohne Adminrechte im Infobereich, wenn die Instanz
/// mit Adminrechten geschlossen wird.
pub fn background_args(demo: bool) -> Vec<String> {
    let mut args = vec![BACKGROUND_FLAG.to_string()];
    if demo {
        args.push("--demo".into());
    }
    args
}
