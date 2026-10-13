// Im Release-Build ohne zusätzliches Konsolenfenster starten.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod icon;
mod runner;
mod scheduler;
mod settings;
mod system;
mod winget;
mod worker;

use std::time::Duration;

use eframe::egui;

use app::{StartupFlags, WpmApp};
use settings::Settings;

/// Markiert den mit Adminrechten neu gestarteten Prozess.
const ELEVATED_FLAG: &str = "--elevated";

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    let settings = Settings::load();
    let demo = has("--demo");
    let autostart = has("--autostart");
    let relaunched = has(ELEVATED_FLAG);

    // Administrator-Modus: vor dem ersten Fenster mit Adminrechten neu starten.
    // Lehnt man die Windows-Bestätigung ab, läuft WPM normal weiter.
    let elevated = system::is_elevated();
    let mut elevation_declined = false;
    if settings.run_as_admin && !elevated && !relaunched && system::ELEVATION_SUPPORTED {
        let mut elevated_args = args.clone();
        elevated_args.push(ELEVATED_FLAG.into());
        match system::relaunch_elevated(&elevated_args, None) {
            Ok(()) => return Ok(()),
            Err(_) => elevation_declined = true,
        }
    }
    // Nur eine Instanz: Autostart und manueller Start sollen sich nicht doppeln.
    // Nach einem Neustart mit Adminrechten kurz warten, bis die alte Instanz zu ist.
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
                if !autostart {
                    system::message_box(
                        "WPM",
                        "WPM läuft bereits – das Fenster ist eventuell in der Taskleiste minimiert.",
                    );
                }
                return Ok(());
            }
        }
    };

    let flags = StartupFlags {
        demo,
        minimized: has("--minimized") || (autostart && settings.start_minimized),
        elevated,
        elevation_declined,
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("WPM – Windows-Paketmanager")
            .with_app_id("wpm")
            .with_inner_size([1100.0, 700.0])
            .with_min_inner_size([760.0, 420.0])
            .with_icon(icon::app_icon()),
        ..Default::default()
    };
    eframe::run_native(
        "WPM",
        options,
        Box::new(move |cc| Ok(Box::new(WpmApp::new(cc, settings, flags)))),
    )
}
