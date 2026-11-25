//! Ursache eines fehlgeschlagenen Installers aus seinem Protokoll. winget
//! nennt bei MSI-Installern (Windows Installer) die Protokolldatei. Darin
//! endet der fehlgeschlagene Schritt mit „Return value 3“, und kurz davor
//! steht meist die Fehlermeldung – vom Windows Installer („Error 1714. …“)
//! oder vom Hersteller des Programms („Product: X -- …“).

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

/// Größere Protokolle werden nur vom Ende her gelesen.
const MAX_BYTES: u64 = 16 * 1024 * 1024;
/// So viele Zeilen vor dem fehlgeschlagenen Schritt wird nach der Meldung gesucht.
const LOOKBACK: usize = 400;
/// Fehlernummern, die nur „Installation fehlgeschlagen“ bedeuten.
const GENERIC_ERRORS: [u32; 3] = [1603, 1708, 1709];
/// Längere Meldungen werden gekürzt.
const MAX_MESSAGE_CHARS: usize = 400;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogCause {
    /// Fehlermeldung aus dem Protokoll, in der Sprache von Windows bzw. des
    /// Herstellers.
    pub message: Option<String>,
    /// Nummer des Windows-Installer-Fehlers, z. B. 1714.
    pub error: Option<u32>,
    /// Fehlgeschlagener Schritt, z. B. `RemoveExistingProducts`.
    pub action: Option<String>,
}

impl LogCause {
    /// Eine Zeile für die Oberfläche, z. B. „Laut Installationsprotokoll: …“.
    pub fn describe(&self) -> Option<String> {
        if self.message.is_none() {
            let step = step_name(self.action.as_deref()?);
            return Some(trf!(
                "Laut Installationsprotokoll ist dieser Schritt fehlgeschlagen: {step}",
                "According to the installer log, this step failed: {step}"
            ));
        }
        let short = self.short()?;
        Some(trf!(
            "Laut Installationsprotokoll: {short}",
            "According to the installer log: {short}"
        ))
    }

    /// Ohne Einleitung, für die Zeilen einer zusammengefassten Meldung.
    pub fn short(&self) -> Option<String> {
        let step = self.action.as_deref().map(step_name);
        match (&self.message, step) {
            (Some(message), Some(step)) => Some(trf!(
                "{message} (Schritt: {step})",
                "{message} (step: {step})"
            )),
            (Some(message), None) => Some(message.clone()),
            (None, Some(step)) => Some(trf!(
                "Schritt fehlgeschlagen: {step}",
                "Failed step: {step}"
            )),
            (None, None) => None,
        }
    }
}

/// Liest und untersucht ein Installationsprotokoll; `None`, wenn es fehlt,
/// kein MSI-Protokoll ist oder nichts Brauchbares enthält.
pub fn read(path: &Path) -> Option<LogCause> {
    analyze(&decode(&read_bytes(path).ok()?))
}

fn read_bytes(path: &Path) -> io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let mut bytes = Vec::new();
    if len <= MAX_BYTES {
        file.read_to_end(&mut bytes)?;
        return Ok(bytes);
    }
    // Anfang (Byte-Order-Mark) und Ende; gerade Position wegen UTF-16.
    let mut head = [0u8; 3];
    file.read_exact(&mut head)?;
    bytes.extend_from_slice(&head[..bom_len(&head)]);
    file.seek(SeekFrom::Start((len - MAX_BYTES) & !1))?;
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn bom_len(bytes: &[u8]) -> usize {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        3
    } else if bytes.starts_with(&[0xFF, 0xFE]) {
        2
    } else {
        0
    }
}

/// Windows Installer schreibt UTF-16 oder in der ANSI-Codepage (bei
/// deutschem Windows Windows-1252); manche Programme auch UTF-8.
fn decode(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16le(rest);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    // UTF-16 ohne Byte-Order-Mark: Text mit Nullbytes an jeder zweiten Stelle.
    let sample = &bytes[..bytes.len().min(512)];
    let zeros = sample
        .iter()
        .skip(1)
        .step_by(2)
        .filter(|&&b| b == 0)
        .count();
    if sample.len() >= 4 && zeros * 4 >= sample.len() {
        return utf16le(bytes);
    }
    // Nur vom Ende gelesen, kann der Text mitten in einem Zeichen beginnen.
    let start = bytes
        .iter()
        .take(3)
        .take_while(|&&b| (0x80..0xC0).contains(&b))
        .count();
    match std::str::from_utf8(&bytes[start..]) {
        Ok(text) => text.to_owned(),
        Err(_) => ansi(bytes),
    }
}

