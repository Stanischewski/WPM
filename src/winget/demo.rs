//! Demo-Modus (`wpm --demo`): simuliert winget mit Beispieldaten.
//!
//! Die Antworten werden als Text im Format von winget erzeugt und laufen
//! durch denselben Parser wie echte Ausgaben.

use std::io;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use super::parse::format_table;
use super::{NO_APPLICATIONS_FOUND, Package};
use crate::runner::{CommandOutput, LineSplitter, OutputEvent, Runner};

struct CatalogEntry {
    name: &'static str,
    id: &'static str,
    version: &'static str,
    tag: &'static str,
}

const CATALOG: &[CatalogEntry] = &[
    entry("7-Zip", "7zip.7zip", "24.09", "archive"),
    entry("Audacity", "Audacity.Audacity", "3.7.0", "audio"),
    entry("Blender", "BlenderFoundation.Blender", "4.2.3", "3d"),
    entry("Discord", "Discord.Discord", "1.0.9171", "chat"),
    entry("Everything", "voidtools.Everything", "1.4.1.1026", "search"),
    entry("GIMP", "GIMP.GIMP", "2.10.38", "image"),
    entry("Git", "Git.Git", "2.47.1", "vcs"),
    entry("Inkscape", "Inkscape.Inkscape", "1.4.0", "image"),
    entry("KeePassXC", "KeePassXCTeam.KeePassXC", "2.7.9", "password"),
    entry(
        "LibreOffice",
        "TheDocumentFoundation.LibreOffice",
        "24.8.3.2",
        "office",
    ),
    entry(
        "Microsoft Edge",
        "Microsoft.Edge",
        "130.0.2849.56",
        "browser",
    ),
    entry(
        "Microsoft Visual C++ 2015-2022 Redistributable (x64)",
        "Microsoft.VCRedist.2015+.x64",
        "14.42.34433.0",
        "runtime",
    ),
    entry(
        "Mozilla Firefox (de)",
        "Mozilla.Firefox.de",
        "132.0.2",
        "browser",
    ),
    entry(
        "Mozilla Thunderbird",
        "Mozilla.Thunderbird",
        "128.4.3",
        "mail",
    ),
    entry("Notepad++", "Notepad++.Notepad++", "8.7", "editor"),
    entry("OBS Studio", "OBSProject.OBSStudio", "30.2.3", "video"),
    entry("Paint.NET", "dotPDN.PaintDotNet", "5.0.13", "image"),
    entry(
        "PowerToys (Preview)",
        "Microsoft.PowerToys",
        "0.86.0",
        "utilities",
    ),
    entry("PuTTY", "PuTTY.PuTTY", "0.81.0.0", "ssh"),
    entry("Python 3.13", "Python.Python.3.13", "3.13.0", "python"),
    entry(
        "Rustup: the Rust toolchain installer",
        "Rustlang.Rustup",
        "1.27.1",
        "rust",
    ),
    entry("Spotify", "Spotify.Spotify", "1.2.48.405", "music"),
    entry("Steam", "Valve.Steam", "2.10.91.91", "games"),
    entry(
        "Visual Studio Code",
        "Microsoft.VisualStudioCode",
        "1.95.3",
        "editor",
    ),
    entry("VLC media player", "VideoLAN.VLC", "3.0.21", "video"),
    entry(
        "Windows Terminal",
        "Microsoft.WindowsTerminal",
        "1.21.2911.0",
        "terminal",
    ),
    entry("WinSCP", "WinSCP.WinSCP", "6.3.5", "ssh"),
];

const fn entry(
    name: &'static str,
    id: &'static str,
    version: &'static str,
    tag: &'static str,
) -> CatalogEntry {
    CatalogEntry {
        name,
        id,
        version,
        tag,
    }
}

fn catalog(id: &str) -> Option<&'static CatalogEntry> {
    CATALOG.iter().find(|e| e.id.eq_ignore_ascii_case(id))
}

struct DemoState {
    installed: Vec<Package>,
    pinned: Vec<String>,
}

pub struct DemoRunner {
    state: Mutex<DemoState>,
    cancelled: AtomicBool,
}

