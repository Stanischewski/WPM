//! Anbindung an die winget-CLI: Befehlszeilen bauen und Ausgaben auswerten.

pub mod demo;
pub mod parse;

use std::path::PathBuf;

use crate::runner::CommandOutput;
use crate::settings::{InstallMode, Scope};

/// Ein Paket, wie es in den Tabellen von `winget list/upgrade/search` erscheint.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    pub id: String,
    pub version: String,
    /// Verfügbare neuere Version (leer, wenn keine bekannt ist).
    pub available: String,
    /// Quelle, z. B. `winget` oder `msstore`; leer bei Programmen, die winget
    /// nur lokal erkannt hat (Systemsteuerung/MSIX).
    pub source: String,
}

impl Package {
    /// Ohne Quelle kann winget das Paket nicht aktualisieren oder Details zeigen.
    pub fn has_source(&self) -> bool {
        !self.source.is_empty()
    }

    pub fn has_update(&self) -> bool {
        !self.available.is_empty()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UpgradeList {
    pub upgrades: Vec<Package>,
    /// Angeheftete Pakete: Update vorhanden, wird aber nur auf ausdrücklichen
    /// Wunsch installiert.
    pub pinned: Vec<Package>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pin {
    pub name: String,
    pub id: String,
    pub version: String,
    pub source: String,
    pub pin_type: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub name: String,
    pub argument: String,
}

/// Ausgabe von `winget show`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Details {
    pub title: String,
    pub fields: Vec<(String, String)>,
}

/// Aus den Einstellungen übernommene Optionen für Installationen.
#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    pub mode: InstallMode,
    pub scope: Scope,
}

#[derive(Clone, Debug)]
pub enum Request {
    Version,
    ListInstalled,
    CheckUpdates {
        include_unknown: bool,
    },
    Search {
        query: String,
        source: Option<String>,
    },
    Show {
        id: String,
        source: String,
    },
    Install {
        package: Package,
    },
    Upgrade {
        package: Package,
    },
    Uninstall {
        package: Package,
    },
    PinAdd {
        package: Package,
    },
    PinRemove {
        id: String,
    },
    PinList,
    SourceList,
    SourceUpdate,
    Export {
        path: PathBuf,
    },
    Import {
        path: PathBuf,
    },
}

#[derive(Debug)]
pub enum Response {
    Version(String),
    Installed(Vec<Package>),
    Updates(UpgradeList),
    SearchResults(Vec<Package>),
    Details(Details),
    Pins(Vec<Pin>),
    Sources(Vec<Source>),
    /// Aktion erfolgreich, optional mit Hinweis (z. B. Neustart nötig).
    Done(Option<String>),
}

pub const NO_APPLICATIONS_FOUND: u32 = 0x8A15_0014;
pub const UPDATE_NOT_APPLICABLE: u32 = 0x8A15_002B;
pub const REBOOT_REQUIRED_TO_FINISH: u32 = 0x8A15_0109;
pub const REBOOT_INITIATED: u32 = 0x8A15_010B;

const ACCEPT_SOURCE: &str = "--accept-source-agreements";
const ACCEPT_PACKAGE: &str = "--accept-package-agreements";
const NO_PROMPTS: &str = "--disable-interactivity";

impl Request {
    /// Kommandozeilenargumente für `winget`.
    pub fn args(&self, options: &Options) -> Vec<String> {
        let mut args: Vec<String> = Vec::new();
        match self {
            Request::Version => return vec!["--version".into()],
            Request::SourceList => return vec!["source".into(), "list".into()],
            Request::SourceUpdate => return vec!["source".into(), "update".into()],
            Request::ListInstalled => push(&mut args, &["list"]),
            Request::CheckUpdates { include_unknown } => {
                push(&mut args, &["upgrade"]);
                if *include_unknown {
                    push(&mut args, &["--include-unknown"]);
                }
            }
            Request::Search { query, source } => {
                push(&mut args, &["search", "--query", query]);
                if let Some(source) = source {
                    push(&mut args, &["--source", source]);
                }
            }
            Request::Show { id, source } => {
                push(&mut args, &["show"]);
                push_id(&mut args, id);
                push_source(&mut args, source);
            }
            Request::Install { package } => {
                args.push("install".into());
                push_id(&mut args, &package.id);
                push_source(&mut args, &package.source);
                push_mode(&mut args, options.mode);
                match options.scope {
                    Scope::Any => {}
                    Scope::User => args.extend(["--scope".into(), "user".into()]),
                    Scope::Machine => args.extend(["--scope".into(), "machine".into()]),
                }
                args.push(ACCEPT_PACKAGE.into());
            }
            Request::Upgrade { package } => {
                args.push("upgrade".into());
                push_id(&mut args, &package.id);
                push_source(&mut args, &package.source);
                push_mode(&mut args, options.mode);
                args.push(ACCEPT_PACKAGE.into());
            }
            Request::Uninstall { package } => {
                args.push("uninstall".into());
                push_id(&mut args, &package.id);
                push_mode(&mut args, options.mode);
            }
            Request::PinAdd { package } => {
                args.extend(["pin".into(), "add".into()]);
                push_id(&mut args, &package.id);
                push_source(&mut args, &package.source);
            }
            Request::PinRemove { id } => {
                args.extend(["pin".into(), "remove".into()]);
                push_id(&mut args, id);
            }
            Request::PinList => push(&mut args, &["pin", "list"]),
            Request::Export { path } => {
                args.extend(["export".into(), "--output".into()]);
                args.push(path.display().to_string());
            }
            Request::Import { path } => {
                args.extend(["import".into(), "--import-file".into()]);
                args.push(path.display().to_string());
                args.extend(["--ignore-unavailable".into(), ACCEPT_PACKAGE.into()]);
            }
        }
        args.push(ACCEPT_SOURCE.into());
        args.push(NO_PROMPTS.into());
        args
    }