/// ANSI-Codepage des Systems (unter deutschem Windows Windows-1252, unter
/// polnischem 1250 usw.).
#[cfg(windows)]
fn ansi(bytes: &[u8]) -> String {
    use windows_sys::Win32::Globalization::{CP_ACP, MultiByteToWideChar};
    let Ok(len) = i32::try_from(bytes.len()) else {
        return windows_1252(bytes);
    };
    if len == 0 {
        return String::new();
    }
    // SAFETY: Erst die nötige Länge erfragen, dann in einen Puffer genau dieser
    // Länge schreiben; beide Zeiger gelten für die angegebene Länge.
    let needed =
        unsafe { MultiByteToWideChar(CP_ACP, 0, bytes.as_ptr(), len, std::ptr::null_mut(), 0) };
    if needed <= 0 {
        return windows_1252(bytes);
    }
    let mut wide = vec![0u16; needed as usize];
    // SAFETY: siehe oben.
    let written =
        unsafe { MultiByteToWideChar(CP_ACP, 0, bytes.as_ptr(), len, wide.as_mut_ptr(), needed) };
    if written <= 0 {
        return windows_1252(bytes);
    }
    String::from_utf16_lossy(&wide[..written as usize])
}

#[cfg(not(windows))]
fn ansi(bytes: &[u8]) -> String {
    windows_1252(bytes)
}

fn utf16le(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    String::from_utf16_lossy(&units)
}

fn windows_1252(bytes: &[u8]) -> String {
    /// 0x80–0x9F; der Rest entspricht Latin-1.
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8D}', 'Ž',
        '\u{8F}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9D}',
        'ž', 'Ÿ',
    ];
    bytes
        .iter()
        .map(|&b| match b {
            0x80..=0x9F => HIGH[usize::from(b - 0x80)],
            _ => char::from(b),
        })
        .collect()
}

/// Sucht den ersten fehlgeschlagenen Schritt und die Meldung davor.
pub fn analyze(text: &str) -> Option<LogCause> {
    if !text.contains("MSI (s) (") && !text.contains("MSI (c) (") && !text.contains("Action start")
    {
        return None;
    }
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let failed = lines.iter().position(|line| is_failed_action(line));
    let failed_action = failed.and_then(|index| action_name(lines[index]));
    // Nach dem eigentlichen Fehler folgen oft Meldungen vom Zurückrollen
    // (Rollback). Deshalb vom Start des fehlgeschlagenen Schritts an
    // vorwärts suchen: Die erste Meldung ist die Ursache.
    let scope = match (failed, &failed_action) {
        (Some(index), Some(action)) => action_start(&lines[..index], action)
            .map(|start| &lines[start..index])
            .unwrap_or(&lines[index.saturating_sub(LOOKBACK)..index]),
        (Some(index), None) => &lines[index.saturating_sub(LOOKBACK)..index],
        (None, _) => &lines[..],
    };
    let scoped = scope.iter().find_map(|line| message(line));
    // Steht im Schritt nichts, davor suchen (nie danach: Dort kommen nur
    // noch Rollback- und Abschlussmeldungen).
    let found = scoped.or_else(|| {
        let before = &lines[..failed.unwrap_or(lines.len())];
        before.iter().rev().find_map(|line| message(line))
    });
    // Eine benutzerdefinierte Aktion ist genauer als der Schritt, in dem sie
    // lief (meist InstallFinalize) – die erste, die darin scheiterte.
    let custom_action = scope.iter().find_map(|line| {
        let rest = line.strip_prefix("CustomAction ")?;
        let (name, _) = rest.split_once(" returned actual error code ")?;
        Some(name.trim().to_string())
    });
    let action = custom_action.or(failed_action);
    let (message, error) = match found {
        Some((message, error)) => (Some(message), error),
        None => (None, None),
    };
    if message.is_none() && action.is_none() {
        return None;
    }
    Some(LogCause {
        message,
        error,
        action,
    })
}

