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
            section(ui, tr!("Allgemein", "General"), |ui| {
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
                        ui.label(tr!("Darstellung:", "Appearance:"));
                        combo(ui, "theme", &mut settings.theme, &Theme::ALL, Theme::label);
                        ui.end_row();
                    });
                ui.add_space(4.0);

                let in_tray = settings.tray_icon && tray::SUPPORTED;
                ui.add_enabled_ui(system::AUTOSTART_SUPPORTED, |ui| {
                    ui.checkbox(&mut settings.autostart, tr!("Mit Windows starten", "Start with Windows"));
                    ui.indent("autostart_options", |ui| {
                        ui.add_enabled(
                            settings.autostart,
                            egui::Checkbox::new(
                                &mut settings.start_minimized,
                                if in_tray {
                                    tr!(
                                        "Im Hintergrund starten (nur Symbol im Infobereich)",
                                        "Start in the background (system tray icon only)"
                                    )
                                } else {
                                    tr!(
                                        "Minimiert starten (nur in der Taskleiste)",
                                        "Start minimized (taskbar only)"
                                    )
                                },
                            ),
                        );
                    });
                })
                .response
                .on_disabled_hover_text(tr!("Nur unter Windows verfügbar", "Only available on Windows"));

                ui.add_space(4.0);
                ui.add_enabled_ui(system::START_MENU_SUPPORTED, |ui| {
                    ui.checkbox(
                        &mut settings.start_menu,
                        tr!("Im Startmenü anzeigen", "Show in Start menu"),
                    );
                    ui.indent("start_menu_options", |ui| {
                        hint(
                            ui,
                            tr!(
                                "Legt die Verknüpfung „WPM“ im Startmenü an. Bei einer Installation \
                                 über winget geschieht das beim ersten Start automatisch, weil winget \
                                 selbst keine anlegt.",
                                "Adds a “WPM” shortcut to the Start menu. When WPM is installed with \
                                 winget, this happens automatically on first start, because winget \
                                 does not create one."
                            ),
                        );
                    });
                })
                .response
                .on_disabled_hover_text(tr!("Nur unter Windows verfügbar", "Only available on Windows"));

                ui.add_space(4.0);
                ui.add_enabled_ui(tray::SUPPORTED, |ui| {
                    ui.checkbox(
                        &mut settings.tray_icon,
                        tr!("Symbol im Infobereich anzeigen", "Show icon in the system tray"),
                    );
                    ui.indent("tray_options", |ui| {
                        ui.add_enabled(
                            settings.tray_icon,
                            egui::Checkbox::new(
                                &mut settings.close_to_tray,
                                tr!(
                                    "Beim Schließen im Hintergrund weiterlaufen",
                                    "Keep running in the background when closed"
                                ),
                            ),
                        );
                        hint(
                            ui,
                            tr!(
                                "Das Symbol sitzt unten rechts neben der Uhr (evtl. hinter „^“) und \
                                 zeigt mit einem orangen Punkt, dass Updates verfügbar sind. Klick: \
                                 WPM öffnen, Rechtsklick: Nach Updates suchen oder Beenden. Im \
                                 Hintergrund sucht WPM weiter nach Updates und meldet neue per \
                                 Benachrichtigung. Mit Administratorrechten kehrt WPM beim Schließen \
                                 ohne Adminrechte in den Hintergrund zurück.",
                                "The icon sits at the bottom right next to the clock (possibly behind \
                                 “^”) and shows an orange dot when updates are available. Click: Open \
                                 WPM, right-click: Check for updates or Quit. In the background, WPM \
                                 keeps checking for updates and reports new ones with a notification. \
                                 If WPM is running with administrator rights, closing it sends it back \
                                 to the background without admin rights."
                            ),
                        );
                    });
                })
                .response
                .on_disabled_hover_text(tr!("Nur unter Windows verfügbar", "Only available on Windows"));

                ui.add_space(4.0);
                ui.add_enabled_ui(system::ELEVATION_SUPPORTED, |ui| {
                    ui.checkbox(
                        &mut settings.run_as_admin,
                        tr!(
                            "Updates mit Administratorrechten installieren",
                            "Install updates with administrator rights"
                        ),
                    );
                    ui.indent("admin_options", |ui| {
                        hint(
                            ui,
                            tr!(
                                "Vor dem ersten Update startet WPM sich einmal mit Administratorrechten \
                                 neu und setzt die Updates automatisch fort. Windows fragt dann einmal \
                                 statt bei jedem einzelnen Installer. Autostart und Update-Suche \
                                 laufen weiterhin ohne Nachfrage. Nur sinnvoll, wenn dein \
                                 Benutzerkonto ein Administratorkonto ist.",
                                "Before the first update, WPM restarts once with administrator rights \
                                 and continues the updates automatically. Windows then asks once \
                                 instead of for every single installer. Autostart and update checks \
                                 still run without prompts. Only useful if your user account is \
                                 an administrator account."
                            ),
                        );
                        if settings.run_as_admin && !system::exe_in_protected_folder() {
                            hint(
                                ui,
                                tr!(
                                    "Tipp: wpm.exe am besten unter „C:\\Program Files\\WPM“ ablegen. \
                                     In beschreibbaren Ordnern wie „Downloads“ könnten fremde Dateien \
                                     neben wpm.exe mit Administratorrechten mitgeladen werden.",
                                    "Tip: Ideally, put wpm.exe in “C:\\Program Files\\WPM”. In \
                                     writable folders such as “Downloads”, untrusted files next to \
                                     wpm.exe could also be loaded with administrator rights."
                                ),
                            );
                        }
                        ui.horizontal(|ui| {
                            if elevated {
                                ui.label(
                                    RichText::new(tr!(
                                        "🛡 WPM läuft gerade mit Administratorrechten.",
                                        "🛡 WPM is currently running with administrator rights."
                                    ))
                                        .weak(),
                                );
                            } else {
                                ui.label(
                                    RichText::new(tr!(
                                        "WPM läuft gerade ohne Administratorrechte.",
                                        "WPM is currently running without administrator rights."
                                    ))
                                        .weak(),
                                );
                                if ui
                                    .add_enabled(
                                        can_restart,
                                        egui::Button::new(tr!(
                                            "🛡 Jetzt als Administrator neu starten",
                                            "🛡 Restart as administrator now"
                                        ))
                                            .small(),
                                    )
                                    .on_disabled_hover_text(tr!(
                                        "Erst möglich, wenn keine Vorgänge mehr laufen.",
                                        "Only possible once no operations are running."
                                    ))
                                    .clicked()
                                {
                                    restart = true;
                                }
                            }
                        });
                    });
                })
                .response
                .on_disabled_hover_text(tr!("Nur unter Windows verfügbar", "Only available on Windows"));
            });

            section(ui, "Updates", |ui| {
                let settings = &mut self.settings;
                ui.checkbox(
                    &mut settings.check_on_startup,
                    tr!("Beim Programmstart nach Updates suchen", "Check for updates at startup"),
                );
                ui.horizontal(|ui| {
                    ui.checkbox(
                        &mut settings.auto_check,
                        tr!("Regelmäßig nach Updates suchen, alle", "Check for updates regularly, every"),
                    );
                    ui.add_enabled(
                        settings.auto_check,
                        egui::DragValue::new(&mut settings.check_interval_hours)
                            .range(1..=168)
                            .suffix(tr!(" Std.", " h")),
                    );
                });
                ui.indent("auto_check_options", |ui| {
                    ui.add_enabled(
                        settings.auto_check,
                        egui::Checkbox::new(
                            &mut settings.notify,
                            tr!(
                                "Windows-Benachrichtigung bei neuen Updates",
                                "Windows notification for new updates"
                            ),
                        ),
                    );
                });
                ui.checkbox(
                    &mut settings.include_unknown,
                    tr!(
                        "Auch Pakete mit unbekannter Versionsnummer anzeigen",
                        "Also show packages with an unknown version number"
                    ),
                )
                .on_hover_text("winget upgrade --include-unknown");
                ui.add_space(4.0);
                hint(
                    ui,
                    tr!(
                        "Die automatische Suche läuft, solange WPM läuft – auch minimiert oder im \
                         Hintergrund (Infobereich). Zusammen mit „Mit Windows starten“ wird nach dem \
                         Hochfahren automatisch geprüft.",
                        "Automatic checks run as long as WPM is running – even when minimized or \
                         in the background (system tray). Together with “Start with Windows”, \
                         updates are checked automatically after Windows starts."
                    ),
                );
            });

            section(ui, "Installation", |ui| {
                let settings = &mut self.settings;
                egui::Grid::new("install_grid")
                    .num_columns(2)
                    .spacing([16.0, 8.0])
                    .show(ui, |ui| {
                        ui.label(tr!("Installationsmodus:", "Installation mode:"));
                        combo(ui, "install_mode", &mut settings.install_mode, &InstallMode::ALL, InstallMode::label);
                        ui.end_row();
                        ui.label(tr!("Installieren für:", "Install for:"));
                        combo(ui, "scope", &mut settings.scope, &Scope::ALL, Scope::label);
                        ui.end_row();
                        ui.label(tr!("Suchen in:", "Search in:"));
                        combo(ui, "search_source", &mut settings.search_source, &SourceFilter::ALL, SourceFilter::label);
                        ui.end_row();
                    });
                ui.checkbox(
                    &mut settings.confirm_uninstall,
                    tr!("Vor dem Deinstallieren nachfragen", "Ask before uninstalling"),
                );
                ui.add_space(4.0);
                hint(
                    ui,
                    tr!(
                        "Im Modus „Still“ erscheinen kaum Installer-Fenster. Einzelne Installer zeigen \
                         trotzdem ein Fenster oder starten das Programm nach dem Update neu. \
                         Windows-Bestätigungen (UAC) vermeidet „Updates mit Administratorrechten installieren“.",
                        "In “Silent” mode, hardly any installer windows appear. Some installers \
                         still show a window or restart the program after the update. UAC prompts \
                         can be avoided with “Install updates with administrator rights”."
                    ),
                );
                hint(
                    ui,
                    tr!(
                        "Lizenz- und Quellvereinbarungen werden automatisch akzeptiert \
                         (--accept-package-agreements, --accept-source-agreements), da winget \
                         ohne Konsole nicht nachfragen kann.",
                        "License and source agreements are accepted automatically \
                         (--accept-package-agreements, --accept-source-agreements) because \
                         winget cannot ask for them without a console."
                    ),
                );
            });

            section(ui, "winget", |ui| {
                match &self.winget_version {
                    Some(Ok(version)) => ui.label(format!("Version: {version}")),
                    Some(Err(e)) => ui.colored_label(ui.visuals().error_fg_color, e),
                    None => ui.label(tr!("Version wird ermittelt …", "Determining version …")),
                };
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    ui.strong(tr!("Quellen", "Sources"));
                    if ui.small_button(tr!("Anzeigen", "Show")).clicked() {
                        requests.push(Request::SourceList);
                    }
                    if ui
                        .small_button(tr!("🔄 Aktualisieren", "🔄 Update"))
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
                    ui.strong(tr!("Angeheftete Pakete", "Pinned packages"));
                    if ui.small_button(tr!("Anzeigen", "Show")).clicked() {
                        requests.push(Request::PinList);
                    }
                });
                hint(
                    ui,
                    tr!(
                        "Angeheftete Pakete werden bei „Alle aktualisieren“ übersprungen (winget pin).",
                        "Pinned packages are skipped by “Update all” (winget pin)."
                    ),
                );
                match &self.pins {
                    Some(pins) if pins.is_empty() => {
                        ui.label(RichText::new(tr!("Keine Pakete angeheftet.", "No pinned packages.")).weak());
                    }
                    Some(pins) => {
                        egui::Grid::new("pins").num_columns(4).striped(true).show(ui, |ui| {
                            for pin in pins {
                                ui.label(&pin.name);
                                ui.label(RichText::new(&pin.id).weak());
                                ui.label(&pin.pin_type);
                                if ui.small_button(tr!("Lösen", "Unpin")).clicked() {
                                    requests.push(Request::PinRemove { id: pin.id.clone() });
                                }
                                ui.end_row();
                            }
                        });
                    }
                    None => {}
                }
                ui.add_space(8.0);

                ui.strong(tr!("Paketliste sichern", "Back up package list"));
                hint(
                    ui,
                    tr!(
                        "Exportiert alle über winget verfügbaren Pakete als JSON-Datei, \
                         z. B. um sie auf einem neuen PC wieder zu installieren.",
                        "Exports all packages available via winget as a JSON file, \
                         e.g. to reinstall them on a new PC."
                    ),
                );
                ui.horizontal(|ui| {
                    if ui.button(tr!("📤 Exportieren …", "📤 Export …")).clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .set_title(tr!("Paketliste exportieren", "Export package list"))
                            .add_filter("JSON", &["json"])
                            .set_file_name(tr!("winget-pakete.json", "winget-packages.json"))
                            .save_file()
                    {
                        requests.push(Request::Export { path });
                    }
                    if ui.button(tr!("📥 Importieren …", "📥 Import …")).clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .set_title(tr!("Paketliste importieren", "Import package list"))
                            .add_filter("JSON", &["json"])
                            .pick_file()
                    {
                        requests.push(Request::Import { path });
                    }
                });
            });

            section(ui, tr!("Über", "About"), |ui| {
                ui.label(format!("WPM {}", env!("CARGO_PKG_VERSION")));
                ui.horizontal(|ui| {
                    ui.label(tr!("Freie Software unter der MIT-Lizenz.", "Free software under the MIT License."));
                    if ui
                        .small_button(tr!("Lizenzen anzeigen", "Show licenses"))
                        .on_hover_text(tr!(
                            "Lizenz von WPM und Lizenzhinweise aller enthaltenen \
                             Open-Source-Bibliotheken",
                            "WPM's license and the license notices of all included \
                             open-source libraries"
                        ))
                        .clicked()
                    {
                        self.show_licenses = true;
                    }
                });
                if let Some(path) = Settings::path() {
                    ui.label(
                        RichText::new(trf!("Einstellungen: {}", "Settings: {}", path.display()))
                            .weak(),
                    );
                }
                ui.add_space(4.0);
                if ui.button(tr!("Standardwerte wiederherstellen", "Restore defaults")).clicked() {
                    // Autostart und Startmenü geben wieder, was eingetragen ist;
                    // die Sprache bleibt, sonst wechselt sie ungefragt.
                    let current = &self.settings;
                    self.settings = Settings {
                        autostart: current.autostart,
                        start_menu: current.start_menu,
                        start_menu_offered: current.start_menu_offered,
                        background_hint_shown: current.background_hint_shown,
                        language: current.language,
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
