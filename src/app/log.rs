//! Reiter „Protokoll“: alle ausgeführten winget-Befehle mit ihrer Ausgabe.

use eframe::egui::{self, RichText};

use super::WpmApp;

impl WpmApp {
    pub(super) fn log_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button(tr!("📋 Kopieren", "📋 Copy")).clicked() {
                let text = self.log.iter().cloned().collect::<Vec<_>>().join("\n");
                ui.ctx().copy_text(text);
            }
            if ui.button(tr!("🗑 Leeren", "🗑 Clear")).clicked() {
                self.log.clear();
            }
            // Nur Englisch braucht die Einzahl, der deutsche Text bleibt gleich.
            let count = match self.log.len() {
                1 => trf!("{} Zeilen", "{} line", 1),
                n => trf!("{} Zeilen", "{} lines", n),
            };
            ui.label(RichText::new(count).weak());
        });
        ui.separator();

        if self.log.is_empty() {
            ui.label(RichText::new(tr!("Noch keine Ausgaben.", "No output yet.")).weak());
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
