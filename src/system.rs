//! Windows-spezifisches: Autostart, Administratorrechte, Einzelinstanz und
//! Benachrichtigungen. Auf anderen Systemen (nur Entwicklung/Demo) Platzhalter.

use std::io;

pub const AUTOSTART_SUPPORTED: bool = cfg!(windows);
pub const ELEVATION_SUPPORTED: bool = cfg!(windows);

pub use imp::SingleInstance;

#[cfg(windows)]
mod imp {
    use std::ffi::{OsStr, OsString};
    use std::fs;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::ptr;
    use std::thread;
    use std::time::{Duration, Instant};

    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND,
        LocalFree, SetLastError,
    };
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    use windows_sys::Win32::Security::{
        GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TOKEN_USER, TokenElevation, TokenUser,
    };
    use windows_sys::Win32::System::Threading::{
        CreateMutexW, GetCurrentProcess, OpenProcessToken,
    };
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MB_ICONINFORMATION, MB_OK, MB_SETFOREGROUND, MessageBoxW, SW_SHOWNORMAL,
    };
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const RUN_VALUE: &str = "WPM";
    const TASK_NAME: &str = "WPM Autostart";
    const MUTEX_NAME: &str = r"Local\WPM-SingleInstance";

    fn wide(text: impl AsRef<OsStr>) -> Vec<u16> {
        text.as_ref().encode_wide().chain(Some(0)).collect()
    }

    /// Ruft `f` mit dem Zugriffstoken des eigenen Prozesses auf.
    fn with_process_token<T>(f: impl FnOnce(HANDLE) -> Option<T>) -> Option<T> {
        let mut token: HANDLE = ptr::null_mut();
        // SAFETY: `token` ist eine gültige lokale Variable; das Pseudo-Handle
        // von GetCurrentProcess muss nicht geschlossen werden.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return None;
        }
        let result = f(token);
        // SAFETY: Handle aus OpenProcessToken, wird genau einmal geschlossen.
        unsafe { CloseHandle(token) };
        result
    }

    pub fn is_elevated() -> bool {
        with_process_token(|token| {
            let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
            let mut size = 0u32;
            // SAFETY: Puffer und Größe passen zu TOKEN_ELEVATION.
            let ok = unsafe {
                GetTokenInformation(
                    token,
                    TokenElevation,
                    (&raw mut elevation).cast(),
                    size_of::<TOKEN_ELEVATION>() as u32,
                    &mut size,
                )
            };
            Some(ok != 0 && elevation.TokenIsElevated != 0)
        })
        .unwrap_or(false)
    }

    /// SID des angemeldeten Benutzers (z. B. `S-1-5-21-…`), eindeutiger als
    /// `DOMÄNE\Name` – auch bei Microsoft- und Entra-ID-Konten.
    fn current_user_sid() -> Option<String> {
        with_process_token(|token| {
            let mut size = 0u32;
            // SAFETY: Erster Aufruf ohne Puffer ermittelt nur die nötige Größe.
            unsafe { GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut size) };
            // u64-Puffer, damit TOKEN_USER (enthält Zeiger) korrekt ausgerichtet ist.
            let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
            // SAFETY: Der Puffer ist mindestens `size` Bytes groß.
            let ok = unsafe {
                GetTokenInformation(
                    token,
                    TokenUser,
                    buffer.as_mut_ptr().cast(),
                    size,
                    &mut size,
                )
            };
            if ok == 0 {
                return None;
            }
            // SAFETY: GetTokenInformation hat eine TOKEN_USER-Struktur geschrieben.
            let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
            let mut text: *mut u16 = ptr::null_mut();
            // SAFETY: Die SID zeigt in `buffer`, das noch lebt; `text` wird mit
            // LocalFree freigegeben.
            unsafe {
                if ConvertSidToStringSidW(user.User.Sid, &mut text) == 0 || text.is_null() {
                    return None;
                }
                let len = (0..).take_while(|&i| *text.add(i) != 0).count();
                let sid = String::from_utf16_lossy(std::slice::from_raw_parts(text, len));
                LocalFree(text.cast());
                Some(sid)
            }
        })
    }

    pub fn relaunch_elevated(args: &[String], window: Option<isize>) -> io::Result<()> {
        let exe = std::env::current_exe()?;
        let directory = exe.parent().map(Path::as_os_str).unwrap_or_default();
        let parameters = super::join_args(args);
        let (verb, file, parameters, directory) =
            (wide("runas"), wide(&exe), wide(parameters), wide(directory));
        // SAFETY: Alle Zeichenketten sind nullterminiert und leben bis nach dem Aufruf.
        let result = unsafe {
            ShellExecuteW(
                // Mit Fensterbezug erscheint die Windows-Bestätigung im Vordergrund.
                window.map_or(ptr::null_mut(), |hwnd| hwnd as HWND),
                verb.as_ptr(),
                file.as_ptr(),
                parameters.as_ptr(),
                directory.as_ptr(),
                SW_SHOWNORMAL,
            )
        };
        // Werte > 32 bedeuten Erfolg; abgelehnte Bestätigung: ERROR_CANCELLED.
        if result as usize > 32 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    /// Hält einen benannten Mutex, solange WPM läuft.
    pub struct SingleInstance(HANDLE);

    impl SingleInstance {
        /// `None`, wenn bereits eine andere Instanz läuft (auch nach `wait`).
        pub fn acquire(wait: Duration) -> Option<Self> {
            let name = wide(MUTEX_NAME);
            let deadline = Instant::now() + wait;
            loop {
                // SAFETY: Der Name ist nullterminiert; Sicherheitsattribute dürfen null sein.
                // SetLastError(0) verhindert, dass ein alter Fehlercode fehlgedeutet wird.
                let (handle, error) = unsafe {
                    SetLastError(0);
                    let handle = CreateMutexW(ptr::null(), 0, name.as_ptr());
                    (handle, GetLastError())
                };
                if !handle.is_null() && error != ERROR_ALREADY_EXISTS {
                    return Some(Self(handle));
                }
                if !handle.is_null() {
                    // SAFETY: Gültiges Handle aus CreateMutexW.
                    unsafe { CloseHandle(handle) };
                } else if error != ERROR_ACCESS_DENIED {
                    // Unerwarteter Fehler: lieber ohne Schutz starten als gar nicht.
                    return Some(Self(ptr::null_mut()));
                }
                // Bereits vorhanden (bei ERROR_ACCESS_DENIED: andere Instanz mit Adminrechten).
                if Instant::now() >= deadline {
                    return None;
                }
                thread::sleep(Duration::from_millis(250));
            }
        }
    }

    impl Drop for SingleInstance {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: Handle stammt aus CreateMutexW und wird nur hier geschlossen.
                unsafe { CloseHandle(self.0) };
            }
        }
    }

    pub fn message_box(title: &str, text: &str) {
        let (title, text) = (wide(title), wide(text));
        // SAFETY: Beide Zeichenketten sind nullterminiert.
        unsafe {
            MessageBoxW(
                ptr::null_mut(),
                text.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONINFORMATION | MB_SETFOREGROUND,
            )
        };
    }

    pub fn notify(title: &str, body: &str) {
        let _ = notify_rust::Notification::new()
            .summary(title)
            .body(body)
            .show();
    }

    // --- Autostart über den Run-Schlüssel (ohne Adminrechte) ---

    pub fn run_entry_exists() -> bool {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(RUN_KEY)
            .and_then(|key| key.get_value::<String, _>(RUN_VALUE))
            .is_ok()
    }

    pub fn set_run_entry() -> io::Result<()> {
        let exe = std::env::current_exe()?;
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN_KEY)?;
        key.set_value(RUN_VALUE, &format!("\"{}\" --autostart", exe.display()))
    }

    pub fn remove_run_entry() -> io::Result<()> {
        let key = match RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(RUN_KEY, winreg::enums::KEY_SET_VALUE)
        {
            Ok(key) => key,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        match key.delete_value(RUN_VALUE) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }

    // --- Autostart über die Aufgabenplanung (mit höchsten Rechten) ---

    /// Vollständiger Pfad, damit nicht versehentlich eine gleichnamige Datei
    /// aus dem Programmordner gestartet wird.
    fn schtasks_path() -> PathBuf {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| OsString::from(r"C:\Windows"));
        Path::new(&root).join("System32").join("schtasks.exe")
    }

    fn schtasks(args: &[&OsStr]) -> io::Result<()> {
        let output = Command::new(schtasks_path())
            .args(args)
            .stdin(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .output()?;
        if output.status.success() {
            return Ok(());
        }
        let text = if output.stderr.is_empty() {
            &output.stdout
        } else {
            &output.stderr
        };
        let message = String::from_utf8_lossy(text).trim().to_string();
        Err(io::Error::other(if message.is_empty() {
            format!("schtasks beendet mit {}", output.status)
        } else {
            message
        }))
    }

    pub fn task_exists() -> bool {
        schtasks(&["/Query".as_ref(), "/TN".as_ref(), TASK_NAME.as_ref()]).is_ok()
    }

    pub fn create_task() -> io::Result<()> {
        let exe = std::env::current_exe()?;
        let directory = exe.parent().map(Path::to_path_buf).unwrap_or_default();
        let user = match (
            current_user_sid(),
            std::env::var("USERDOMAIN"),
            std::env::var("USERNAME"),
        ) {
            (Some(sid), _, _) => sid,
            (None, Ok(domain), Ok(name)) => format!(r"{domain}\{name}"),
            (None, Err(_), Ok(name)) => name,
            _ => return Err(io::Error::other("Benutzerkonto nicht ermittelbar")),
        };
        let xml = super::task_xml(
            &user,
            &exe.display().to_string(),
            "--autostart",
            &directory.display().to_string(),
        );

        // schtasks erwartet die XML-Datei als UTF-16 mit BOM.
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend(xml.encode_utf16().flat_map(u16::to_le_bytes));
        let path = std::env::temp_dir().join(format!("wpm-autostart-{}.xml", std::process::id()));
        fs::write(&path, bytes)?;
        let result = schtasks(&[
            "/Create".as_ref(),
            "/TN".as_ref(),
            TASK_NAME.as_ref(),
            "/XML".as_ref(),
            path.as_os_str(),
            "/F".as_ref(),
        ]);
        let _ = fs::remove_file(&path);
        result
    }

    pub fn delete_task() -> io::Result<()> {
        if !task_exists() {
            return Ok(());
        }
        schtasks(&[
            "/Delete".as_ref(),
            "/TN".as_ref(),
            TASK_NAME.as_ref(),
            "/F".as_ref(),
        ])
    }
}