    /// Kurze Beschreibung für Statuszeile und Warteschlange.
    pub fn label(&self) -> String {
        match self {
            Request::Version => "winget-Version abfragen".into(),
            Request::ListInstalled => "Installierte Pakete laden".into(),
            Request::CheckUpdates { .. } => "Nach Updates suchen".into(),
            Request::Search { query, .. } => format!("Suche nach „{query}“"),
            Request::Show { id, .. } => format!("Details zu {id} laden"),
            Request::Install { package } => format!("{} installieren", package.name),
            Request::Upgrade { package } => format!("{} aktualisieren", package.name),
            Request::Uninstall { package } => format!("{} deinstallieren", package.name),
            Request::PinAdd { package } => format!("{} anheften", package.name),
            Request::PinRemove { id } => format!("{id} lösen"),
            Request::PinList => "Angeheftete Pakete laden".into(),
            Request::SourceList => "Quellen laden".into(),
            Request::SourceUpdate => "Quellen aktualisieren".into(),
            Request::Export { .. } => "Paketliste exportieren".into(),
            Request::Import { .. } => "Paketliste importieren".into(),
        }
    }

    /// Paket-ID, falls sich der Auftrag auf genau ein Paket bezieht.
    pub fn package_id(&self) -> Option<&str> {
        match self {
            Request::Install { package }
            | Request::Upgrade { package }
            | Request::Uninstall { package }
            | Request::PinAdd { package } => Some(&package.id),
            Request::PinRemove { id } => Some(id),
            _ => None,
        }
    }

    /// Ändert der Auftrag installierte Software (danach Listen neu laden)?
    pub fn changes_packages(&self) -> bool {
        matches!(
            self,
            Request::Install { .. }
                | Request::Upgrade { .. }
                | Request::Uninstall { .. }
                | Request::Import { .. }
                | Request::PinAdd { .. }
                | Request::PinRemove { .. }
        )
    }

