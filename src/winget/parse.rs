//! Parser für die Textausgabe der winget-CLI.
//!
//! winget bietet keine maschinenlesbare Ausgabe. Die Tabellen sind aber
//! spaltenbündig: Die Kopfzeile steht direkt über einer Linie aus `-` und gibt
//! die Startposition jeder Spalte vor. Da die Spaltennamen in der Sprache von
//! Windows ausgegeben werden, wird nur mit Position und Anzahl gearbeitet.
//! Positionen sind Anzeigebreiten (CJK-Zeichen zählen doppelt), so wie winget
//! selbst ausrichtet.

use unicode_width::UnicodeWidthChar;

use super::{Details, Package, Pin, Source, UpgradeList};

/// Eine Tabelle aus der Ausgabe.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Table {
    pub columns: usize,
    pub rows: Vec<Vec<String>>,
    /// Letzte Textzeile vor der Kopfzeile (z. B. die Überschrift der
    /// Tabelle mit angehefteten Paketen bei `winget upgrade`).
    pub preamble: String,
}

pub fn tables(lines: &[String]) -> Vec<Table> {
    let separators: Vec<usize> = (1..lines.len())
        .filter(|&i| is_separator(&lines[i]))
        .collect();

    let mut tables = Vec::new();
    let mut region_start = 0;
    for (n, &sep) in separators.iter().enumerate() {
        let header = sep - 1;
        let starts = column_starts(&lines[header]);
        // Die Tabelle endet vor der Kopfzeile der nächsten Tabelle.
        let end = separators.get(n + 1).map_or(lines.len(), |&next| next - 1);
        let preamble = lines[region_start..header]
            .iter()
            .rev()
            .map(|l| l.trim())
            .find(|l| !l.is_empty())
            .unwrap_or_default()
            .to_string();
        region_start = sep + 1;
        if starts.is_empty() {
            continue;
        }
        let candidates: Vec<&str> = lines[sep + 1..end.max(sep + 1)]
            .iter()
            .map(String::as_str)
            .filter(|line| !is_prose(line))
            .collect();
        let starts = merge_spanned_columns(&lines[header], starts, &candidates);
        let rows = candidates
            .iter()
            .filter_map(|line| split_row(line, &starts))
            .collect();
        tables.push(Table {
            columns: starts.len(),
            rows,
            preamble,
        });
    }
    tables
}

fn is_separator(line: &str) -> bool {
    let line = line.trim();
    line.len() >= 10 && line.chars().all(|c| c == '-')
}

/// Fließtext (z. B. „3 Aktualisierungen verfügbar.“) statt einer Tabellenzeile.
fn is_prose(line: &str) -> bool {
    let line = line.trim();
    line.is_empty() || line.ends_with(['.', ':', '。', '：', '!'])
}

fn column_starts(header: &str) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut pos = 0;
    let mut previous_blank = true;
    for ch in header.chars() {
        let blank = ch.is_whitespace();
        if !blank && previous_blank {
            starts.push(pos);
        }
        previous_blank = blank;
        pos += ch.width().unwrap_or(0);
    }
    starts
}

/// Zeichen an einer Anzeigeposition.
fn char_at(line: &str, target: usize) -> Option<char> {
    let mut pos = 0;
    for ch in line.chars() {
        let width = ch.width().unwrap_or(0);
        if (pos..pos + width).contains(&target) {
            return Some(ch);
        }
        pos += width;
        if pos > target {
            break;
        }
    }
    None
}

/// Mehrwortige Überschriften (z. B. „Pin type“) sehen wie zwei Spalten aus.
/// Steht vor einem Spaltenanfang nur ein Leerzeichen und laufen die Werte der
/// meisten Zeilen über diese Grenze hinweg, ist es keine echte Spalte.
fn merge_spanned_columns(header: &str, starts: Vec<usize>, rows: &[&str]) -> Vec<usize> {
    let mut merged = Vec::with_capacity(starts.len());
    for (index, &start) in starts.iter().enumerate() {
        let single_gap =
            start >= 2 && char_at(header, start - 2).is_some_and(|c| !c.is_whitespace());
        if index > 0 && single_gap {
            let reaching: Vec<char> = rows
                .iter()
                .filter_map(|row| char_at(row, start - 1))
                .collect();
            let spanning = reaching.iter().filter(|c| !c.is_whitespace()).count();
            if spanning * 2 > reaching.len() {
                continue;
            }
        }
        merged.push(start);
    }
    merged
}

