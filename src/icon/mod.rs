//! Programmsymbol für Fenster und Infobereich, zur Laufzeit gezeichnet. Das
//! Symbol der EXE (Explorer, Startmenü) entsteht beim Bauen aus derselben
//! Vorlage, siehe `build.rs`.

mod draw;

use eframe::egui::IconData;

const SIZE: u32 = 64;

pub fn app_icon() -> IconData {
    IconData {
        rgba: draw::rgba(SIZE, false),
        width: SIZE,
        height: SIZE,
    }
}

/// Symbol für den Infobereich; `badge` ergänzt einen orangen Punkt
/// („Updates verfügbar“). Liefert RGBA-Daten und die Kantenlänge.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn tray_icon(badge: bool) -> (Vec<u8>, u32) {
    (draw::rgba(SIZE, badge), SIZE)
}