impl Default for DemoRunner {
    fn default() -> Self {
        let pkg = |name: &str, id: &str, version: &str, source: &str| {
            let available = catalog(id)
                .map(|e| e.version)
                .filter(|v| *v != version)
                .unwrap_or_default();
            Package {
                name: name.into(),
                id: id.into(),
                version: version.into(),
                available: available.into(),
                source: source.into(),
            }
        };
        let installed = vec![
            pkg("7-Zip 24.08 (x64)", "7zip.7zip", "24.08", "winget"),
            pkg("Discord", "Discord.Discord", "1.0.9166", "winget"),
            pkg(
                "Druckerhilfe XY",
                "ARP\\Machine\\X64\\Druckerhilfe XY_is1",
                "1.0",
                "",
            ),
            pkg("Git", "Git.Git", "2.46.0", "winget"),
            pkg(
                "Microsoft Edge",
                "Microsoft.Edge",
                "130.0.2849.56",
                "winget",
            ),
            pkg(
                "Microsoft Teams",
                "MSIX\\MSTeams_24277.3507.3205.5228_x64__8wekyb3d8bbwe",
                "24277.3507.3205.5228",
                "",
            ),
            pkg(
                "Microsoft Visual C++ 2015-2022 Redistributable (x64) - 14.40.33810",
                "Microsoft.VCRedist.2015+.x64",
                "14.40.33810.0",
                "winget",
            ),
            pkg(
                "Microsoft Visual Studio Code (User)",
                "Microsoft.VisualStudioCode",
                "1.94.2",
                "winget",
            ),
            pkg(
                "Mozilla Firefox (x64 de)",
                "Mozilla.Firefox.de",
                "131.0.3",
                "winget",
            ),
            pkg(
                "Notepad++ (64-bit x64)",
                "Notepad++.Notepad++",
                "8.7",
                "winget",
            ),
            pkg(
                "PowerToys (Preview) x64",
                "Microsoft.PowerToys",
                "0.85.1",
                "winget",
            ),
            pkg(
                "Rustup: the Rust toolchain installer",
                "Rustlang.Rustup",
                "1.27.1",
                "winget",
            ),
            pkg("Spotify", "Spotify.Spotify", "1.2.48.405", "winget"),
            pkg("Steam", "Valve.Steam", "2.10.91.91", "winget"),
            pkg("VLC media player", "VideoLAN.VLC", "3.0.21", "winget"),
            pkg(
                "Windows Terminal",
                "Microsoft.WindowsTerminal",
                "1.21.2911.0",
                "winget",
            ),
        ];
        Self {
            state: Mutex::new(DemoState {
                installed,
                pinned: vec!["Discord.Discord".into()],
            }),
            cancelled: AtomicBool::new(false),
        }
    }
}

/// Antwort eines simulierten Befehls: Text (darf `\r` enthalten) und Exitcode.
struct Reply {
    text: String,
    code: i32,
}

impl Reply {
    fn ok(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            code: 0,
        }
    }

    fn not_found(text: &str) -> Self {
        Self {
            text: format!("{text}\n"),
            code: NO_APPLICATIONS_FOUND as i32,
        }
    }
}

fn arg_value<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

/// Sucht wie winget: exakte ID oder (bei gekürzten IDs) Teilstring.
fn find_index(packages: &[Package], args: &[String]) -> Option<usize> {
    let id = arg_value(args, "--id")?;
    if args.iter().any(|a| a == "--exact") {
        packages.iter().position(|p| p.id == id)
    } else {
        let id = id.to_lowercase();
        packages
            .iter()
            .position(|p| p.id.to_lowercase().contains(&id))
    }
}

fn package_row(p: &Package, with_available: bool) -> Vec<String> {
    let mut row = vec![p.name.clone(), p.id.clone(), p.version.clone()];
    if with_available {
        row.push(p.available.clone());
    }
    row.push(p.source.clone());
    row
}

impl DemoRunner {
    fn sleep(&self, ms: u64) -> bool {
        thread::sleep(Duration::from_millis(ms));
        !self.cancelled.load(Ordering::Relaxed)
    }

