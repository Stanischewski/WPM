//! Reiter „Updates“: verfügbare Aktualisierungen anzeigen und installieren.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use eframe::egui::{self, RichText};
use egui_extras::Column;

use super::widgets::{self, HEADER_HEIGHT, ROW_HEIGHT};
use super::{Action, PackageStatus, WpmApp};
use crate::winget::{Package, Request};

impl WpmApp {
    pub(super) fn updates_tab(&mut self, ui: &mut egui::Ui) {
        let checking = self.busy(|r| matches!(r, Request::CheckUpdates { .. }));
        let mut actions = Vec::new();
        let upgrades: &[Package] = self.updates.as_ref().map_or(&[], |u| &u.upgrades);
        let selected: Vec<&Package> = upgrades
            .iter()
            .filter(|p| self.selected.contains(&p.id))
            .collect();

        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !checking,
                    egui::Button::new(tr!("🔄 Nach Updates suchen", "🔄 Check for updates")),
                )
                .clicked()
            {
                actions.push(Action::CheckUpdates);
            }
            let label = trf!(
                "⬆ Auswahl aktualisieren ({})",
                "⬆ Update selected ({})",
                selected.len()
            );
            if ui
                .add_enabled(!selected.is_empty(), egui::Button::new(label))
                .clicked()
            {
                actions.extend(selected.iter().map(|p| Action::Upgrade((*p).clone())));
            }
            let label = trf!(
                "⏫ Alle aktualisieren ({})",
                "⏫ Update all ({})",
                upgrades.len()
            );
            if ui
                .add_enabled(!upgrades.is_empty(), egui::Button::new(label))
                .on_hover_text(tr!(
                    "Angeheftete Pakete werden übersprungen.",
                    "Pinned packages are skipped."
                ))
                .clicked()
            {
                actions.extend(upgrades.iter().cloned().map(Action::Upgrade));
            }
            if let Some(checked) = self.updates_checked_at {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(trf!(
                            "Zuletzt geprüft: {}",
                            "Last checked: {}",
                            widgets::ago(checked)
                        ))
                        .weak(),
                    );
                    ui.ctx().request_repaint_after(Duration::from_secs(30));
                });
            }
        });
        ui.separator();

        match &self.updates {
            None => {
                let text = if checking {
                    tr!("Suche nach Updates …", "Checking for updates …")
                } else {
                    tr!(
                        "Noch nicht nach Updates gesucht.",
                        "Not checked for updates yet."
                    )
                };
                widgets::message(ui, text, checking);
            }
            Some(list) if list.upgrades.is_empty() && list.pinned.is_empty() => {
                widgets::message(
                    ui,
                    tr!(
                        "✔ Alle Pakete sind auf dem neuesten Stand.",
                        "✔ All packages are up to date."
                    ),
                    false,
                );
            }
            Some(list) => {
                egui::ScrollArea::vertical()
                    .id_salt("updates")
                    .auto_shrink(false)
                    .show(ui, |ui| {
                    if list.upgrades.is_empty() {
                        ui.label(RichText::new(tr!("Keine Updates außer für angeheftete Pakete.", "No updates except for pinned packages.")).weak());
                    } else {
                        upgrades_table(ui, &list.upgrades, &mut self.selected, &self.status, &mut actions);
                    }
                    if !list.pinned.is_empty() {
                        ui.add_space(16.0);
                        ui.label(RichText::new(tr!("📌 Angeheftete Pakete", "📌 Pinned packages")).heading());
                        ui.label(
                            RichText::new(tr!(
                                "Für diese Pakete gibt es Updates, sie werden aber nur auf ausdrücklichen Wunsch installiert.",
                                "Updates are available for these packages, but they are only installed on explicit request.",
                            ))
                            .weak(),
                        );
                        pinned_table(ui, &list.pinned, &self.status, &mut actions);
                    }
                });
            }
        }

        for action in actions {
            self.apply(action);
        }
    }
}

