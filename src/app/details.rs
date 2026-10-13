//! Fenster mit Paketdetails (`winget show`) und Rückfrage beim Deinstallieren.

use eframe::egui::{self, RichText};

use super::{Action, WpmApp};
use crate::winget::Request;

impl WpmApp {
    pub(super) fn details_window(&mut self, ctx: &egui::Context) {
        let Some(view) = &self.details else { return };
        let package = &view.package;
        let installed = self.installed_ids().contains(package.id.as_str());
        let mut open = true;
        let mut action = None;

        egui::Window::new(format!("Details: {}", package.name))
            .id(egui::Id::new("details_window"))
            .open(&mut open)
            .collapsible(false)
            .default_width(560.0)
            .default_height(440.0)
            .show(ctx, |ui| {
                match &view.result {
                    None => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Lade Details …");
                        });
                    }
                    Some(Err(failure)) => {
                        ui.colored_label(ui.visuals().error_fg_color, failure.full_text());
                    }
                    Some(Ok(details)) => {
                        ui.label(RichText::new(&details.title).strong());
                        ui.separator();
                        egui::ScrollArea::vertical()
                            .id_salt("details")
                            .max_height(360.0)
                            .show(ui, |ui| {
                                egui::Grid::new("details_grid")
                                    .num_columns(2)
                                    .striped(true)
                                    .spacing([12.0, 6.0])
                                    .show(ui, |ui| {
                                        for (key, value) in &details.fields {
                                            ui.label(RichText::new(key).strong());
                                            if value.starts_with("https://")
                                                || value.starts_with("http://")
                                            {
                                                ui.hyperlink(value);
                                            } else {
                                                ui.add(egui::Label::new(value).wrap());
                                            }
                                            ui.end_row();
                                        }
                                    });
                            });
                    }
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if package.has_update()
                        && ui
                            .button(format!("⬆ Auf {} aktualisieren", package.available))
                            .clicked()
                    {
                        action = Some(Action::Upgrade(package.clone()));
                    }
                    if !installed && !package.has_update() && ui.button("⬇ Installieren").clicked()
                    {
                        action = Some(Action::Install(package.clone()));
                    }
                });
            });

        if !open || action.is_some() {
            self.details = None;
        }
        if let Some(action) = action {
            self.apply(action);
        }
    }

    pub(super) fn confirm_uninstall_dialog(&mut self, ctx: &egui::Context) {
        let Some(package) = &self.confirm_uninstall else {
            return;
        };
        let mut confirmed = None;
        let modal = egui::Modal::new(egui::Id::new("confirm_uninstall")).show(ctx, |ui| {
            ui.set_width(400.0);
            ui.heading("Deinstallieren?");
            ui.add_space(4.0);
            ui.label(format!(
                "Soll „{}“ wirklich deinstalliert werden?",
                package.name
            ));
            ui.label(RichText::new(&package.id).monospace().weak());
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.button("🗑 Deinstallieren").clicked() {
                    confirmed = Some(true);
                }
                if ui.button("Abbrechen").clicked() {
                    confirmed = Some(false);
                }
            });
        });
        if modal.should_close() && confirmed.is_none() {
            confirmed = Some(false);
        }
        if let Some(confirmed) = confirmed {
            let package = self.confirm_uninstall.take().expect("Dialog ist offen");
            if confirmed {
                self.submit(Request::Uninstall { package });
            }
        }
    }
}
