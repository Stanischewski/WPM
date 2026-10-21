//! Reiter „Einstellungen“ inklusive winget-Verwaltung (Quellen, Pins, Export/Import).

use eframe::egui::{self, RichText};

use super::WpmApp;
use super::widgets::section;
use crate::settings::{InstallMode, Language, Scope, Settings, SourceFilter, Theme};
use crate::system;
use crate::tray;
use crate::winget::Request;

fn combo<T: Copy + PartialEq>(
    ui: &mut egui::Ui,
    id: &str,
    value: &mut T,
    options: &[T],
    label: impl Fn(T) -> &'static str,
) {
    egui::ComboBox::from_id_salt(id)
        .width(220.0)
        .selected_text(label(*value))
        .show_ui(ui, |ui| {
            for &option in options {
                ui.selectable_value(value, option, label(option));
            }
        });
}

fn hint(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).weak().small()).wrap());
}

impl WpmApp {
    pub(super) fn settings_tab(&mut self, ui: &mut egui::Ui) {
        let mut requests = Vec::new();
        let mut restart = false;

        egui::ScrollArea::vertical()
            .id_salt("settings")
            .auto_shrink(false)
            .show(ui, |ui| {
            ui.set_max_width(760.0);

            let elevated = self.elevated;
            let can_restart = self.can_restart_elevated();
            section(ui, "Allgemein", |ui| {
                let settings = &mut self.settings;
                egui::Grid::new("appearance_grid")
                    .num_columns(2)
                    .spacing([16.0, 8.0])
                    .show(ui, |ui| {
                        // Auch auf Englisch auffindbar, falls jemand die Sprache
                        // versehentlich umgestellt hat.
                        ui.label(tr!("Sprache (Language):", "Language:"));
                        combo(ui, "language", &mut settings.language, &Language::ALL, Language::label);
                        ui.end_row();
                        ui.label("Darstellung:");
                        combo(ui, "theme", &mut settings.theme, &Theme::ALL, Theme::label);
                        ui.end_row();
                    });
                ui.add_space(4.0);

                let in_tray = settings.tray_icon && tray::SUPPORTED;
                ui.add_enabled_ui(system::AUTOSTART_SUPPORTED, |ui| {
                    ui.checkbox(&mut settings.autostart, "Mit Windows starten");
                    ui.indent("autostart_options", |ui| {
                        ui.add_enabled(
                            settings.autostart,
                            egui::Checkbox::new(
                                &mut settings.start_minimized,
                                if in_tray {
                                    "Im Hintergrund starten (nur Symbol im Infobereich)"
                                } else {
                                    "Minimiert starten (nur in der Taskleiste)"
                                },
                            ),
                        );
                    });
                })
                .response
                .on_disabled_hover_text("Nur unter Windows verfügbar");

                ui.add_space(4.0);
                ui.add_enabled_ui(tray::SUPPORTED, |ui| {
                    ui.checkbox(&mut settings.tray_icon, "Symbol im Infobereich anzeigen");
                    ui.indent("tray_options", |ui| {
                        ui.add_enabled(
                            settings.tray_icon,
                            egui::Checkbox::new(
                                &mut settings.close_to_tray,
                                "Beim Schließen im Hintergrund weiterlaufen",
                            ),
                        );
                        hint(
                            ui,
                            "Das Symbol sitzt unten rechts neben der Uhr (evtl. hinter „^“) und \
                             zeigt mit einem orangen Punkt, dass Updates verfügbar sind. Klick: \
                             WPM öffnen, Rechtsklick: Nach Updates suchen oder Beenden. Im \
                             Hintergrund sucht WPM weiter nach Updates und meldet neue per \
                             Benachrichtigung. Mit Administratorrechten kehrt WPM beim Schließen \
                             ohne Adminrechte in den Hintergrund zurück.",
                        );
                    });
                })
                .response
                .on_disabled_hover_text("Nur unter Windows verfügbar");

                ui.add_space(4.0);
                ui.add_enabled_ui(system::ELEVATION_SUPPORTED, |ui| {
                    ui.checkbox(
                        &mut settings.run_as_admin,
                        "Updates mit Administratorrechten installieren",
                    );
                    ui.indent("admin_options", |ui| {
                        hint(
                            ui,
                            "Vor dem ersten Update startet WPM sich einmal mit Administratorrechten \
                             neu und setzt die Updates automatisch fort. Windows fragt dann einmal \
                             statt bei jedem einzelnen Installer. Autostart und Update-Suche \
                             laufen weiterhin ohne Nachfrage. Nur sinnvoll, wenn dein \
                             Benutzerkonto ein Administratorkonto ist.",
                        );
                        if settings.run_as_admin && !system::exe_in_protected_folder() {
                            hint(
                                ui,
                                "Tipp: wpm.exe am besten unter „C:\\Program Files\\WPM“ ablegen. \
                                 In beschreibbaren Ordnern wie „Downloads“ könnten fremde Dateien \
                                 neben wpm.exe mit Administratorrechten mitgeladen werden.",
                            );
                        }
                        ui.horizontal(|ui| {
                            if elevated {
                                ui.label(
                                    RichText::new("🛡 WPM läuft gerade mit Administratorrechten.")
                                        .weak(),
                                );
                            } else {
                                ui.label(
                                    RichText::new("WPM läuft gerade ohne Administratorrechte.")
                                        .weak(),
                                );
                                if ui
                                    .add_enabled(
                                        can_restart,
                                        egui::Button::new("🛡 Jetzt als Administrator neu starten")
                                            .small(),
                                    )
                                    .on_disabled_hover_text(
                                        "Erst möglich, wenn keine Vorgänge mehr laufen.",
                                    )
                                    .clicked()
                                {
                                    restart = true;
                                }
                            }
                        });
                    });
                })
                .response
                .on_disabled_hover_text("Nur unter Windows verfügbar");
            });

            section(ui, "Updates", |ui| {
                let settings = &mut self.settings;
                ui.checkbox(&mut settings.check_on_startup, "Beim Programmstart nach Updates suchen");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut settings.auto_check, "Regelmäßig nach Updates suchen, alle");
                    ui.add_enabled(
                        settings.auto_check,
                        egui::DragValue::new(&mut settings.check_interval_hours)
                            .range(1..=168)
                            .suffix(" Std."),
                    );
                });
                ui.indent("auto_check_options", |ui| {
                    ui.add_enabled(
                        settings.auto_check,
                        egui::Checkbox::new(&mut settings.notify, "Windows-Benachrichtigung bei neuen Updates"),
                    );
                });
                ui.checkbox(
                    &mut settings.include_unknown,
                    "Auch Pakete mit unbekannter Versionsnummer anzeigen",
                )
                .on_hover_text("winget upgrade --include-unknown");
                ui.add_space(4.0);
                hint(
                    ui,
                    "Die automatische Suche läuft, solange WPM läuft – auch minimiert oder im \
                     Hintergrund (Infobereich). Zusammen mit „Mit Windows starten“ wird nach dem \
                     Hochfahren automatisch geprüft.",
                );
            });