#[cfg(not(windows))]
mod imp {
    use std::io;
    use std::time::Duration;

    fn unsupported() -> io::Error {
        io::Error::new(io::ErrorKind::Unsupported, "nur unter Windows verfügbar")
    }

    pub fn is_elevated() -> bool {
        false
    }

    pub fn relaunch_elevated(_args: &[String], _window: Option<isize>) -> io::Result<()> {
        Err(unsupported())
    }

    pub struct SingleInstance;

    impl SingleInstance {
        pub fn acquire(_wait: Duration) -> Option<Self> {
            Some(Self)
        }
    }

    pub fn message_box(title: &str, text: &str) {
        eprintln!("[{title}] {text}");
    }

    pub fn notify(title: &str, body: &str) {
        eprintln!("[Benachrichtigung] {title}: {body}");
    }

    pub fn run_entry_exists() -> bool {
        false
    }

    pub fn set_run_entry() -> io::Result<()> {
        Err(unsupported())
    }

    pub fn remove_run_entry() -> io::Result<()> {
        Ok(())
    }

    pub fn task_exists() -> bool {
        false
    }

    pub fn create_task() -> io::Result<()> {
        Err(unsupported())
    }

    pub fn delete_task() -> io::Result<()> {
        Ok(())
    }
}

/// Läuft WPM mit Administratorrechten? (Ändert sich zur Laufzeit nicht.)
pub fn is_elevated() -> bool {
    static ELEVATED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ELEVATED.get_or_init(imp::is_elevated)
}

