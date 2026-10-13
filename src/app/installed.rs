//! Reiter „Installiert“: alle installierten Programme, filterbar.

use eframe::egui::{self, RichText};
use egui_extras::Column;

use super::widgets::{self, HEADER_HEIGHT, ROW_HEIGHT};
use super::{Action, WpmApp};
use crate::winget::{Package, Request};

impl WpmApp {
    pub(super) fn installed_tab(&mut self, ui: &mut egui::Ui) {
        let loading = self.busy(|r| matches!(r, Request::ListInstalled));
        if self.installed.is_none() && !loading {
            self.submit(Request::ListInstalled);
        }
        let mut actions = Vec::new();

        let filter = self.installed_filter.trim().to_lowercase();
        let hide_unmanaged = self.settings.hide_unmanaged;
        let visible: Vec<&Package> = self
            .installed
            .iter()
            .flatten()
            .filter(|p| !hide_unmanaged || p.has_source())
            .filter(|p| {
                filter.is_empty()
                    || p.name.to_lowercase().contains(&filter)
                    || p.id.to_lowercase().contains(&filter)
            })
            .collect();

        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.installed_filter)
                    .hint_text("🔍 Nach Name oder ID filtern …")
                    .desired_width(260.0),
            );
            ui.checkbox(&mut self.settings.hide_unmanaged, "Nur Pakete aus Quellen")
                .on_hover_text(
                    "Programme ausblenden, die winget nur lokal erkannt hat \
                     (z. B. Treiber, Systemkomponenten). Diese lassen sich nicht über winget aktualisieren.",
                );
            if ui
                .add_enabled(!loading, egui::Button::new("🔄 Neu laden"))
                .clicked()
            {
                actions.push(Action::LoadInstalled);
            }
            if loading {
                ui.spinner();
            }
            if let Some(all) = &self.installed {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(format!("{} von {} Paketen", visible.len(), all.len())).weak());
                });
            }
        });
        ui.separator();

        if self.installed.is_none() {
            widgets::message(ui, "Lade installierte Pakete …", true);
        } else if visible.is_empty() {
            widgets::message(ui, "Keine passenden Pakete.", false);
        } else {
            let status = &self.status;
            widgets::table(ui, "installed")
                .column(Column::initial(260.0).at_least(100.0).clip(true))
                .column(Column::initial(250.0).at_least(80.0).clip(true))
                .column(Column::initial(110.0).at_least(60.0).clip(true))
                .column(Column::initial(110.0).at_least(60.0).clip(true))
                .column(Column::initial(70.0).at_least(50.0).clip(true))
                .column(Column::remainder().at_least(200.0))
                .header(HEADER_HEIGHT, |mut header| {
                    widgets::header(
                        &mut header,
                        &["Name", "ID", "Version", "Verfügbar", "Quelle", "Aktionen"],
                    );
                })
                .body(|body| {
                    body.rows(ROW_HEIGHT, visible.len(), |mut row| {
                        let package = visible[row.index()];
                        row.col(|ui| widgets::cell(ui, &package.name));
                        row.col(|ui| widgets::cell(ui, &package.id));
                        row.col(|ui| widgets::cell(ui, &package.version));
                        row.col(|ui| widgets::cell(ui, RichText::new(&package.available).strong()));
                        row.col(|ui| widgets::cell(ui, &package.source));
                        row.col(|ui| {
                            if !widgets::status(ui, status.get(&package.id)) {
                                return;
                            }
                            if package.has_update() && ui.button("⬆ Aktualisieren").clicked() {
                                actions.push(Action::Upgrade(package.clone()));
                            }
                            if package.has_source()
                                && ui.button("ℹ").on_hover_text("Details").clicked()
                            {
                                actions.push(Action::ShowDetails(package.clone()));
                            }
                            if ui.button("🗑").on_hover_text("Deinstallieren").clicked() {
                                actions.push(Action::Uninstall(package.clone()));
                            }
                        });
                    });
                });
        }

        for action in actions {
            self.apply(action);
        }
    }
}
