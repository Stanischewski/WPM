//! WPM über winget aktualisieren. winget kann die laufende `wpm.exe` nicht
//! ersetzen, deshalb: WPM legt eine Kopie von sich als Hilfsprogramm neben
//! die EXE, startet sie und beendet sich. Das Hilfsprogramm wartet auf das
//! Ende von WPM, führt `winget upgrade` aus und startet WPM wieder.
//!
//! Die Kopie liegt im Programmordner, weil sie dort dieselben Schreibrechte
//! hat wie `wpm.exe` selbst – bei einer systemweiten Installation kann sie also
//! kein normales Programm austauschen, bevor sie mit Adminrechten läuft.

use std::io;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use crate::runner::{Runner, WingetRunner};
use crate::settings::{InstallMode, Scope};
use crate::system;
use crate::winget::{Options, Package, Request};

/// Paket-ID von WPM in der winget-Quelle.
pub const PACKAGE_ID: &str = "Stanischewski.WPM";

/// Startet das Hilfsprogramm: `--self-update-helper=<PID von WPM>`.
const HELPER_FLAG: &str = "--self-update-helper=";
/// Die wieder zu startende `wpm.exe`.
const TARGET_FLAG: &str = "--self-update-target=";
/// WPM nach dem Update wieder im Infobereich statt mit Fenster starten.
const HIDDEN_FLAG: &str = "--self-update-hidden";
/// Dem neu gestarteten WPM: Update erfolgreich …
pub const DONE_FLAG: &str = "--self-update-done";
/// … bzw. fehlgeschlagen, mit Kurzfassung des Fehlers.
const FAILED_FLAG: &str = "--self-update-failed=";

const HELPER_NAME: &str = "wpm-update.exe";

/// Ist das Paket WPM selbst? (Auch gekürzte IDs aus breiten Tabellen.)
pub fn is_self(package: &Package) -> bool {
    is_self_id(&package.id)
}

fn is_self_id(id: &str) -> bool {
    let own = PACKAGE_ID.to_lowercase();
    let id = id.to_lowercase();
    // Aus einer lokalen Manifest-Datei installiert oder keiner Quelle
    // zugeordnet: winget zeigt dann den Eintrag aus „Apps“, z. B.
    // `ARP\User\X64\Stanischewski.WPM__DefaultSource`.
    if let Some(entry) = id.strip_prefix(r"arp\") {
        let name = entry.rsplit('\\').next().unwrap_or_default();
        return name.trim_end_matches('…').starts_with(&format!("{own}_"));
    }
    match id.strip_suffix('…') {
        Some(prefix) => prefix.len() >= 3 && own.starts_with(prefix),
        None => id == own,
    }
}

/// Wie das Hilfsprogramm gestartet wird.
pub struct Launch {
    /// WPM war versteckt (Infobereich) und soll es danach wieder sein.
    pub hidden: bool,
    /// WPM läuft mit Adminrechten, das Update braucht aber keine: dann das
    /// Hilfsprogramm (und damit auch das neue WPM) ohne Adminrechte starten.
    pub drop_elevation: bool,
}

/// Legt das Hilfsprogramm an und startet es. WPM muss sich danach beenden.
pub fn start(launch: &Launch) -> io::Result<()> {
    let exe = system::exe_path()?;
    let helper = exe.with_file_name(HELPER_NAME);
    std::fs::copy(&exe, &helper)?;
    let mut args = vec![
        format!("{HELPER_FLAG}{}", std::process::id()),
        format!("{TARGET_FLAG}{}", exe.display()),
    ];
    if launch.hidden {
        args.push(HIDDEN_FLAG.into());
    }
    if launch.drop_elevation {
        system::launch_unelevated_exe(&helper, &args)
    } else {
        system::spawn_detached(&helper, &args)
    }
}

/// Ergebnis eines Selbst-Updates, das dem neu gestarteten WPM mitgegeben wird.
pub fn outcome(args: &[String]) -> Option<Result<(), String>> {
    if let Some(summary) = args.iter().find_map(|a| a.strip_prefix(FAILED_FLAG)) {
        return Some(Err(summary.to_string()));
    }
    args.iter().any(|a| a == DONE_FLAG).then_some(Ok(()))
}

/// Läuft WPM gerade als Hilfsprogramm? Dann das Update ausführen und `true`
/// zurückgeben – `main` beendet sich danach, ohne ein Fenster zu öffnen.
pub fn run_helper_if_requested(args: &[String]) -> bool {
    let Some(pid) = args.iter().find_map(|a| a.strip_prefix(HELPER_FLAG)) else {
        return false;
    };
    let Some(target) = args.iter().find_map(|a| a.strip_prefix(TARGET_FLAG)) else {
        return true;
    };
    let hidden = args.iter().any(|a| a == HIDDEN_FLAG);
    run_helper(pid.parse().unwrap_or(0), Path::new(target), hidden);
    true
}

