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

use eframe::egui;

use app::{StartupFlags, WpmApp};
use settings::Settings;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    let settings = Settings::load();
    let flags = StartupFlags {
        demo: has("--demo"),
        minimized: has("--minimized") || (has("--autostart") && settings.start_minimized),
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