/// Zeile, mit der `action` begann: „Action start 21:41:24: LaunchConditions.“
/// bzw. „Aktion gestartet um 21:41:24: LaunchConditions.“ (die letzte vor dem Ende).
fn action_start(lines: &[&str], action: &str) -> Option<usize> {
    let ending = format!(": {action}.");
    lines.iter().rposition(|line| {
        (line.starts_with("Action start") || line.starts_with("Aktion gestartet"))
            && line.ends_with(&ending)
    })
}

/// „Action ended 21:41:24: LaunchConditions. Return value 3.“ – auch auf
/// deutschem Windows meist englisch, sonst „Rückgabewert 3“.
fn is_failed_action(line: &str) -> bool {
    line.contains("Return value 3.") || line.contains("Rückgabewert 3.")
}

fn action_name(line: &str) -> Option<String> {
    let end = line
        .find("Return value 3.")
        .or_else(|| line.find("Rückgabewert 3."))?;
    let (_, name) = line[..end].trim_end_matches([' ', '.']).rsplit_once(": ")?;
    let name = name.trim();
    (!name.is_empty() && !name.contains(' ')).then(|| name.to_string())
}

/// Eine Fehlermeldung samt Windows-Installer-Fehlernummer, falls die Zeile eine ist.
fn message(line: &str) -> Option<(String, Option<u32>)> {
    let text = if line.starts_with("MSI (") {
        // „MSI (s) (D8:64) [21:41:24:480]: Product: WordCraft -- Meldung“
        let (_, rest) = line.split_once("]: ")?;
        let rest = rest
            .strip_prefix("Product: ")
            .or_else(|| rest.strip_prefix("Produkt: "))?;
        let text = rest.split_once(" -- ")?.1.trim();
        // Warnungen und Hinweise (z. B. „Warning 1909.“) sind keine Ursache.
        if ["Warning ", "Warnung ", "Info "]
            .iter()
            .any(|prefix| text.starts_with(prefix))
        {
            return None;
        }
        text
    } else if error_code(line).is_some() {
        line
    } else {
        return None;
    };
    if text.is_empty() || is_status(text) {
        return None;
    }
    let error = error_code(text);
    if error.is_some_and(|e| GENERIC_ERRORS.contains(&e)) {
        return None;
    }
    Some((shorten(text), error))
}

/// „Error 1714. …“ bzw. „Fehler 1714. …“
fn error_code(text: &str) -> Option<u32> {
    let rest = text
        .strip_prefix("Error ")
        .or_else(|| text.strip_prefix("Fehler "))?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let after = rest[digits.len()..].chars().next();
    if digits.len() != 4 || !matches!(after, Some('.' | ':')) {
        return None;
    }
    digits.parse().ok()
}

/// Abschlussmeldungen ohne Ursache, z. B. „Installation failed.“
/// Nur die ganze Meldung zählt: „Installation failed: .NET 8 is required“
/// eines Herstellers ist dagegen eine Ursache.
fn is_status(text: &str) -> bool {
    const ENGLISH: [&str; 11] = [
        "installation completed successfully",
        "installation operation completed successfully",
        "installation failed",
        "installation operation failed",
        "removal completed successfully",
        "removal failed",
        "configuration completed successfully",
        "configuration failed",
        "advertisement completed successfully",
        "advertisement failed",
        "reconfiguration completed successfully",
    ];
    let lower = text.trim_end_matches('.').trim().to_lowercase();
    ENGLISH.contains(&lower.as_str())
        // Deutsche Fassungen, z. B. „Installation erfolgreich abgeschlossen.“
        // oder „Fehler bei der Installation.“
        || (lower.chars().count() <= 60
            && (lower.contains("erfolgreich abgeschlossen")
                || lower.ends_with("fehlgeschlagen")
                || lower.starts_with("fehler bei der")
                || lower.starts_with("fehler beim")))
}

fn shorten(text: &str) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= MAX_MESSAGE_CHARS {
        return text;
    }
    let mut short: String = text.chars().take(MAX_MESSAGE_CHARS).collect();
    short.push('…');
    short
}