fn run_helper(pid: u32, target: &Path, hidden: bool) {
    // Erst wenn WPM beendet ist, lässt sich wpm.exe ersetzen.
    system::wait_for_process(pid, Duration::from_secs(60));

    let request = Request::Upgrade {
        package: Package {
            name: "WPM".into(),
            id: PACKAGE_ID.into(),
            source: "winget".into(),
            ..Package::default()
        },
    };
    let options = Options {
        mode: InstallMode::Silent,
        scope: Scope::Any,
    };
    let result = WingetRunner::default()
        .run(&request.args(&options), &mut |_| {})
        .map_err(|e| {
            trf!(
                "winget konnte nicht gestartet werden: {e}",
                "Could not start winget: {e}"
            )
        })
        .and_then(|output| {
            request
                .interpret(&output)
                .map(drop)
                .map_err(|failure| failure.summary)
        });

    let mut args = match result {
        Ok(()) => vec![DONE_FLAG.to_string()],
        Err(summary) => vec![format!("{FAILED_FLAG}{summary}")],
    };
    if hidden {
        args.extend(crate::background_args(false));
    }
    // Mit Adminrechten (systemweite Installation) das neue WPM normal starten.
    let started = (system::is_elevated() && system::launch_unelevated_exe(target, &args).is_ok())
        || system::spawn_detached(target, &args).is_ok();
    if !started {
        system::message_box(
            "WPM",
            &trf!(
                "WPM konnte nach dem Update nicht neu gestartet werden ({}). Bitte WPM \
                 von Hand starten (z. B. mit Win+R → wpm).",
                "WPM could not be restarted after the update ({}). Please start WPM \
                 manually (e.g. press Win+R and enter wpm).",
                target.display()
            ),
        );
    }
    if system::is_elevated() {
        // Das neue WPM läuft ohne Adminrechte und kann diese Kopie unter
        // „Programme“ nicht löschen – Windows entfernt sie beim nächsten Start.
        if let Ok(me) = std::env::current_exe() {
            let _ = system::delete_on_reboot(&me);
        }
    }
}

/// Entfernt ein übrig gebliebenes Hilfsprogramm. Es kann sich noch kurz
/// beenden, während das neue WPM startet – daher mehrere Versuche.
pub fn remove_leftover_helper() {
    let Ok(exe) = system::exe_path() else { return };
    let helper: PathBuf = exe.with_file_name(HELPER_NAME);
    if helper == exe || !helper.exists() {
        return;
    }
    // Unter „Programme“ geht das nur mit Adminrechten; sonst entfernt Windows
    // die Kopie beim nächsten Start (siehe run_helper).
    if system::exe_in_protected_folder() && !system::is_elevated() {
        return;
    }
    let _ = thread::Builder::new()
        .name("remove-update-helper".into())
        .spawn(move || {
            for _ in 0..30 {
                match std::fs::remove_file(&helper) {
                    Err(e) if e.kind() != io::ErrorKind::NotFound => {
                        thread::sleep(Duration::from_secs(1));
                    }
                    _ => return,
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_package_is_recognized() {
        let package = |id: &str| Package {
            id: id.into(),
            ..Package::default()
        };
        assert!(is_self(&package("Stanischewski.WPM")));
        assert!(is_self(&package("stanischewski.wpm")));
        // winget kürzt breite Tabellen und hängt „…“ an.
        assert!(is_self(&package("Stanischewski.W…")));
        assert!(!is_self(&package("Stanischewski.WPMX")));
        assert!(!is_self(&package("Other.WPM")));
        assert!(!is_self(&package("S…")));
        // Einträge aus „Apps“ (lokal installiert bzw. keiner Quelle zugeordnet).
        assert!(is_self(&package(
            r"ARP\User\X64\Stanischewski.WPM__DefaultSource"
        )));
        assert!(is_self(&package(
            r"ARP\Machine\X64\Stanischewski.WPM_Microsoft.Winget.Source_8wekyb3d8bbwe"
        )));
        assert!(is_self(&package(r"ARP\User\X64\Stanischewski.WPM__Def…")));
        assert!(!is_self(&package(
            r"ARP\User\X64\Stanischewski.WPMX_DefaultSource"
        )));
        assert!(!is_self(&package(r"ARP\Machine\X64\Druckerhilfe XY_is1")));
    }

    #[test]
    fn outcome_is_passed_to_the_restarted_instance() {
        assert_eq!(outcome(&[]), None);
        assert_eq!(outcome(&[DONE_FLAG.into()]), Some(Ok(())));
        assert_eq!(
            outcome(&[format!("{FAILED_FLAG}Zugriff verweigert")]),
            Some(Err("Zugriff verweigert".into()))
        );
    }
}