/// Schneidet eine Zeile an den Spaltengrenzen. Liefert `None`, wenn die Zeile
/// nicht zur Tabelle passt (vor jedem Spaltenanfang muss Leerraum stehen).
fn split_row(line: &str, starts: &[usize]) -> Option<Vec<String>> {
    let mut cells = vec![String::new(); starts.len()];
    let mut pos = 0;
    let mut col = 0;
    for ch in line.chars() {
        let width = ch.width().unwrap_or(0);
        while col + 1 < starts.len() && pos >= starts[col + 1] {
            col += 1;
        }
        if let Some(&next) = starts.get(col + 1) {
            let end = pos + width;
            if end > next || (end == next && !ch.is_whitespace()) {
                return None;
            }
        }
        cells[col].push(ch);
        pos += width;
    }
    Some(cells.into_iter().map(|c| c.trim().to_string()).collect())
}

/// IDs aus Quellen enthalten keine Leerzeichen, lokal erkannte Programme
/// (`ARP\Machine\X64\…`, `MSIX\…`) dagegen schon.
fn looks_like_id(id: &str) -> bool {
    !id.is_empty() && (id.contains('\\') || !id.contains(char::is_whitespace))
}

/// Spaltenzuordnung: Index von (verfügbar, Quelle) je nach Spaltenzahl.
fn package_from_row(
    row: &[String],
    available: Option<usize>,
    source: Option<usize>,
) -> Option<Package> {
    let cell = |i: Option<usize>| i.and_then(|i| row.get(i)).cloned().unwrap_or_default();
    let package = Package {
        name: row.first()?.clone(),
        id: row.get(1)?.clone(),
        version: row.get(2)?.clone(),
        available: cell(available),
        source: cell(source),
    };
    (looks_like_id(&package.id) && !package.version.is_empty()).then_some(package)
}

/// `winget list`: Name, ID, Version, [Verfügbar], [Quelle]
///
/// Die Quelle ist immer die letzte Spalte. So bleibt die Zuordnung auch dann
/// richtig, wenn eine mehrwortige Überschrift eine leere Zusatzspalte erzeugt.
pub fn installed(lines: &[String]) -> Vec<Package> {
    tables(lines)
        .into_iter()
        .flat_map(|table| {
            let last = table.columns.saturating_sub(1);
            let (available, source) = match table.columns {
                0..=3 => (None, None),
                4 => (None, Some(3)),
                _ => (Some(3), Some(last)),
            };
            table
                .rows
                .into_iter()
                .filter_map(move |row| package_from_row(&row, available, source))
        })
        .collect()
}

/// `winget upgrade`: Name, ID, Version, Verfügbar, [Quelle]. Eine zweite
/// Tabelle enthält Pakete, die nur bei expliziter Auswahl aktualisiert werden.
pub fn upgrades(lines: &[String]) -> UpgradeList {
    let mut list = UpgradeList::default();
    for (index, table) in tables(lines).into_iter().enumerate() {
        let source = (table.columns >= 5).then(|| table.columns - 1);
        let pinned = index > 0 || table.preamble.ends_with([':', '：']);
        let packages = table
            .rows
            .iter()
            .filter_map(|row| package_from_row(row, Some(3), source))
            .filter(Package::has_update);
        if pinned {
            list.pinned.extend(packages);
        } else {
            list.upgrades.extend(packages);
        }
    }
    list
}

/// `winget search`: Name, ID, Version, [Übereinstimmung], [Quelle]
pub fn search_results(lines: &[String]) -> Vec<Package> {
    tables(lines)
        .into_iter()
        .flat_map(|table| {
            let source = (table.columns >= 4).then(|| table.columns - 1);
            table
                .rows
                .into_iter()
                .filter_map(move |row| package_from_row(&row, None, source))
        })
        .collect()
}