/// Bekannte Schritte verständlich benennen, z. B. „Entfernen der alten
/// Version“ statt `RemoveExistingProducts`; andere bleiben, wie sie heißen.
fn step_name(action: &str) -> String {
    let known = match action {
        "LaunchConditions" => tr!("Prüfen der Voraussetzungen", "checking requirements"),
        "CostFinalize" => tr!(
            "Prüfen von Zielordner und Speicherplatz",
            "checking target folder and disk space"
        ),
        "InstallValidate" => tr!(
            "Prüfen von Speicherplatz und geöffneten Dateien",
            "checking disk space and files in use"
        ),
        "RemoveExistingProducts" => tr!("Entfernen der alten Version", "removing the old version"),
        "InstallFiles" => tr!("Kopieren der Dateien", "copying files"),
        "InstallServices" | "StartServices" => {
            tr!(
                "Einrichten eines Windows-Dienstes",
                "setting up a Windows service"
            )
        }
        "InstallFinalize" => tr!("Abschluss der Installation", "finishing the installation"),
        _ => return action.to_string(),
    };
    known.to_string()
}

/// Was bei häufigen Windows-Installer-Fehlern hilft.
pub fn advice(error: u32) -> Option<&'static str> {
    Some(match error {
        1714 | 1612 | 1316 => tr!(
            "Die alte Version lässt sich nicht entfernen, meist weil ihr Installationspaket \
             fehlt. Programm erst deinstallieren, dann neu installieren.",
            "The old version cannot be removed, usually because its installation package is \
             missing. Uninstall the program first, then install it again."
        ),
        1304 | 1310 | 1306 => tr!(
            "Eine Datei ließ sich nicht schreiben – meist läuft das Programm noch. Programm \
             vollständig beenden (auch im Infobereich neben der Uhr) und erneut versuchen.",
            "A file could not be written – usually the program is still running. Quit the \
             program completely (also in the system tray next to the clock) and try again."
        ),
        1308 | 1311 | 1335 => tr!(
            "Das heruntergeladene Installationspaket ist unvollständig oder beschädigt. \
             Erneut versuchen.",
            "The downloaded installation package is incomplete or corrupt. Try again."
        ),
        1721..=1723 => tr!(
            "Ein Hilfsprogramm des Installers ist fehlgeschlagen – meist ein Fehler im \
             Installer selbst. Programm schließen und erneut versuchen; hilft das nicht, \
             deinstallieren und neu installieren.",
            "A helper program of the installer failed – usually a bug in the installer itself. \
             Close the program and try again; if that does not help, uninstall and reinstall it."
        ),
        1920 | 1921 | 1923 => tr!(
            "Ein Windows-Dienst des Programms ließ sich nicht einrichten oder starten. Windows \
             neu starten und erneut versuchen.",
            "A Windows service of the program could not be set up or started. Restart Windows \
             and try again."
        ),
        1618 | 1500 => tr!(
            "Eine andere Installation läuft gerade (z. B. Windows Update). Etwas warten und \
             erneut versuchen.",
            "Another installation is in progress (e.g. Windows Update). Wait a moment and try \
             again."
        ),
        1303 | 1925 | 1730 => tr!(
            "Dafür sind Administratorrechte nötig.",
            "This requires administrator rights."
        ),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{self, Lang};

    /// Gekürzt aus einem echten Protokoll (ArtCraft WordCraft über winget, als
    /// Administrator, deutsches Windows); persönliche Angaben ersetzt.
    const LAUNCH_CONDITION_LOG: &str = "\
=== Logging started: 25.11.2026  21:41:24 ===
Action start 21:41:24: INSTALL.
Action start 21:41:24: FindRelatedProducts.
Action ended 21:41:24: FindRelatedProducts. Return value 1.
Action start 21:41:24: AppSearch.
Action ended 21:41:24: AppSearch. Return value 1.
Action start 21:41:24: CraftMFromLegacyExe.
Action ended 21:41:24: CraftMFromLegacyExe. Return value 1.
Action start 21:41:24: LaunchConditions.
MSI (s) (D8:64) [21:41:24:480]: Product: WordCraft -- This app is already installed for everyone. Choose Everyone to update it, or pass ALLUSERS=1 to msiexec. Uninstall that copy before changing who it is installed for.

This app is already installed for everyone. Choose Everyone to update it, or pass ALLUSERS=1 to msiexec. Uninstall that copy before changing who it is installed for.
Action ended 21:41:24: LaunchConditions. Return value 3.
Action ended 21:41:24: INSTALL. Return value 3.
Property(S): MSIINSTALLPERUSER = 1
Property(S): ErrorDialog = ErrorDlg
Property(S): LogonUser = demo
MSI (s) (D8:64) [21:41:24:495]: Product: WordCraft -- Installation failed.

MSI (s) (D8:64) [21:41:24:495]: Das Produkt wurde durch Windows Installer installiert. Produktname: WordCraft. Produktversion: 0.18.1. Produktsprache: 1033. Hersteller: Learning Machines LLC. Erfolg- bzw. Fehlerstatus der Installation: 1603.

=== Logging stopped: 25.11.2026  21:41:24 ===
";

    const OLD_VERSION_LOG: &str = "\
=== Logging started: 01.12.2026  10:00:00 ===
Action start 10:00:01: RemoveExistingProducts.
MSI (s) (A0:B4) [10:00:01:234]: Note: 1: 1714 2: Demo 3: 1612
MSI (s) (A0:B4) [10:00:01:234]: Produkt: Demo -- Fehler 1714. Die ältere Version von Demo kann nicht entfernt werden. Wenden Sie sich an Ihren technischen Support.  Systemfehler 1612.

Fehler 1714. Die ältere Version von Demo kann nicht entfernt werden. Wenden Sie sich an Ihren technischen Support.  Systemfehler 1612.
Action ended 10:00:02: RemoveExistingProducts. Return value 3.
Action ended 10:00:02: INSTALL. Return value 3.
=== Logging stopped: 01.12.2026  10:00:02 ===
";

    #[test]
    fn vendor_message_before_the_failed_step_is_found() {
        let cause = analyze(LAUNCH_CONDITION_LOG).unwrap();
        assert_eq!(
            cause.message.as_deref(),
            Some(
                "This app is already installed for everyone. Choose Everyone to update it, or \
                 pass ALLUSERS=1 to msiexec. Uninstall that copy before changing who it is \
                 installed for."
            )
        );
        assert_eq!(cause.error, None);
        assert_eq!(cause.action.as_deref(), Some("LaunchConditions"));
        let text = cause.describe().unwrap();
        assert!(text.starts_with("Laut Installationsprotokoll: This app is already installed"));
        assert!(text.ends_with("(Schritt: Prüfen der Voraussetzungen)"));
        i18n::with(Lang::En, || {
            assert!(
                cause
                    .describe()
                    .unwrap()
                    .ends_with("(step: checking requirements)")
            );
        });
    }

    #[test]
    fn windows_installer_error_and_advice() {
        let cause = analyze(OLD_VERSION_LOG).unwrap();
        assert_eq!(cause.error, Some(1714));
        assert_eq!(cause.action.as_deref(), Some("RemoveExistingProducts"));
        assert!(
            cause.message.as_deref().unwrap().starts_with(
                "Fehler 1714. Die ältere Version von Demo kann nicht entfernt werden."
            )
        );
        assert!(advice(1714).unwrap().contains("deinstallieren"));
        assert_eq!(advice(1603), None);
    }

    #[test]
    fn custom_action_is_more_precise_than_install_finalize() {
        let log = "\
Action start 10:00:00: InstallFinalize.
CustomAction CA_RegisterService returned actual error code 1603 (note this may not be 100% accurate if translation happened inside sandbox)
MSI (s) (A0:B4) [10:00:05:000]: Product: Demo -- Error 1722. There is a problem with this Windows Installer package. A program run as part of the setup did not finish as expected.
Action ended 10:00:05: InstallFinalize. Return value 3.
";
        let cause = analyze(log).unwrap();
        assert_eq!(cause.action.as_deref(), Some("CA_RegisterService"));
        assert_eq!(cause.error, Some(1722));
        assert!(advice(1722).is_some());
    }

    #[test]
    fn rollback_messages_do_not_hide_the_real_cause() {
        let log = "\
Action start 10:00:00: InstallFinalize.
MSI (s) (A0:B4) [10:00:01:000]: Product: Demo -- Error 1001. Exception occurred while initializing the installation: System.IO.FileNotFoundException: demo.dll
CustomAction CA_Install returned actual error code 1603 (note this may not be 100% accurate if translation happened inside sandbox)
Rollback: Installing service
MSI (s) (A0:B4) [10:00:02:000]: Product: Demo -- Error 1001. An exception occurred during the Rollback phase of the installation. This exception will be ignored.
CustomAction CA_Rollback returned actual error code 1603 (note this may not be 100% accurate if translation happened inside sandbox)
Action ended 10:00:03: InstallFinalize. Return value 3.
MSI (s) (A0:B4) [10:00:03:000]: Product: Demo -- Configuration failed.
";
        let cause = analyze(log).unwrap();
        assert!(cause.message.unwrap().contains("FileNotFoundException"));
        assert_eq!(cause.action.as_deref(), Some("CA_Install"));
    }

    #[test]
    fn vendor_texts_are_kept_but_status_and_warnings_are_not() {
        let log = "\
Action start 10:00:00: LaunchConditions.
MSI (s) (A0:B4) [10:00:00:100]: Product: Demo -- Warning 1909. Could not create shortcut Demo.lnk.
MSI (s) (A0:B4) [10:00:00:200]: Product: Demo -- Installation failed: .NET Desktop Runtime 8 is required.
Action ended 10:00:00: LaunchConditions. Return value 3.
";
        assert_eq!(
            analyze(log).unwrap().message.as_deref(),
            Some("Installation failed: .NET Desktop Runtime 8 is required.")
        );
        assert!(is_status("Configuration failed."));
        assert!(is_status("Installation erfolgreich abgeschlossen."));
        assert!(!is_status(
            "Das Update war nicht erfolgreich: Bitte zuerst Demo 1.x deinstallieren."
        ));
        assert_eq!(advice(1303), advice(1730));
    }

    #[test]
    fn only_the_failed_step_is_known() {
        let log = "\
Action start 10:00:00: InstallFiles.
Action ended 10:00:05: InstallFiles. Return value 3.
MSI (s) (A0:B4) [10:00:05:000]: Product: Demo -- Installation failed.
";
        let cause = analyze(log).unwrap();
        assert_eq!(cause.message, None);
        assert_eq!(
            cause.describe().unwrap(),
            "Laut Installationsprotokoll ist dieser Schritt fehlgeschlagen: Kopieren der Dateien"
        );
    }

    #[test]
    fn other_logs_and_successful_installs_give_nothing() {
        assert_eq!(analyze("Inno Setup Setup Log\nFatal error\n"), None);
        let success = "\
Action start 10:00:00: INSTALL.
Action ended 10:00:05: INSTALL. Return value 1.
MSI (s) (A0:B4) [10:00:05:000]: Product: Demo -- Installation completed successfully.
";
        assert_eq!(analyze(success), None);
    }

    #[test]
    fn encodings_are_decoded() {
        let text = "Fehler 1714. Die ältere Version";
        // UTF-16LE mit und ohne Byte-Order-Mark.
        let utf16: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let mut with_bom = vec![0xFF, 0xFE];
        with_bom.extend(&utf16);
        assert_eq!(decode(&with_bom), text);
        assert_eq!(decode(&utf16), text);
        // ANSI (Windows-1252) und UTF-8.
        let ansi: Vec<u8> = b"Fehler 1714. Die \xE4ltere Version \x84x\x93".to_vec();
        assert_eq!(decode(&ansi), "Fehler 1714. Die ältere Version „x“");
        assert_eq!(decode(text.as_bytes()), text);
    }

    #[test]
    fn log_files_are_read() {
        let dir = std::env::temp_dir().join(format!("wpm-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("install.log");
        let utf16: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain(
                LAUNCH_CONDITION_LOG
                    .encode_utf16()
                    .flat_map(u16::to_le_bytes),
            )
            .collect();
        std::fs::write(&path, utf16).unwrap();
        let cause = read(&path).unwrap();
        assert_eq!(cause.action.as_deref(), Some("LaunchConditions"));
        assert_eq!(read(&dir.join("fehlt.log")), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