    /// Wertet die Ausgabe von winget aus.
    pub fn interpret(&self, out: &CommandOutput) -> Result<Response, String> {
        let lines = &out.lines;
        match self {
            Request::Version => match parse::version(lines) {
                Some(version) if out.success() => Ok(Response::Version(version)),
                _ => Err(error_text(out)),
            },
            Request::ListInstalled => {
                let packages = parse::installed(lines);
                query_result(out, packages.is_empty()).map(|()| Response::Installed(packages))
            }
            Request::CheckUpdates { .. } => {
                let list = parse::upgrades(lines);
                let empty = list.upgrades.is_empty() && list.pinned.is_empty();
                query_result(out, empty).map(|()| Response::Updates(list))
            }
            Request::Search { .. } => {
                let packages = parse::search_results(lines);
                query_result(out, packages.is_empty()).map(|()| Response::SearchResults(packages))
            }
            Request::Show { .. } => match parse::details(lines) {
                Some(details) => Ok(Response::Details(details)),
                None => Err(error_text(out)),
            },
            Request::PinList => {
                let pins = parse::pins(lines);
                query_result(out, pins.is_empty()).map(|()| Response::Pins(pins))
            }
            Request::SourceList => {
                let sources = parse::sources(lines);
                query_result(out, sources.is_empty()).map(|()| Response::Sources(sources))
            }
            _ => match out.code() {
                Some(0) => Ok(Response::Done(None)),
                Some(REBOOT_REQUIRED_TO_FINISH | REBOOT_INITIATED) => Ok(Response::Done(Some(
                    "Ein Neustart ist nötig, um den Vorgang abzuschließen.".into(),
                ))),
                _ => Err(error_text(out)),
            },
        }
    }
}

/// Abfragen gelten als erfolgreich, wenn winget „nichts gefunden“ meldet oder
/// trotz Fehler (z. B. eine Quelle nicht erreichbar) Ergebnisse geliefert hat.
fn query_result(out: &CommandOutput, empty: bool) -> Result<(), String> {
    if out.success() || !empty || out.code() == Some(NO_APPLICATIONS_FOUND) {
        Ok(())
    } else {
        Err(error_text(out))
    }
}

/// Gekürzte IDs („Foo.Ba…“) lassen sich nicht exakt suchen. Ohne `--exact`
/// sucht winget nach Teilzeichenketten, was bei eindeutigem Anfang genügt.
fn push_id(args: &mut Vec<String>, id: &str) {
    args.push("--id".into());
    match id.strip_suffix('…') {
        Some(prefix) => args.push(prefix.into()),
        None => args.extend([id.into(), "--exact".into()]),
    }
}

fn push(args: &mut Vec<String>, items: &[&str]) {
    args.extend(items.iter().map(|s| s.to_string()));
}

fn push_source(args: &mut Vec<String>, source: &str) {
    if !source.is_empty() {
        args.extend(["--source".into(), source.into()]);
    }
}

fn push_mode(args: &mut Vec<String>, mode: InstallMode) {
    match mode {
        InstallMode::Default => {}
        InstallMode::Silent => args.push("--silent".into()),
        InstallMode::Interactive => args.push("--interactive".into()),
    }
}

/// Fehlermeldung aus den letzten Ausgabezeilen plus Fehlercode.
pub fn error_text(out: &CommandOutput) -> String {
    let mut tail: Vec<&str> = out
        .lines
        .iter()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.chars().all(|c| c == '-'))
        .rev()
        .take(3)
        .collect();
    tail.reverse();
    let mut message = tail.join("\n");

    if let Some(code) = out.code() {
        if !message.is_empty() {
            message.push('\n');
        }
        message.push_str(&format!("winget-Code 0x{code:08X}"));
        if let Some(hint) = describe_code(code) {
            message.push_str(&format!(" – {hint}"));
        }
    } else if message.is_empty() {
        message = "winget wurde unerwartet beendet.".into();
    }
    message
}

