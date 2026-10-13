//! Kleine, von mehreren Reitern genutzte Bausteine.

use std::time::Instant;

use eframe::egui::{self, Color32, RichText, Ui};
use egui_extras::TableBuilder;

use super::PackageStatus;

pub const ROW_HEIGHT: f32 = 28.0;
pub const HEADER_HEIGHT: f32 = 22.0;
pub const GREEN: Color32 = Color32::from_rgb(60, 160, 80);

pub fn table<'a>(ui: &'a mut Ui, id: &str) -> TableBuilder<'a> {
    TableBuilder::new(ui)
        .id_salt(id)
        .striped(true)
        .resizable(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
}

pub fn header(row: &mut egui_extras::TableRow<'_, '_>, titles: &[&str]) {
    for title in titles {
        row.col(|ui| {
            ui.strong(*title);
        });
    }
}

/// Text, der bei zu schmaler Spalte mit „…“ abgeschnitten wird.
pub fn cell(ui: &mut Ui, text: impl Into<egui::WidgetText>) {
    ui.add(egui::Label::new(text).truncate());
}

/// Zeigt den Bearbeitungsstand eines Pakets. Liefert `true`, wenn das Paket
/// gerade nicht bearbeitet wird und Aktionen angeboten werden dürfen.
pub fn status(ui: &mut Ui, status: Option<&PackageStatus>) -> bool {
    match status {
        Some(PackageStatus::Queued) => {
            ui.label(RichText::new("⏳ wartet …").weak());
            false
        }
        Some(PackageStatus::Running) => {
            ui.spinner();
            ui.label("läuft …");
            false
        }
        Some(PackageStatus::Done(message)) => {
            ui.label(RichText::new("✔").color(GREEN).strong())
                .on_hover_text(message);
            true
        }
        Some(PackageStatus::Failed(message)) => {
            ui.label(RichText::new("✖ Fehler").color(ui.visuals().error_fg_color))
                .on_hover_text(message);
            true
        }
        None => true,
    }
}

/// Hinweis mittig im freien Bereich, optional mit Ladeanimation.
pub fn message(ui: &mut Ui, text: &str, loading: bool) {
    ui.add_space(40.0);
    ui.vertical_centered(|ui| {
        if loading {
            ui.spinner();
            ui.add_space(6.0);
        }
        ui.label(RichText::new(text).size(16.0).weak());
    });
}

pub fn ago(time: Instant) -> String {
    let seconds = time.elapsed().as_secs();
    match seconds {
        0..60 => "gerade eben".into(),
        60..3600 => format!("vor {} Min.", seconds / 60),
        _ => format!("vor {} Std.", seconds / 3600),
    }
}

/// Abschnitt mit Überschrift und Rahmen (Einstellungen).
pub fn section(ui: &mut Ui, title: &str, add_contents: impl FnOnce(&mut Ui)) {
    ui.add_space(10.0);
    ui.label(RichText::new(title).heading());
    ui.add_space(2.0);
    egui::Frame::group(ui.style())
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add_contents(ui);
        });
}