/// Startet WPM mit Administratorrechten neu (Windows fragt per UAC nach).
/// Gibt einen Fehler zurück, wenn die Bestätigung abgelehnt wurde. `window`
/// ist das Fensterhandle (HWND), damit die Nachfrage im Vordergrund erscheint.
pub fn relaunch_elevated(args: &[String], window: Option<isize>) -> io::Result<()> {
    imp::relaunch_elevated(args, window)
}

/// Fensterhandle (HWND) des Hauptfensters, sofern unter Windows verfügbar.
pub fn native_window(handle: &impl raw_window_handle::HasWindowHandle) -> Option<isize> {
    match handle.window_handle().ok()?.as_raw() {
        raw_window_handle::RawWindowHandle::Win32(window) => Some(window.hwnd.get()),
        _ => None,
    }
}

pub fn message_box(title: &str, text: &str) {
    imp::message_box(title, text);
}

pub fn notify(title: &str, body: &str) {
    imp::notify(title, body);
}

/// Ist ein Autostart eingetragen (Run-Schlüssel oder Aufgabenplanung)?
pub fn autostart_enabled() -> bool {
    imp::run_entry_exists() || imp::task_exists()
}

/// Richtet den Autostart ein oder entfernt ihn. Vorhandene Einträge werden
/// mit dem aktuellen Programmpfad überschrieben.
///
/// Mit `elevated_task` startet WPM über die Aufgabenplanung mit höchsten
/// Rechten – ohne Windows-Bestätigung beim Anmelden. Das Einrichten selbst
/// braucht Adminrechte. Sonst dient `HKCU\…\Run` (ohne Adminrechte).
pub fn configure_autostart(enabled: bool, elevated_task: bool) -> io::Result<()> {
    if enabled && elevated_task {
        imp::create_task()?;
        imp::remove_run_entry()
    } else if enabled {
        imp::set_run_entry()?;
        imp::delete_task().map_err(|e| {
            io::Error::new(
                e.kind(),
                format!(
                    "Die Autostart-Aufgabe mit Adminrechten ließ sich nicht entfernen ({e}). \
                     WPM als Administrator starten und erneut versuchen."
                ),
            )
        })
    } else {
        imp::remove_run_entry()?;
        imp::delete_task()
    }
}

