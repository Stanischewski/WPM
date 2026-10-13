//! Windows-spezifisches: Autostart über die Registry und Benachrichtigungen.
//! Auf anderen Systemen (nur für Entwicklung/Demo) sind das Platzhalter.

use std::io;

#[cfg(windows)]
mod imp {
    use std::io;

    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const VALUE_NAME: &str = "WPM";

    fn command_line() -> io::Result<String> {
        let exe = std::env::current_exe()?;
        Ok(format!("\"{}\" --autostart", exe.display()))
    }

    pub fn autostart_enabled() -> bool {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(RUN_KEY)
            .and_then(|key| key.get_value::<String, _>(VALUE_NAME))
            .is_ok()
    }

    pub fn set_autostart(enabled: bool) -> io::Result<()> {
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN_KEY)?;
        if enabled {
            key.set_value(VALUE_NAME, &command_line()?)
        } else {
            match key.delete_value(VALUE_NAME) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                other => other,
            }
        }
    }

    pub fn notify(title: &str, body: &str) {
        let _ = notify_rust::Notification::new()
            .summary(title)
            .body(body)
            .show();
    }
}

#[cfg(not(windows))]
mod imp {
    use std::io;

    pub fn autostart_enabled() -> bool {
        false
    }

    pub fn set_autostart(_enabled: bool) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Autostart gibt es nur unter Windows",
        ))
    }

    pub fn notify(title: &str, body: &str) {
        eprintln!("[Benachrichtigung] {title}: {body}");
    }
}

pub const AUTOSTART_SUPPORTED: bool = cfg!(windows);

pub fn autostart_enabled() -> bool {
    imp::autostart_enabled()
}

/// Trägt WPM unter `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` ein
/// bzw. entfernt den Eintrag. Ein vorhandener Eintrag wird mit dem aktuellen
/// Programmpfad überschrieben, falls die EXE verschoben wurde.
pub fn set_autostart(enabled: bool) -> io::Result<()> {
    imp::set_autostart(enabled)
}

pub fn notify(title: &str, body: &str) {
    imp::notify(title, body);
}