            section(ui, "Installation", |ui| {
                let settings = &mut self.settings;
                egui::Grid::new("install_grid")
                    .num_columns(2)
                    .spacing([16.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("Installationsmodus:");
                        combo(ui, "install_mode", &mut settings.install_mode, &InstallMode::ALL, InstallMode::label);
                        ui.end_row();
                        ui.label("Installieren für:");
                        combo(ui, "scope", &mut settings.scope, &Scope::ALL, Scope::label);
                        ui.end_row();
                        ui.label("Suchen in:");
                        combo(ui, "search_source", &mut settings.search_source, &SourceFilter::ALL, SourceFilter::label);
                        ui.end_row();
                    });
                ui.checkbox(&mut settings.confirm_uninstall, "Vor dem Deinstallieren nachfragen");
                ui.add_space(4.0);
                hint(
                    ui,
                    "Im Modus „Still“ erscheinen kaum Installer-Fenster. Einzelne Installer zeigen \
                     trotzdem ein Fenster oder starten das Programm nach dem Update neu. \
                     Windows-Bestätigungen (UAC) vermeidet „Updates mit Administratorrechten installieren“.",
                );
                hint(
                    ui,
                    "Lizenz- und Quellvereinbarungen werden automatisch akzeptiert \
                     (--accept-package-agreements, --accept-source-agreements), da winget \
                     ohne Konsole nicht nachfragen kann.",
                );
            });