    fn respond(&self, args: &[String], emit: &mut dyn FnMut(&str)) -> Option<Reply> {
        let command: Vec<&str> = args
            .iter()
            .map(String::as_str)
            .take_while(|a| !a.starts_with("--"))
            .collect();
        let first = args.first().map(String::as_str).unwrap_or_default();

        // Wie das echte winget ohne Konsole: kein Spinner, kein Fortschrittsbalken,
        // nur etwas Wartezeit.
        if first != "--version" && !self.sleep(450) {
            return None;
        }

        let mut state = self.state.lock().unwrap();
        let reply = match command.as_slice() {
            [] if first == "--version" => Reply::ok("v1.11.430 (Demo)\n"),
            ["list"] => {
                let rows: Vec<_> = state
                    .installed
                    .iter()
                    .map(|p| package_row(p, true))
                    .collect();
                Reply::ok(format_table(
                    &["Name", "Id", "Version", "Available", "Source"],
                    &rows,
                    120,
                ))
            }
            ["upgrade"] if !args.iter().any(|a| a == "--id") => {
                let (pinned, normal): (Vec<&Package>, Vec<&Package>) = state
                    .installed
                    .iter()
                    .filter(|p| p.has_update())
                    .partition(|p| state.pinned.contains(&p.id));
                let headers = ["Name", "Id", "Version", "Available", "Source"];
                let mut text = String::new();
                if normal.is_empty() {
                    text.push_str("No installed package found matching input criteria.\n");
                } else {
                    let rows: Vec<_> = normal.iter().map(|p| package_row(p, true)).collect();
                    text.push_str(&format_table(&headers, &rows, 120));
                    text.push_str(&format!("{} upgrades available.\n", rows.len()));
                }
                if !pinned.is_empty() {
                    text.push_str("\nThe following packages have an upgrade available, but require explicit targeting for upgrade:\n");
                    let rows: Vec<_> = pinned.iter().map(|p| package_row(p, true)).collect();
                    text.push_str(&format_table(&headers, &rows, 120));
                }
                Reply::ok(text)
            }
            ["search"] => {
                let query = arg_value(args, "--query")
                    .unwrap_or_default()
                    .to_lowercase();
                let hits: Vec<(&CatalogEntry, bool)> = CATALOG
                    .iter()
                    .filter_map(|e| {
                        if e.name.to_lowercase().contains(&query)
                            || e.id.to_lowercase().contains(&query)
                        {
                            Some((e, false))
                        } else {
                            (e.tag == query).then_some((e, true))
                        }
                    })
                    .collect();
                if hits.is_empty() {
                    Reply::not_found("No package found matching input criteria.")
                } else if hits.iter().any(|(_, by_tag)| *by_tag) {
                    let rows: Vec<_> = hits
                        .iter()
                        .map(|(e, by_tag)| {
                            let matched = if *by_tag {
                                format!("Tag: {}", e.tag)
                            } else {
                                String::new()
                            };
                            vec![
                                e.name.into(),
                                e.id.into(),
                                e.version.into(),
                                matched,
                                "winget".into(),
                            ]
                        })
                        .collect();
                    Reply::ok(format_table(
                        &["Name", "Id", "Version", "Match", "Source"],
                        &rows,
                        120,
                    ))
                } else {
                    let rows: Vec<_> = hits
                        .iter()
                        .map(|(e, _)| {
                            vec![
                                e.name.into(),
                                e.id.into(),
                                e.version.into(),
                                "winget".into(),
                            ]
                        })
                        .collect();
                    Reply::ok(format_table(
                        &["Name", "Id", "Version", "Source"],
                        &rows,
                        120,
                    ))
                }
            }
            ["show"] => {
                let id = arg_value(args, "--id").unwrap_or_default().to_lowercase();
                match CATALOG
                    .iter()
                    .find(|e| e.id.to_lowercase().starts_with(&id))
                {
                    Some(e) => Reply::ok(format!(
                        "Found {name} [{id}]\nVersion: {version}\nPublisher: Beispiel-Herausgeber\n\
                         Description: Dies ist eine Beispielbeschreibung für {name}.\n  \
                         Im Demo-Modus werden keine echten Daten abgefragt.\n\
                         Homepage: https://example.org/{tag}\nLicense: Demo-Lizenz\n\
                         Tags:\n  {tag}\n  demo\nInstaller:\n  Installer Type: exe\n  \
                         Installer Url: https://example.org/download/{id}.exe\n",
                        name = e.name,
                        id = e.id,
                        version = e.version,
                        tag = e.tag,
                    )),
                    None => Reply::not_found("No package found matching input criteria."),
                }
            }
            ["install"] | ["upgrade"] => {
                let id = arg_value(args, "--id").unwrap_or_default().to_string();
                let Some(entry) = CATALOG
                    .iter()
                    .find(|e| e.id.to_lowercase().contains(&id.to_lowercase()))
                else {
                    return Some(Reply::not_found(
                        "No package found matching input criteria.",
                    ));
                };
                let index = find_index(&state.installed, args);
                if command[0] == "upgrade" && index.is_none_or(|i| !state.installed[i].has_update())
                {
                    return Some(Reply {
                        text: "No available upgrade found.\nNo newer package versions are available from the configured sources.\n".into(),
                        code: super::UPDATE_NOT_APPLICABLE as i32,
                    });
                }
                drop(state);
                emit(&format!(
                    "Found {} [{}] Version {}\n",
                    entry.name, entry.id, entry.version
                ));
                emit("This application is licensed to you by its owner.\n");
                emit(
                    "Microsoft is not responsible for, nor does it grant any licenses to, third-party packages.\n",
                );
                emit(&format!(
                    "Downloading https://example.org/download/{}.exe\n",
                    entry.id
                ));
                if !self.sleep(1800) {
                    return None;
                }
                emit("Successfully verified installer hash\nStarting package install...\n");
                if !self.sleep(1500) {
                    return None;
                }
                // Systemweite Installer brauchen eine Windows-Bestätigung; ohne
                // Adminrechte simuliert die Demo, dass sie abgelehnt wurde.
                if entry.id.starts_with("Microsoft.VCRedist") && !crate::system::is_elevated() {
                    return Some(Reply {
                        text: "Installer failed with exit code: 1602\n\
                               Installer log is available at: C:\\Users\\demo\\AppData\\Local\\Temp\\vcredist.log\n"
                            .into(),
                        code: super::INSTALL_CANCELLED_BY_USER as i32,
                    });
                }
                state = self.state.lock().unwrap();
                match find_index(&state.installed, args) {
                    Some(i) => {
                        let p = &mut state.installed[i];
                        p.version = entry.version.into();
                        p.available.clear();
                    }
                    None => state.installed.push(Package {
                        name: entry.name.into(),
                        id: entry.id.into(),
                        version: entry.version.into(),
                        available: String::new(),
                        source: "winget".into(),
                    }),
                }
                Reply::ok("Successfully installed\n")
            }
            ["uninstall"] => match find_index(&state.installed, args) {
                Some(_) => {
                    drop(state);
                    emit("Starting package uninstall...\n");
                    if !self.sleep(1200) {
                        return None;
                    }
                    state = self.state.lock().unwrap();
                    if let Some(i) = find_index(&state.installed, args) {
                        state.installed.remove(i);
                    }
                    Reply::ok("Successfully uninstalled\n")
                }
                None => Reply::not_found("No installed package found matching input criteria."),
            },
            ["pin", "list"] => {
                let rows: Vec<Vec<String>> = state
                    .installed
                    .iter()
                    .filter(|p| state.pinned.contains(&p.id))
                    .map(|p| {
                        vec![
                            p.name.clone(),
                            p.id.clone(),
                            p.version.clone(),
                            p.source.clone(),
                            "Pinning".into(),
                        ]
                    })
                    .collect();
                if rows.is_empty() {
                    Reply::ok("There are no pins configured.\n")
                } else {
                    Reply::ok(format_table(
                        &["Name", "Id", "Version", "Source", "Pin type"],
                        &rows,
                        120,
                    ))
                }
            }
            ["pin", "add"] => match find_index(&state.installed, args) {
                Some(i) => {
                    let id = state.installed[i].id.clone();
                    state.pinned.push(id);
                    Reply::ok("Pin added successfully\n")
                }
                None => Reply::not_found("No installed package found matching input criteria."),
            },
            ["pin", "remove"] => {
                let id = arg_value(args, "--id").unwrap_or_default().to_string();
                state.pinned.retain(|p| *p != id);
                Reply::ok("Pin removed successfully\n")
            }
            ["source", "list"] => Reply::ok(format_table(
                &["Name", "Argument", "Explicit"],
                &[
                    vec![
                        "msstore".into(),
                        "https://storeedgefd.dsx.mp.microsoft.com/v9.0".into(),
                        "false".into(),
                    ],
                    vec![
                        "winget".into(),
                        "https://cdn.winget.microsoft.com/cache".into(),
                        "false".into(),
                    ],
                ],
                120,
            )),
            ["source", "update"] => Reply::ok(
                "Updating all sources...\nUpdating source: msstore...\nDone\nUpdating source: winget...\nDone\n",
            ),
            ["export"] => {
                let path = arg_value(args, "--output").unwrap_or_default();
                let ids: Vec<String> = state
                    .installed
                    .iter()
                    .filter(|p| p.has_source())
                    .map(|p| format!("{{\"PackageIdentifier\": \"{}\"}}", p.id))
                    .collect();
                let json = format!(
                    "{{\"Sources\": [{{\"Packages\": [{}]}}]}}\n",
                    ids.join(", ")
                );
                match std::fs::write(path, json) {
                    Ok(()) => Reply::ok("Successfully exported package list.\n"),
                    Err(e) => Reply {
                        text: format!("{e}\n"),
                        code: 1,
                    },
                }
            }
            ["import"] => {
                Reply::ok("Demo: Import wird nur simuliert.\nAll packages are already installed.\n")
            }
            _ => Reply {
                text: format!("Demo: unbekannter Befehl: {}\n", args.join(" ")),
                code: 0x8A15_0002_u32 as i32,
            },
        };
        Some(reply)
    }
}