/// `winget pin list`: Name, ID, Version, Quelle, Pin-Typ, [Version angeheftet]
pub fn pins(lines: &[String]) -> Vec<Pin> {
    tables(lines)
        .into_iter()
        .flat_map(|table| table.rows)
        .filter(|row| row.len() >= 4 && looks_like_id(&row[1]))
        .map(|row| Pin {
            name: row[0].clone(),
            id: row[1].clone(),
            version: row[2].clone(),
            source: row[3].clone(),
            pin_type: row.get(4).cloned().unwrap_or_default(),
        })
        .collect()
}

/// `winget source list`: Name, Argument, [Explizit]
pub fn sources(lines: &[String]) -> Vec<Source> {
    tables(lines)
        .into_iter()
        .flat_map(|table| table.rows)
        .filter(|row| row.len() >= 2 && !row[0].is_empty() && !row[1].is_empty())
        .map(|row| Source {
            name: row[0].clone(),
            argument: row[1].clone(),
        })
        .collect()
}

/// `winget --version` gibt z. B. `v1.9.25200` aus.
pub fn version(lines: &[String]) -> Option<String> {
    lines
        .iter()
        .map(|l| l.trim())
        .find(|l| l.starts_with('v') && l[1..].starts_with(|c: char| c.is_ascii_digit()))
        .map(str::to_string)
}

/// `winget show`: erste Zeile „Gefunden Name [ID]“, danach „Schlüssel: Wert“,
/// eingerückte Zeilen gehören zum vorherigen Feld.
pub fn details(lines: &[String]) -> Option<Details> {
    let start = lines
        .iter()
        .position(|l| l.trim_end().ends_with(']') && l.contains(" ["))?;
    let mut details = Details {
        title: lines[start].trim().to_string(),
        fields: Vec::new(),
    };
    for line in &lines[start + 1..] {
        if line.trim().is_empty() {
            continue;
        }
        let indented = line.starts_with(char::is_whitespace);
        let field = if indented { None } else { split_field(line) };
        match (field, details.fields.last_mut()) {
            (Some((key, value)), _) => details.fields.push((key, value)),
            (None, Some((_, value))) => {
                if !value.is_empty() {
                    value.push('\n');
                }
                value.push_str(line.strip_prefix("  ").unwrap_or(line).trim_end());
            }
            (None, None) => details
                .fields
                .push((String::new(), line.trim().to_string())),
        }
    }
    Some(details)
}

fn split_field(line: &str) -> Option<(String, String)> {
    let line = line.trim_end();
    if let Some((key, value)) = line.split_once(": ") {
        return Some((key.trim().to_string(), value.trim().to_string()));
    }
    line.strip_suffix(':')
        .map(|key| (key.trim().to_string(), String::new()))
}

/// Erzeugt eine Tabelle im Stil von winget (für den Demo-Modus und Tests).
/// Ist die Tabelle breiter als `max_width`, wird die erste Spalte mit „…“
/// gekürzt – wie winget es tut, wenn die Ausgabe nicht in eine Konsole geht.
pub fn format_table(headers: &[&str], rows: &[Vec<String>], max_width: usize) -> String {
    let display_width = |s: &str| s.chars().map(|c| c.width().unwrap_or(0)).sum::<usize>();
    let mut widths: Vec<usize> = headers.iter().map(|h| display_width(h)).collect();
    for row in rows {
        for (w, cell) in widths.iter_mut().zip(row) {
            *w = (*w).max(display_width(cell));
        }
    }
    let total = widths.iter().sum::<usize>() + widths.len().saturating_sub(1);
    if total > max_width {
        widths[0] = widths[0]
            .saturating_sub(total - max_width)
            .max(display_width(headers[0]) + 1);
    }

    let fit = |text: &str, width: usize| -> String {
        if display_width(text) <= width {
            return text.to_string();
        }
        let mut out = String::new();
        let mut used = 0;
        for c in text.chars() {
            let w = c.width().unwrap_or(0);
            if used + w + 1 > width {
                break;
            }
            out.push(c);
            used += w;
        }
        out.push('…');
        out
    };
    let format_row = |cells: Vec<String>| -> String {
        let mut line = String::new();
        for (i, (cell, &width)) in cells.iter().zip(&widths).enumerate() {
            let cell = fit(cell, width);
            line.push_str(&cell);
            if i + 1 < widths.len() {
                line.push_str(&" ".repeat(width - display_width(&cell) + 1));
            }
        }
        line.trim_end().to_string()
    };

    let mut out = format_row(headers.iter().map(|h| h.to_string()).collect());
    out.push('\n');
    out.push_str(&"-".repeat(widths.iter().sum::<usize>() + widths.len() - 1));
    out.push('\n');
    for row in rows {
        out.push_str(&format_row(row.clone()));
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_string).collect()
    }

    const LIST_DE: &str = r#"Name                                   ID                                       Version        Verfügbar     Quelle
