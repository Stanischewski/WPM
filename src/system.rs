//! Windows-spezifisches: Autostart, Administratorrechte, Einzelinstanz und
//! Benachrichtigungen. Auf anderen Systemen (nur Entwicklung/Demo) Platzhalter.
//!
//! Adminrechte gibt es nur über eine sichtbare Windows-Bestätigung (UAC) –
//! bewusst keinen Weg, WPM ohne Nachfrage erhöht zu starten.

use std::io;

pub const AUTOSTART_SUPPORTED: bool = cfg!(windows);
pub const ELEVATION_SUPPORTED: bool = cfg!(windows);

pub use imp::SingleInstance;

#[cfg(windows)]
mod imp {
    use std::ffi::{OsStr, OsString};
    use std::fs;
    use std::io;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::{Path, PathBuf};
    use std::ptr;
    use std::thread;
    use std::time::{Duration, Instant};

    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND,
        LocalFree, SetLastError, WAIT_OBJECT_0,
    };
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{
        AdjustTokenPrivileges, DuplicateTokenEx, GetTokenInformation, LookupPrivilegeValueW,
        PSECURITY_DESCRIPTOR, SE_INCREASE_QUOTA_NAME, SE_PRIVILEGE_ENABLED, SECURITY_ATTRIBUTES,
        SecurityImpersonation, TOKEN_ADJUST_DEFAULT, TOKEN_ADJUST_PRIVILEGES,
        TOKEN_ADJUST_SESSIONID, TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_ELEVATION,
        TOKEN_PRIVILEGES, TOKEN_QUERY, TOKEN_USER, TokenElevation, TokenPrimary, TokenUser,
    };
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::System::LibraryLoader::{
        LOAD_LIBRARY_SEARCH_SYSTEM32, SetDefaultDllDirectories,
    };
    use windows_sys::Win32::System::Threading::{
        CreateEventW, CreateMutexW, CreateProcessWithTokenW, EVENT_MODIFY_STATE, GetCurrentProcess,
        INFINITE, OpenEventW, OpenProcess, OpenProcessToken, PROCESS_INFORMATION,
        PROCESS_QUERY_LIMITED_INFORMATION, STARTUPINFOW, SetEvent, WaitForSingleObject,
    };
    use windows_sys::Win32::UI::Shell::{
        FOLDERID_LocalAppData, FOLDERID_ProgramFiles, FOLDERID_ProgramFilesX86, KF_FLAG_DEFAULT,
        SHGetKnownFolderPath, ShellExecuteW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        ASFW_ANY, AllowSetForegroundWindow, GetShellWindow, GetWindowThreadProcessId,
        MB_ICONINFORMATION, MB_OK, MB_SETFOREGROUND, MessageBoxW, SW_SHOWNORMAL,
    };
    use windows_sys::core::GUID;
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const RUN_VALUE: &str = "WPM";
    const MUTEX_NAME: &str = r"Local\WPM-SingleInstance";
    /// Ereignis, mit dem ein zweiter Start das Fenster der laufenden Instanz öffnet.
    const SHOW_EVENT_NAME: &str = r"Local\WPM-Show";
    /// Zugriff auf das Ereignis: System und Administratoren voll, angemeldete
    /// Benutzer dürfen es nur auslösen – auch wenn WPM mit Adminrechten läuft.
    const SHOW_EVENT_SDDL: &str = "D:(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x00100002;;;IU)";

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
    pub fn current_user_sid() -> Option<String> {
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

    /// Startet WPM ohne Adminrechte: mit dem Zugriffstoken der Windows-Shell
    /// (Explorer), unter dem auch ein normaler Programmstart liefe.
    pub fn launch_unelevated(args: &[String]) -> io::Result<()> {
        let exe = std::env::current_exe()?;
        let directory = exe.parent().map(Path::as_os_str).unwrap_or_default();
        let token = shell_token()?;
        let application = wide(&exe);
        let mut command_line = wide(format!(
            "{} {}",
            super::quote_arg(&exe.to_string_lossy()),
            super::join_args(args)
        ));
        let directory = wide(directory);
        let startup = STARTUPINFOW {
            cb: size_of::<STARTUPINFOW>() as u32,
            ..Default::default()
        };
        let mut info = PROCESS_INFORMATION::default();
        // SAFETY: Token aus `shell_token`; alle Zeichenketten sind nullterminiert
        // und leben bis nach dem Aufruf; `command_line` ist veränderbar, wie verlangt.
        let (created, error) = unsafe {
            let created = CreateProcessWithTokenW(
                token,
                0,
                application.as_ptr(),
                command_line.as_mut_ptr(),
                0,
                ptr::null(),
                directory.as_ptr(),
                &startup,
                &mut info,
            );
            let error = io::Error::last_os_error();
            CloseHandle(token);
            (created, error)
        };
        if created == 0 {
            return Err(error);
        }
        // SAFETY: Beide Handles stammen aus CreateProcessWithTokenW.
        unsafe {
            CloseHandle(info.hThread);
            CloseHandle(info.hProcess);
        }
        Ok(())
    }

    /// Primäres Token des Explorer-Prozesses (der Shell des angemeldeten Benutzers).
    fn shell_token() -> io::Result<HANDLE> {
        // Laut Microsoft-Beispiel nötig, damit CreateProcessWithTokenW das
        // Token verwenden darf; schlägt das fehl, versuchen wir es trotzdem.
        enable_privilege(SE_INCREASE_QUOTA_NAME);
        // SAFETY: Reine Abfragen bzw. Handles, die hier wieder geschlossen werden.
        unsafe {
            let shell = GetShellWindow();
            if shell.is_null() {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "Die Windows-Shell (Explorer) läuft nicht",
                ));
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(shell, &mut pid);
            if pid == 0 {
                return Err(io::Error::last_os_error());
            }
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                return Err(io::Error::last_os_error());
            }
            let mut shell_token: HANDLE = ptr::null_mut();
            let opened = OpenProcessToken(process, TOKEN_DUPLICATE, &mut shell_token);
            let error = io::Error::last_os_error();
            CloseHandle(process);
            if opened == 0 {
                return Err(error);
            }
            let mut token: HANDLE = ptr::null_mut();
            let duplicated = DuplicateTokenEx(
                shell_token,
                TOKEN_QUERY
                    | TOKEN_ASSIGN_PRIMARY
                    | TOKEN_DUPLICATE
                    | TOKEN_ADJUST_DEFAULT
                    | TOKEN_ADJUST_SESSIONID,
                ptr::null(),
                SecurityImpersonation,
                TokenPrimary,
                &mut token,
            );
            let error = io::Error::last_os_error();
            CloseHandle(shell_token);
            if duplicated == 0 {
                return Err(error);
            }
            Ok(token)
        }
    }

    fn enable_privilege(name: windows_sys::core::PCWSTR) {
        // SAFETY: Alle Zeiger zeigen auf lokale Variablen; das Token wird geschlossen.
        unsafe {
            let mut token: HANDLE = ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES, &mut token) == 0 {
                return;
            }
            let mut privileges = TOKEN_PRIVILEGES {
                PrivilegeCount: 1,
                ..Default::default()
            };
            if LookupPrivilegeValueW(ptr::null(), name, &mut privileges.Privileges[0].Luid) != 0 {
                privileges.Privileges[0].Attributes = SE_PRIVILEGE_ENABLED;
                AdjustTokenPrivileges(token, 0, &privileges, 0, ptr::null_mut(), ptr::null_mut());
            }
            CloseHandle(token);
        }
    }

    /// Wartet in einem eigenen Thread darauf, dass ein zweiter Start von WPM
    /// das Fenster öffnen möchte, und ruft dann `on_signal` auf.
    pub fn listen_for_activation(on_signal: Box<dyn Fn() + Send>) {
        let name = wide(SHOW_EVENT_NAME);
        let sddl = wide(SHOW_EVENT_SDDL);
        let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
        // SAFETY: Zeichenketten sind nullterminiert; der Deskriptor wird nach
        // CreateEventW mit LocalFree freigegeben.
        let event = unsafe {
            let have_descriptor = ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                ptr::null_mut(),
            ) != 0;
            let attributes = SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            };
            let event = CreateEventW(
                if have_descriptor {
                    &attributes
                } else {
                    ptr::null()
                },
                0,
                0,
                name.as_ptr(),
            );
            if have_descriptor {
                LocalFree(descriptor);
            }
            event
        };
        if event.is_null() {
            return;
        }
        // HANDLE ist ein Zeiger und damit nicht `Send`; als Zahl übergeben.
        let event = event as usize;
        let _ = thread::Builder::new()
            .name("activation".into())
            .spawn(move || {
                // SAFETY: Das Handle bleibt bis zum Prozessende offen.
                while unsafe { WaitForSingleObject(event as HANDLE, INFINITE) } == WAIT_OBJECT_0 {
                    on_signal();
                }
            });
    }

    /// Bittet die laufende Instanz, ihr Fenster zu zeigen. `false`, wenn
    /// keine erreichbar ist.
    pub fn signal_running_instance() -> bool {
        let name = wide(SHOW_EVENT_NAME);
        // SAFETY: Der Name ist nullterminiert; das Handle wird geschlossen.
        unsafe {
            let event = OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr());
            if event.is_null() {
                return false;
            }
            // Die laufende Instanz darf ihr Fenster in den Vordergrund holen.
            AllowSetForegroundWindow(ASFW_ANY);
            let signalled = SetEvent(event) != 0;
            CloseHandle(event);
            signalled
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

    pub fn open_file(path: &Path) -> io::Result<()> {
        let (verb, file) = (wide("open"), wide(path));
        // SAFETY: Beide Zeichenketten sind nullterminiert.
        let result = unsafe {
            ShellExecuteW(
                ptr::null_mut(),
                verb.as_ptr(),
                file.as_ptr(),
                ptr::null(),
                ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        if result as usize > 32 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
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

    /// Pfad eines Systemordners (Known Folder), unabhängig von Umgebungsvariablen.
    fn known_folder(id: &GUID) -> Option<PathBuf> {
        let mut raw: *mut u16 = ptr::null_mut();
        // SAFETY: `raw` erhält einen von der Shell allozierten String, der mit
        // CoTaskMemFree freigegeben werden muss – laut Doku auch bei Fehlern.
        unsafe {
            let result =
                SHGetKnownFolderPath(id, KF_FLAG_DEFAULT as u32, ptr::null_mut(), &mut raw);
            let path = (result >= 0 && !raw.is_null()).then(|| {
                let len = (0..).take_while(|&i| *raw.add(i) != 0).count();
                PathBuf::from(OsString::from_wide(std::slice::from_raw_parts(raw, len)))
            });
            CoTaskMemFree(raw.cast());
            path
        }
    }

    pub fn local_app_data() -> Option<PathBuf> {
        known_folder(&FOLDERID_LocalAppData)
    }

    pub fn harden_dll_search() {
        // SAFETY: Reiner Aufruf mit einer Konstante.
        unsafe { SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32) };
    }

    /// Liegt wpm.exe unter „Programme“, wo normale Programme weder die EXE
    /// noch DLLs daneben austauschen können?
    pub fn exe_in_protected_folder() -> bool {
        let Ok(exe) = std::env::current_exe().and_then(fs::canonicalize) else {
            return false;
        };
        let exe = PathBuf::from(exe.as_os_str().to_string_lossy().to_lowercase());
        [FOLDERID_ProgramFiles, FOLDERID_ProgramFilesX86]
            .iter()
            .filter_map(known_folder)
            .filter_map(|dir| fs::canonicalize(dir).ok())
            .map(|dir| PathBuf::from(dir.as_os_str().to_string_lossy().to_lowercase()))
            .any(|dir| exe.starts_with(dir))
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

    pub fn launch_unelevated(_args: &[String]) -> io::Result<()> {
        Err(unsupported())
    }

    pub fn listen_for_activation(_on_signal: Box<dyn Fn() + Send>) {}

    pub fn signal_running_instance() -> bool {
        false
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

    pub fn open_file(path: &std::path::Path) -> io::Result<()> {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map(|_| ())
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

    pub fn exe_in_protected_folder() -> bool {
        false
    }

    pub fn current_user_sid() -> Option<String> {
        None
    }

    pub fn harden_dll_search() {}
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

/// Startet WPM ohne Adminrechte (mit dem Token der Windows-Shell), z. B. um
/// nach Updates mit Adminrechten normal im Hintergrund weiterzulaufen.
pub fn launch_unelevated(args: &[String]) -> io::Result<()> {
    imp::launch_unelevated(args)
}

/// Ruft `on_signal` auf, wenn WPM ein zweites Mal gestartet wird.
pub fn listen_for_activation(on_signal: impl Fn() + Send + 'static) {
    imp::listen_for_activation(Box::new(on_signal));
}

/// Bittet eine bereits laufende Instanz, ihr Fenster zu zeigen.
pub fn signal_running_instance() -> bool {
    imp::signal_running_instance()
}

/// Nachgeladene DLLs nur noch aus System32 laden, nie aus dem Programmordner
/// (z. B. dem Download-Ordner, in dem eine untergeschobene DLL liegen könnte).
pub fn harden_dll_search() {
    imp::harden_dll_search();
}

/// `%LOCALAPPDATA%` über die Known-Folder-API, nicht über Umgebungsvariablen.
#[cfg(windows)]
pub fn local_app_data() -> Option<std::path::PathBuf> {
    imp::local_app_data()
}

/// SID des Benutzerkontos, unter dem WPM läuft (nur Windows).
pub fn user_sid() -> Option<String> {
    imp::current_user_sid()
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

/// Öffnet eine Protokolldatei (`.log`/`.txt`) mit dem zugehörigen Programm.
/// Andere Dateitypen werden bewusst nicht geöffnet – der Pfad stammt aus der
/// Ausgabe eines Installers.
pub fn open_log_file(path: &std::path::Path) -> io::Result<()> {
    let is_log = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("log") || e.eq_ignore_ascii_case("txt"));
    if !is_log || !path.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Protokolldatei nicht gefunden",
        ));
    }
    imp::open_file(path)
}

pub fn notify(title: &str, body: &str) {
    imp::notify(title, body);
}

/// Ist der Autostart (`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`) eingetragen?
pub fn autostart_enabled() -> bool {
    imp::run_entry_exists()
}

/// Trägt WPM in den Run-Schlüssel ein bzw. entfernt den Eintrag. WPM startet
/// dort immer ohne Adminrechte und damit ohne Windows-Bestätigung; ein
/// vorhandener Eintrag wird mit dem aktuellen Programmpfad erneuert.
pub fn set_autostart(enabled: bool) -> io::Result<()> {
    if enabled {
        imp::set_run_entry()
    } else {
        imp::remove_run_entry()
    }
}

/// Liegt wpm.exe unter „Programme“? Mit Adminrechten aus einem beschreibbaren
/// Ordner (z. B. Downloads) könnten dort abgelegte fremde DLLs mitgeladen werden.
pub fn exe_in_protected_folder() -> bool {
    static PROTECTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *PROTECTED.get_or_init(imp::exe_in_protected_folder)
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
}
