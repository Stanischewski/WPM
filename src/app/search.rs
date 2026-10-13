//! Reiter „Suchen“: neue Pakete finden und installieren.

use eframe::egui::{self, RichText};
use egui_extras::Column;

use super::widgets::{self, GREEN, HEADER_HEIGHT, ROW_HEIGHT};
use super::{Action, WpmApp};
use crate::settings::SourceFilter;
use crate::winget::Request;

impl WpmApp {
    pub(super) fn search_tab(&mut self, ui: &mut egui::Ui) {
        // Die Liste der installierten Pakete wird gebraucht, um Treffer zu markieren.
        if self.installed.is_none() && !self.busy(|r| matches!(r, Request::ListInstalled)) {
            self.submit(Request::ListInstalled);
        }
        let searching = self.busy(|r| matches!(r, Request::Search { .. }));
        let mut start_search = false;

        ui.horizontal(|ui| {
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.search_query)
                    .hint_text("Paketname, ID oder Stichwort …")
                    .desired_width(320.0),
            );
            if std::mem::take(&mut self.focus_search) {
                response.request_focus();
            }
            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                start_search = true;
            }
            let can_search = !searching && !self.search_query.trim().is_empty();
            if ui
                .add_enabled(can_search, egui::Button::new("🔍 Suchen"))
                .clicked()
            {
                start_search = true;
            }
            egui::ComboBox::from_id_salt("search_source")
                .selected_text(self.settings.search_source.label())
                .show_ui(ui, |ui| {
                    for source in SourceFilter::ALL {
                        ui.selectable_value(
                            &mut self.settings.search_source,
                            source,
                            source.label(),
                        );
                    }
                });
            if searching {
                ui.spinner();
            }
        });
        ui.separator();

        let query = self.search_query.trim().to_string();
        if start_search && !query.is_empty() {
            let source = self.settings.search_source.source_name().map(String::from);
            self.submit(Request::Search { query, source });
        }

        let mut actions = Vec::new();
        match &self.search_results {
            None if searching => widgets::message(ui, "Suche läuft …", true),
            None => widgets::message(ui, "Suchbegriff eingeben und mit Enter bestätigen.", false),
            Some(results) if results.is_empty() => {
                widgets::message(ui, "Keine Pakete gefunden.", searching);
            }
            Some(results) => {
                let installed = self.installed_ids();
                let status = &self.status;
                widgets::table(ui, "search")
                    .column(Column::initial(280.0).at_least(100.0).clip(true))
                    .column(Column::initial(280.0).at_least(80.0).clip(true))
                    .column(Column::initial(120.0).at_least(60.0).clip(true))
                    .column(Column::initial(70.0).at_least(50.0).clip(true))
                    .column(Column::remainder().at_least(180.0))
                    .header(HEADER_HEIGHT, |mut header| {
                        widgets::header(
                            &mut header,
                            &["Name", "ID", "Version", "Quelle", "Aktionen"],
                        );
                    })
                    .body(|body| {
                        body.rows(ROW_HEIGHT, results.len(), |mut row| {
                            let package = &results[row.index()];
                            row.col(|ui| widgets::cell(ui, &package.name));
                            row.col(|ui| widgets::cell(ui, &package.id));
                            row.col(|ui| widgets::cell(ui, &package.version));
                            row.col(|ui| widgets::cell(ui, &package.source));
                            row.col(|ui| {
                                if !widgets::status(ui, status.get(&package.id)) {
                                    return;
                                }
                                if installed.contains(package.id.as_str()) {
                                    ui.label(RichText::new("✔ Installiert").color(GREEN));
                                } else if ui.button("⬇ Installieren").clicked() {
                                    actions.push(Action::Install(package.clone()));
                                }
                                if ui.button("ℹ").on_hover_text("Details").clicked() {
                                    actions.push(Action::ShowDetails(package.clone()));
                                }
                            });
                        });
                    });
            }
        }

        for action in actions {
            self.apply(action);
        }
    }
}