-------------------------------------------------------------------------------------------------------------------
7-Zip 24.08 (x64)                      7zip.7zip                                24.08          24.09         winget
Druckerhilfe XY                        ARP\Machine\X64\Druckerhilfe XY_is1      1.0
Microsoft Visual C++ 2015-2022 Redist… Microsoft.VCRedist.2015+.x64             14.40.33810.0  14.42.34433.0 winget
Microsoft Edge                         Microsoft.Edge                           130.0.2849.56                winget
Microsoft Teams                        MSIX\MSTeams_24277.3507.3205.5228_x64__… 24277.3507.32…
"#;

    #[test]
    fn parses_installed_packages_with_localized_headers() {
        let packages = installed(&lines(LIST_DE));
        assert_eq!(packages.len(), 5);
        assert_eq!(
            packages[0],
            Package {
                name: "7-Zip 24.08 (x64)".into(),
                id: "7zip.7zip".into(),
                version: "24.08".into(),
                available: "24.09".into(),
                source: "winget".into(),
            }
        );
        // Lokal erkannte Programme: ID mit Leerzeichen, keine Quelle.
        assert_eq!(packages[1].id, r"ARP\Machine\X64\Druckerhilfe XY_is1");
        assert_eq!(packages[1].source, "");
        assert_eq!(packages[2].name, "Microsoft Visual C++ 2015-2022 Redist…");
        assert_eq!(packages[3].available, "");
        assert_eq!(packages[4].id, r"MSIX\MSTeams_24277.3507.3205.5228_x64__…");
    }

    #[test]
    fn list_without_available_column() {
        let text = "\
Name      Id        Version Source
-----------------------------------
Git       Git.Git   2.47.1  winget
";
        let packages = installed(&lines(text));
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].source, "winget");
        assert_eq!(packages[0].available, "");
    }

    #[test]
    fn multi_word_headers_do_not_shift_the_source_column() {
        // Lange Werte laufen über die Wortgrenze: Spalten werden zusammengelegt.
        let text = "\
Name Id      Version Neue Version Quelle
-----------------------------------------
Git  Git.Git 2.46.0  2.47.1       winget
";
        let list = upgrades(&lines(text));
        assert_eq!(list.upgrades[0].available, "2.47.1");
        assert_eq!(list.upgrades[0].source, "winget");

        // Kurze Werte: eine leere Zusatzspalte entsteht, die Quelle bleibt die letzte.
        let text = "\
Name Id      Version Neue Version Quelle
-----------------------------------------
Git  Git.Git 2.4     2.5          winget
";
        let packages = installed(&lines(text));
        assert_eq!(packages[0].available, "2.5");
        assert_eq!(packages[0].source, "winget");
    }

    #[test]
    fn upgrades_with_progress_noise_footer_and_pinned_table() {
        let text = "\
Name               Id                  Version Available Source
---------------------------------------------------------------
Git                Git.Git             2.46.0  2.47.1    winget
PowerToys (Previe… Microsoft.PowerToys 0.85.1  0.86.0    winget
2 upgrades available.

The following packages have an upgrade available, but require explicit targeting for upgrade:
Name    Id              Version  Available Source
--------------------------------------------------
Discord Discord.Discord 1.0.9166 1.0.9171  winget
";
        let list = upgrades(&lines(text));
        assert_eq!(list.upgrades.len(), 2, "{list:?}");
        assert_eq!(list.upgrades[1].id, "Microsoft.PowerToys");
        assert_eq!(list.upgrades[1].available, "0.86.0");
        assert_eq!(list.pinned.len(), 1);
        assert_eq!(list.pinned[0].id, "Discord.Discord");
    }

    #[test]
    fn only_pinned_upgrades() {
        let text = "\
No installed package found matching input criteria.
The following packages have an upgrade available, but require explicit targeting for upgrade:
Name    Id              Version  Available Source
--------------------------------------------------
Discord Discord.Discord 1.0.9166 1.0.9171  winget
";
        let list = upgrades(&lines(text));
        assert!(list.upgrades.is_empty());
        assert_eq!(list.pinned.len(), 1);
    }

    #[test]
    fn long_footer_text_is_not_mistaken_for_a_row() {
        let text = "\
Name Id      Version Available Source
--------------------------------------
Git  Git.Git 2.46.0  2.47.1    winget
1 package(s) have version numbers that cannot be determined. Use --include-unknown to see all results.
";
        assert_eq!(upgrades(&lines(text)).upgrades.len(), 1);
    }

    #[test]
    fn wide_characters_are_aligned_by_display_width() {
        let text = "\
名前         ID              バージョン 一致         ソース
--------------------------------------------------------------
メモ帳アプリ Example.Memo    1.2.3      Tag: memo    winget
Firefox      Mozilla.Firefox 132.0.2                 winget
";
        let results = search_results(&lines(text));
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].name, "メモ帳アプリ");
        assert_eq!(results[0].id, "Example.Memo");
        assert_eq!(results[0].version, "1.2.3");
        assert_eq!(results[0].source, "winget");
        assert_eq!(results[1].source, "winget");
    }

    #[test]
    fn no_table_means_no_results() {
        assert!(search_results(&lines("No package found matching input criteria.")).is_empty());
    }

    #[test]
    fn parses_pins_and_sources() {
        let pins_text = "\
Name    Id              Version  Source Pin type
-------------------------------------------------
Discord Discord.Discord 1.0.9166 winget Pinning
";
        let pins = pins(&lines(pins_text));
        assert_eq!(pins.len(), 1);
        assert_eq!(pins[0].pin_type, "Pinning");

        let sources_text = "\
Name    Argument                                      Explicit
--------------------------------------------------------------
msstore https://storeedgefd.dsx.mp.microsoft.com/v9.0 false
winget  https://cdn.winget.microsoft.com/cache        false
";
        let sources = sources(&lines(sources_text));
        assert_eq!(sources.len(), 2);
        assert_eq!(
            sources[1].argument,
            "https://cdn.winget.microsoft.com/cache"
        );
    }

    #[test]
    fn parses_show_output() {
        let text = "\
Gefunden Visual Studio Code [Microsoft.VisualStudioCode]
Version: 1.95.3
Herausgeber: Microsoft Corporation
Beschreibung: Ein Editor.
  Zweite Zeile.
Startseite: https://code.visualstudio.com
Markierungen:
  developer-tools
  editor
Installationsprogramm:
  Installertyp: inno
";
        let details = details(&lines(text)).unwrap();
        assert_eq!(
            details.title,
            "Gefunden Visual Studio Code [Microsoft.VisualStudioCode]"
        );
        assert_eq!(details.fields[0], ("Version".into(), "1.95.3".into()));
        assert_eq!(details.fields[2].1, "Ein Editor.\nZweite Zeile.");
        assert_eq!(details.fields[3].1, "https://code.visualstudio.com");
        assert_eq!(
            details.fields[4],
            ("Markierungen".into(), "developer-tools\neditor".into())
        );
        assert_eq!(details.fields[5].1, "Installertyp: inno");
    }

    #[test]
    fn parses_version() {
        assert_eq!(version(&lines("\nv1.9.25200\n")), Some("v1.9.25200".into()));
        assert_eq!(version(&lines("Fehler")), None);
    }

    #[test]
    fn format_table_round_trips_and_truncates_like_winget() {
        let rows = vec![
            vec![
                "Ein sehr langer Paketname, der gekürzt werden muss".into(),
                "Foo.Bar".into(),
                "1.0".into(),
            ],
            vec!["Kurz".into(), "Baz.Qux".into(), "2.0".into()],
        ];
        let text = format_table(&["Name", "Id", "Version"], &rows, 30);
        assert!(text.lines().all(|l| l.chars().count() <= 30), "{text}");
        let packages = installed(&lines(&text));
        assert_eq!(packages.len(), 2);
        assert!(packages[0].name.ends_with('…'));
        assert_eq!(packages[0].id, "Foo.Bar");
        assert_eq!(packages[1].version, "2.0");
    }
}