            section(ui, "winget", |ui| {
                match &self.winget_version {
                    Some(Ok(version)) => ui.label(format!("Version: {version}")),
                    Some(Err(e)) => ui.colored_label(ui.visuals().error_fg_color, e),
                    None => ui.label("Version wird ermittelt …"),
                };
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    ui.strong("Quellen");
                    if ui.small_button("Anzeigen").clicked() {
                        requests.push(Request::SourceList);
                    }
                    if ui
                        .small_button("🔄 Aktualisieren")
                        .on_hover_text("winget source update")
                        .clicked()
                    {
                        requests.push(Request::SourceUpdate);
                    }
                });
                if let Some(sources) = &self.sources {
                    egui::Grid::new("sources").num_columns(2).striped(true).show(ui, |ui| {
                        for source in sources {
                            ui.label(&source.name);
                            ui.label(RichText::new(&source.argument).weak());
                            ui.end_row();
                        }
                    });
                }
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    ui.strong("Angeheftete Pakete");
                    if ui.small_button("Anzeigen").clicked() {
                        requests.push(Request::PinList);
                    }
                });
                hint(ui, "Angeheftete Pakete werden bei „Alle aktualisieren“ übersprungen (winget pin).");
                match &self.pins {
                    Some(pins) if pins.is_empty() => {
                        ui.label(RichText::new("Keine Pakete angeheftet.").weak());
                    }
                    Some(pins) => {
                        egui::Grid::new("pins").num_columns(4).striped(true).show(ui, |ui| {
                            for pin in pins {
                                ui.label(&pin.name);
                                ui.label(RichText::new(&pin.id).weak());
                                ui.label(&pin.pin_type);
                                if ui.small_button("Lösen").clicked() {
                                    requests.push(Request::PinRemove { id: pin.id.clone() });
                                }
                                ui.end_row();
                            }
                        });
                    }
                    None => {}
                }
                ui.add_space(8.0);

                ui.strong("Paketliste sichern");
                hint(
                    ui,
                    "Exportiert alle über winget verfügbaren Pakete als JSON-Datei, \
                     z. B. um sie auf einem neuen PC wieder zu installieren.",
                );
                ui.horizontal(|ui| {
                    if ui.button("📤 Exportieren …").clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .set_title("Paketliste exportieren")
                            .add_filter("JSON", &["json"])
                            .set_file_name("winget-pakete.json")
                            .save_file()
                    {
                        requests.push(Request::Export { path });
                    }
                    if ui.button("📥 Importieren …").clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .set_title("Paketliste importieren")
                            .add_filter("JSON", &["json"])
                            .pick_file()
                    {
                        requests.push(Request::Import { path });
                    }
                });
            });

            section(ui, "Über", |ui| {
                ui.label(format!("WPM {}", env!("CARGO_PKG_VERSION")));
                ui.horizontal(|ui| {
                    ui.label("Freie Software unter der MIT-Lizenz.");
                    if ui
                        .small_button("Lizenzen anzeigen")
                        .on_hover_text(
                            "Lizenz von WPM und Lizenzhinweise aller enthaltenen \
                             Open-Source-Bibliotheken",
                        )
                        .clicked()
                    {
                        self.show_licenses = true;
                    }
                });
                if let Some(path) = Settings::path() {
                    ui.label(RichText::new(format!("Einstellungen: {}", path.display())).weak());
                }
                ui.add_space(4.0);
                if ui.button("Standardwerte wiederherstellen").clicked() {
                    // Die Sprache bleibt: Sonst wechselt sie ungefragt.
                    let autostart = self.settings.autostart;
                    let background_hint_shown = self.settings.background_hint_shown;
                    let language = self.settings.language;
                    self.settings = Settings {
                        autostart,
                        background_hint_shown,
                        language,
                        ..Settings::default()
                    };
                }
            });
            ui.add_space(12.0);
        });

        for request in requests {
            self.submit(request);
        }
        if restart {
            let ctx = ui.ctx().clone();
            self.restart_elevated(&ctx, &[]);
        }
    }
}