fn upgrades_table(
    ui: &mut egui::Ui,
    upgrades: &[Package],
    selected: &mut HashSet<String>,
    status: &HashMap<String, PackageStatus>,
    actions: &mut Vec<Action>,
) {
    widgets::table(ui, "upgrades")
        .vscroll(false)
        .column(Column::exact(24.0))
        .column(Column::initial(240.0).at_least(100.0).clip(true))
        .column(Column::initial(220.0).at_least(80.0).clip(true))
        .column(Column::initial(110.0).at_least(60.0).clip(true))
        .column(Column::initial(110.0).at_least(60.0).clip(true))
        .column(Column::initial(70.0).at_least(50.0).clip(true))
        .column(Column::remainder().at_least(190.0))
        .header(HEADER_HEIGHT, |mut header| {
            header.col(|ui| {
                let mut all = upgrades.iter().all(|p| selected.contains(&p.id));
                if ui
                    .checkbox(&mut all, "")
                    .on_hover_text(tr!("Alle auswählen", "Select all"))
                    .changed()
                {
                    if all {
                        selected.extend(upgrades.iter().map(|p| p.id.clone()));
                    } else {
                        selected.clear();
                    }
                }
            });
            widgets::header(
                &mut header,
                &[
                    "Name",
                    "ID",
                    tr!("Installiert", "Installed"),
                    tr!("Verfügbar", "Available"),
                    tr!("Quelle", "Source"),
                    tr!("Aktionen", "Actions"),
                ],
            );
        })
        .body(|body| {
            body.rows(ROW_HEIGHT, upgrades.len(), |mut row| {
                let package = &upgrades[row.index()];
                row.col(|ui| {
                    let mut checked = selected.contains(&package.id);
                    if ui.checkbox(&mut checked, "").changed() {
                        if checked {
                            selected.insert(package.id.clone());
                        } else {
                            selected.remove(&package.id);
                        }
                    }
                });
                row.col(|ui| widgets::cell(ui, &package.name));
                row.col(|ui| widgets::cell(ui, &package.id));
                row.col(|ui| widgets::cell(ui, &package.version));
                row.col(|ui| widgets::cell(ui, RichText::new(&package.available).strong()));
                row.col(|ui| widgets::cell(ui, &package.source));
                row.col(|ui| {
                    if widgets::status(ui, status.get(&package.id)) {
                        let mut button = ui.button(tr!("⬆ Aktualisieren", "⬆ Update"));
                        if crate::self_update::is_self(package) {
                            button = button.on_hover_text(tr!(
                                "WPM beendet sich dafür kurz und startet danach von selbst neu \
                                 (nach allen anderen Updates).",
                                "WPM briefly closes for this and then restarts automatically \
                                 (after all other updates).",
                            ));
                        }
                        if button.clicked() {
                            actions.push(Action::Upgrade(package.clone()));
                        }
                        if ui
                            .button("📌")
                            .on_hover_text(tr!(
                                "Anheften: Update ignorieren (winget pin)",
                                "Pin: ignore update (winget pin)"
                            ))
                            .clicked()
                        {
                            actions.push(Action::Pin(package.clone()));
                        }
                        if ui.button("ℹ").on_hover_text("Details").clicked() {
                            actions.push(Action::ShowDetails(package.clone()));
                        }
                    }
                });
            });
        });
}

fn pinned_table(
    ui: &mut egui::Ui,
    pinned: &[Package],
    status: &HashMap<String, PackageStatus>,
    actions: &mut Vec<Action>,
) {
    widgets::table(ui, "pinned")
        .vscroll(false)
        .column(Column::initial(264.0).at_least(100.0).clip(true))
        .column(Column::initial(220.0).at_least(80.0).clip(true))
        .column(Column::initial(110.0).at_least(60.0).clip(true))
        .column(Column::initial(110.0).at_least(60.0).clip(true))
        .column(Column::initial(70.0).at_least(50.0).clip(true))
        .column(Column::remainder().at_least(190.0))
        .header(HEADER_HEIGHT, |mut header| {
            widgets::header(
                &mut header,
                &[
                    "Name",
                    "ID",
                    tr!("Installiert", "Installed"),
                    tr!("Verfügbar", "Available"),
                    tr!("Quelle", "Source"),
                    tr!("Aktionen", "Actions"),
                ],
            );
        })
        .body(|body| {
            body.rows(ROW_HEIGHT, pinned.len(), |mut row| {
                let package = &pinned[row.index()];
                row.col(|ui| widgets::cell(ui, &package.name));
                row.col(|ui| widgets::cell(ui, &package.id));
                row.col(|ui| widgets::cell(ui, &package.version));
                row.col(|ui| widgets::cell(ui, &package.available));
                row.col(|ui| widgets::cell(ui, &package.source));
                row.col(|ui| {
                    if widgets::status(ui, status.get(&package.id)) {
                        if ui
                            .button(tr!("⬆ Trotzdem aktualisieren", "⬆ Update anyway"))
                            .clicked()
                        {
                            actions.push(Action::Upgrade(package.clone()));
                        }
                        if ui
                            .button(tr!("Lösen", "Unpin"))
                            .on_hover_text(tr!("Pin entfernen", "Remove pin"))
                            .clicked()
                        {
                            actions.push(Action::Unpin(package.id.clone()));
                        }
                    }
                });
            });
        });
}