impl Runner for DemoRunner {
    fn run(
        &self,
        args: &[String],
        on_event: &mut dyn FnMut(OutputEvent),
    ) -> io::Result<CommandOutput> {
        self.cancelled.store(false, Ordering::Relaxed);
        let mut lines = Vec::new();
        let mut splitter = LineSplitter::default();
        let mut forward = |event: OutputEvent| {
            if let OutputEvent::Line(line) = &event {
                lines.push(line.clone());
            }
            on_event(event);
        };
        let reply = self.respond(args, &mut |text| {
            splitter.feed(text.as_bytes(), &mut forward)
        });
        let exit_code = reply.map(|reply| {
            splitter.feed(reply.text.as_bytes(), &mut forward);
            reply.code
        });
        splitter.finish(&mut forward);
        Ok(CommandOutput { lines, exit_code })
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{InstallMode, Scope};
    use crate::winget::{Options, Request, Response};

    fn run(runner: &DemoRunner, request: &Request) -> Result<Response, crate::winget::Failure> {
        let options = Options {
            mode: InstallMode::Silent,
            scope: Scope::Any,
        };
        let output = runner.run(&request.args(&options), &mut |_| {}).unwrap();
        request.interpret(&output)
    }

    #[test]
    fn demo_round_trip_through_the_real_parser() {
        let runner = DemoRunner::default();
        let Ok(Response::Updates(list)) = run(
            &runner,
            &Request::CheckUpdates {
                include_unknown: false,
            },
        ) else {
            panic!("keine Updates");
        };
        assert!(list.upgrades.iter().any(|p| p.id == "Git.Git"));
        assert!(list.pinned.iter().any(|p| p.id == "Discord.Discord"));

        let git = list
            .upgrades
            .iter()
            .find(|p| p.id == "Git.Git")
            .unwrap()
            .clone();
        assert!(matches!(
            run(&runner, &Request::Upgrade { package: git }),
            Ok(Response::Done(None))
        ));

        let Ok(Response::Installed(installed)) = run(&runner, &Request::ListInstalled) else {
            panic!("keine Liste");
        };
        let git = installed.iter().find(|p| p.id == "Git.Git").unwrap();
        assert_eq!(git.version, "2.47.1");
        assert!(!git.has_update());
        // Lange Namen werden wie bei winget gekürzt, IDs bleiben vollständig.
        assert!(installed.iter().any(|p| p.name.ends_with('…')));
        assert!(installed.iter().any(|p| p.id.starts_with("ARP\\")));
    }

    #[test]
    fn demo_search_with_match_column() {
        let runner = DemoRunner::default();
        let request = Request::Search {
            query: "browser".into(),
            source: None,
        };
        let Ok(Response::SearchResults(results)) = run(&runner, &request) else {
            panic!("keine Suchergebnisse");
        };
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|p| p.source == "winget"));
    }
}