/// Deutsche Kurzbeschreibung bekannter winget-Fehlercodes.
pub fn describe_code(code: u32) -> Option<&'static str> {
    Some(match code {
        0x8A15_0008 => "Download fehlgeschlagen",
        0x8A15_0011 => "Prüfsumme des Installers stimmt nicht",
        NO_APPLICATIONS_FOUND => "kein passendes Paket gefunden",
        0x8A15_0016 => "mehrere Pakete passen, Auswahl nicht eindeutig",
        UPDATE_NOT_APPLICABLE => "kein anwendbares Update gefunden",
        0x8A15_0101 => "die Anwendung läuft gerade – bitte schließen",
        0x8A15_0102 => "eine andere Installation läuft bereits",
        0x8A15_0103 => "Dateien werden gerade verwendet",
        0x8A15_0104 => "eine Abhängigkeit fehlt",
        0x8A15_0105 => "nicht genügend Speicherplatz",
        0x8A15_0106 => "nicht genügend Arbeitsspeicher",
        0x8A15_0107 => "keine Internetverbindung",
        0x8A15_010A => "Neustart vor der Installation nötig",
        0x8A15_010C => "vom Benutzer abgebrochen",
        0x8A15_010D => "eine andere Version ist bereits installiert",
        0x8A15_010E => "eine neuere Version ist bereits installiert",
        0x8A15_010F => "durch Richtlinien blockiert",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(id: &str, source: &str) -> Package {
        Package {
            name: "Test".into(),
            id: id.into(),
            source: source.into(),
            ..Default::default()
        }
    }

    #[test]
    fn exact_id_for_complete_ids_and_prefix_for_truncated_ones() {
        let options = Options::default();
        let args = Request::Upgrade {
            package: package("Git.Git", "winget"),
        }
        .args(&options);
        assert_eq!(
            args,
            [
                "upgrade",
                "--id",
                "Git.Git",
                "--exact",
                "--source",
                "winget",
                "--accept-package-agreements",
                "--accept-source-agreements",
                "--disable-interactivity"
            ]
        );

        let args = Request::Uninstall {
            package: package("Microsoft.VisualStudio.2022.Comm…", ""),
        }
        .args(&options);
        assert_eq!(
            &args[..3],
            ["uninstall", "--id", "Microsoft.VisualStudio.2022.Comm"]
        );
        assert!(!args.contains(&"--exact".to_string()));
    }

    #[test]
    fn install_options_are_applied() {
        let options = Options {
            mode: InstallMode::Silent,
            scope: Scope::User,
        };
        let args = Request::Install {
            package: package("VideoLAN.VLC", "winget"),
        }
        .args(&options);
        assert!(args.contains(&"--silent".to_string()));
        assert!(args.windows(2).any(|w| w == ["--scope", "user"]));
    }

    #[test]
    fn no_results_is_not_an_error() {
        let out = CommandOutput {
            lines: vec!["Kein installiertes Paket entspricht den Eingabekriterien.".into()],
            exit_code: Some(NO_APPLICATIONS_FOUND as i32),
        };
        let response = Request::CheckUpdates {
            include_unknown: false,
        }
        .interpret(&out);
        assert!(matches!(response, Ok(Response::Updates(list)) if list.upgrades.is_empty()));
    }

    #[test]
    fn failed_action_reports_last_lines_and_code() {
        let out = CommandOutput {
            lines: vec![
                "Paketinstallation wird gestartet...".into(),
                "Installer failed with exit code: 1603".into(),
                "".into(),
            ],
            exit_code: Some(0x8A15_0101_u32 as i32),
        };
        let error = Request::Upgrade {
            package: package("A.B", "winget"),
        }
        .interpret(&out)
        .unwrap_err();
        assert!(error.contains("1603"));
        assert!(error.contains("0x8A150101"));
        assert!(error.contains("läuft gerade"));
    }
}