/// Setzt Argumente nach den Windows-Regeln (CommandLineToArgvW) zusammen.
#[cfg_attr(not(windows), allow(dead_code))]
fn join_args(args: &[String]) -> String {
    args.iter()
        .map(|arg| quote_arg(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '"']) {
        return arg.to_string();
    }
    let mut quoted = String::from('"');
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                // Backslashes vor einem Anführungszeichen verdoppeln und es escapen.
                quoted.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                quoted.extend(std::iter::repeat_n('\\', backslashes));
                quoted.push(c);
                backslashes = 0;
            }
        }
    }
    // Backslashes vor dem schließenden Anführungszeichen verdoppeln.
    quoted.extend(std::iter::repeat_n('\\', backslashes * 2));
    quoted.push('"');
    quoted
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Aufgabe für die Aufgabenplanung: beim Anmelden des Benutzers mit höchsten
/// Rechten starten, auch im Akkubetrieb, ohne Zeitlimit.
#[cfg_attr(not(windows), allow(dead_code))]
fn task_xml(user: &str, command: &str, arguments: &str, directory: &str) -> String {
    let (user, command, arguments, directory) = (
        xml_escape(user),
        xml_escape(command),
        xml_escape(arguments),
        xml_escape(directory),
    );
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Startet WPM beim Anmelden mit Administratorrechten, ohne nachzufragen.</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
      <Delay>PT15S</Delay>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>false</StartWhenAvailable>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>5</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{command}</Command>
      <Arguments>{arguments}</Arguments>
      <WorkingDirectory>{directory}</WorkingDirectory>
    </Exec>
  </Actions>
</Task>
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_are_quoted_like_command_line_to_argv() {
        assert_eq!(
            join_args(&["--autostart".into(), "--elevated".into()]),
            "--autostart --elevated"
        );
        assert_eq!(quote_arg(""), r#""""#);
        assert_eq!(quote_arg(r"C:\Program Files\"), r#""C:\Program Files\\""#);
        assert_eq!(quote_arg(r#"a "b" c"#), r#""a \"b\" c""#);
        assert_eq!(quote_arg(r"a\\b c"), r#""a\\b c""#);
    }

    #[test]
    fn task_xml_escapes_values() {
        let xml = task_xml(
            r"PC\Anna & Ben",
            r"C:\Apps & Tools\wpm.exe",
            "--autostart",
            r"C:\Apps & Tools",
        );
        assert!(xml.contains(r"<UserId>PC\Anna &amp; Ben</UserId>"));
        assert!(xml.contains(r"<Command>C:\Apps &amp; Tools\wpm.exe</Command>"));
        assert!(xml.contains("<RunLevel>HighestAvailable</RunLevel>"));
        assert!(!xml.contains("{user}"));
    }
}
