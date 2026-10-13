//! Reiter „Protokoll“: alle ausgeführten winget-Befehle mit ihrer Ausgabe.

use eframe::egui::{self, RichText};

use super::WpmApp;

impl WpmApp {
    pub(super) fn log_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("📋 Kopieren").clicked() {
                let text = self.log.iter().cloned().collect::<Vec<_>>().join("\n");
                ui.ctx().copy_text(text);
            }
            if ui.button("🗑 Leeren").clicked() {
                self.log.clear();
            }
            ui.label(RichText::new(format!("{} Zeilen", self.log.len())).weak());
        });
        ui.separator();

        if self.log.is_empty() {
            ui.label(RichText::new("Noch keine Ausgaben.").weak());
            return;
        }
        let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
        let command_color = ui.visuals().hyperlink_color;
        egui::ScrollArea::both()
            .id_salt("log")
            .auto_shrink(false)
            .stick_to_bottom(true)
            .show_rows(ui, row_height, self.log.len(), |ui, rows| {
                for line in self.log.range(rows) {
                    let mut text = RichText::new(line).monospace();
                    if line.starts_with("> ") {
                        text = text.color(command_color).strong();
                    }
                    ui.add(egui::Label::new(text).extend());
                }
            });
    }
}
