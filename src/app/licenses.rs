//! Fenster „Lizenzen“: die MIT-Lizenz von WPM und die Lizenzhinweise aller
//! enthaltenen Bibliotheken. Beides steckt in der EXE, damit die Hinweise auch
//! dabei sind, wenn nur `wpm.exe` weitergegeben wird.

use std::sync::OnceLock;

use eframe::egui::{self, RichText};

use super::WpmApp;

const OWN_LICENSE: &str = include_str!("../../LICENSE");
/// Erzeugt mit cargo-about (about.toml, about.hbs), siehe README.
const THIRD_PARTY_LICENSES: &str = include_str!("../../THIRD-PARTY-LICENSES.txt");
/// Längere Zeilen werden umbrochen, damit alle Zeilen gleich hoch sind und nur
/// die sichtbaren gezeichnet werden müssen.
const WRAP_WIDTH: usize = 100;

fn lines() -> &'static [String] {
    static LINES: OnceLock<Vec<String>> = OnceLock::new();
    LINES.get_or_init(|| {
        format!("WPM\n===\n\n{OWN_LICENSE}\n\n{THIRD_PARTY_LICENSES}")
            .lines()
            .flat_map(|line| wrap(line, WRAP_WIDTH))
            .collect()
    })
}

/// Bricht eine Zeile an Leerzeichen um; Folgezeilen behalten die Einrückung.
fn wrap(line: &str, width: usize) -> Vec<String> {
    if line.chars().count() <= width {
        return vec![line.to_string()];
    }
    let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
    let mut wrapped = Vec::new();
    let mut current = String::new();
    for word in line.split_whitespace() {
        let length = current.chars().count();
        if length > 0 && length + 1 + word.chars().count() > width {
            wrapped.push(std::mem::take(&mut current));
        }
        if current.is_empty() {
            current.push_str(&indent);
        } else {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        wrapped.push(current);
    }
    wrapped
}

impl WpmApp {
    pub(super) fn licenses_window(&mut self, ctx: &egui::Context) {
        if !self.show_licenses {
            return;
        }
        let mut open = true;
        egui::Window::new(tr!("Lizenzen", "Licenses"))
            .id(egui::Id::new("licenses_window"))
            .open(&mut open)
            .collapsible(false)
            .default_width(800.0)
            .default_height(520.0)
            .show(ctx, |ui| {
                ui.add(
                    egui::Label::new(tr!(
                        "WPM steht unter der MIT-Lizenz und enthält Open-Source-Bibliotheken \
                         anderer Autorinnen und Autoren. Hier stehen alle Lizenzhinweise, wie es \
                         deren Lizenzen verlangen.",
                        "WPM is licensed under the MIT License and includes open-source libraries \
                         by other authors. All license notices are listed here, as their licenses \
                         require."
                    ))
                    .wrap(),
                );
                ui.separator();
                let lines = lines();
                // Zeilen ohne Abstand, wie in einem Texteditor.
                ui.spacing_mut().item_spacing.y = 0.0;
                let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
                egui::ScrollArea::both()
                    .id_salt("licenses")
                    .auto_shrink(false)
                    .show_rows(ui, row_height, lines.len(), |ui, rows| {
                        for line in &lines[rows] {
                            ui.add(
                                egui::Label::new(RichText::new(line).monospace())
                                    .extend()
                                    .selectable(true),
                            );
                        }
                    });
            });
        if !open {
            self.show_licenses = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_lines_wrap_at_spaces_and_keep_indentation() {
        assert_eq!(wrap("kurz", 10), vec!["kurz"]);
        assert_eq!(wrap("", 10), vec![""]);
        assert_eq!(
            wrap("  eins zwei drei vier", 12),
            vec!["  eins zwei", "  drei vier"]
        );
        // Ein einzelnes überlanges Wort bleibt ganz.
        assert_eq!(
            wrap("https://example.com/sehr/lang ok", 10),
            vec!["https://example.com/sehr/lang", "ok"]
        );
        assert!(
            lines()
                .iter()
                .all(|line| { line.chars().count() <= WRAP_WIDTH || !line.trim().contains(' ') })
        );
    }

    #[test]
    fn embedded_notices_cover_own_license_and_dependencies() {
        let text = lines().join("\n");
        assert!(text.contains("MIT License"));
        assert!(text.contains("Copyright (c) 2026 Stanischewski"));
        // Stichproben: egui (aus dem Repository), windows-rs, Schriftarten.
        assert!(text.contains("Copyright (c) 2018-2021 Emil Ernerfeldt"));
        assert!(text.contains("Copyright (c) Microsoft Corporation."));
        assert!(text.contains("Source Foundry Authors"));
        assert!(text.contains("BITSTREAM VERA LICENSE"));
        assert!(text.contains("SIL OPEN FONT LICENSE"));
        assert!(text.contains("UBUNTU FONT LICENCE"));
        assert!(text.contains("Copyright 2013 Google Inc."));
        // AccessKit enthält Code aus Chromium (BSD-3-Clause).
        assert!(text.contains("Copyright 2015 The Chromium Authors"));
    }
}
