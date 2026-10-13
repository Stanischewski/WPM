//! Startet winget als Kindprozess und liest dessen Ausgabe live mit.

use std::io::{self, Read};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::thread;

/// Ein Stück Ausgabe, so wie es ein Terminal darstellen würde.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutputEvent {
    /// Eine abgeschlossene Zeile (endet mit `\n`).
    Line(String),
    /// Ein Zwischenstand, der im Terminal per `\r` überschrieben würde
    /// (Fortschrittsbalken, Spinner).
    Progress(String),
}

#[derive(Clone, Debug, Default)]
pub struct CommandOutput {
    pub lines: Vec<String>,
    /// `None`, wenn der Prozess abgebrochen wurde.
    pub exit_code: Option<i32>,
}

impl CommandOutput {
    pub fn success(&self) -> bool {
        self.exit_code == Some(0)
    }

    /// winget liefert HRESULT-artige Fehlercodes (z. B. `0x8A150014`).
    pub fn code(&self) -> Option<u32> {
        self.exit_code.map(|c| c as u32)
    }
}

pub trait Runner: Send + Sync {
    /// Führt `winget <args>` aus und meldet jede Ausgabe über `on_event`.
    fn run(
        &self,
        args: &[String],
        on_event: &mut dyn FnMut(OutputEvent),
    ) -> io::Result<CommandOutput>;

    /// Bricht den gerade laufenden Befehl ab.
    fn cancel(&self);
}

/// Führt die echte `winget.exe` aus.
#[derive(Default)]
pub struct WingetRunner {
    child: Mutex<Option<Child>>,
}

impl Runner for WingetRunner {
    fn run(
        &self,
        args: &[String],
        on_event: &mut dyn FnMut(OutputEvent),
    ) -> io::Result<CommandOutput> {
        let mut cmd = Command::new(winget_program()?);
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // Kein Konsolenfenster für winget aufpoppen lassen.
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd.spawn()?;
        let mut stdout = child.stdout.take().expect("stdout ist gepiped");
        let mut stderr = child.stderr.take().expect("stderr ist gepiped");
        *self.child.lock().unwrap() = Some(child);

        let stderr_reader = thread::spawn(move || {
            let mut data = Vec::new();
            let _ = stderr.read_to_end(&mut data);
            data
        });

        let mut lines = Vec::new();
        let mut emit = |event: OutputEvent| {
            if let OutputEvent::Line(line) = &event {
                lines.push(line.clone());
            }
            on_event(event);
        };

        let mut splitter = LineSplitter::default();
        let mut buf = [0u8; 4096];
        let read_result = loop {
            match stdout.read(&mut buf) {
                Ok(0) => break Ok(()),
                Ok(n) => splitter.feed(&buf[..n], &mut emit),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => break Err(e),
            }
        };
        splitter.finish(&mut emit);

        let stderr_data = stderr_reader.join().unwrap_or_default();
        let mut err_splitter = LineSplitter::default();
        err_splitter.feed(&stderr_data, &mut emit);
        err_splitter.finish(&mut emit);

        let child = self.child.lock().unwrap().take();
        let status = match child {
            Some(mut child) => child.wait()?,
            None => return Err(io::Error::other("Prozess verloren")),
        };
        read_result?;

        Ok(CommandOutput {
            lines,
            exit_code: status.code(),
        })
    }

    fn cancel(&self) {
        if let Some(child) = self.child.lock().unwrap().as_mut() {
            let _ = child.kill();
        }
    }
}

/// Vollständiger Pfad zu winget. Ein bloßes `winget` würde zuerst im
/// Programmordner von WPM gesucht – mit Adminrechten wäre eine dort abgelegte
/// fremde `winget.exe` gefährlich. Bevorzugt wird der App-Alias des
/// App-Installers, sonst `PATH` ohne Programmordner, aktuelles Verzeichnis
/// und relative Einträge.
#[cfg(windows)]
fn winget_program() -> io::Result<std::path::PathBuf> {
    use std::path::{Path, PathBuf};

    // Der Alias ist ein Reparse-Point: nicht folgen, nur Existenz prüfen.
    let exists = |path: &Path| std::fs::symlink_metadata(path).is_ok();
    if let Some(local) = crate::system::local_app_data() {
        let alias = local.join(r"Microsoft\WindowsApps\winget.exe");
        if exists(&alias) {
            return Ok(alias);
        }
    }
    let normalize = |path: &Path| path.to_string_lossy().trim_end_matches('\\').to_lowercase();
    let excluded: Vec<String> = [
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(PathBuf::from)),
        std::env::current_dir().ok(),
    ]
    .into_iter()
    .flatten()
    .map(|dir| normalize(&dir))
    .collect();
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .filter(|dir| dir.is_absolute() && !excluded.contains(&normalize(dir)))
        .map(|dir| dir.join("winget.exe"))
        .find(|candidate| exists(candidate))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "winget.exe nicht gefunden"))
}

#[cfg(not(windows))]
fn winget_program() -> io::Result<std::path::PathBuf> {
    Ok("winget".into())
}

/// Zerlegt einen Byte-Strom in Zeilen und `\r`-Zwischenstände.
///
/// `\r\n` gilt als normales Zeilenende, ein alleinstehendes `\r` als
/// „Zeile überschreiben“, wie es winget für Fortschrittsanzeigen nutzt.
#[derive(Default)]
pub struct LineSplitter {
    buf: Vec<u8>,
    pending_cr: bool,
}

impl LineSplitter {
    pub fn feed(&mut self, data: &[u8], emit: &mut dyn FnMut(OutputEvent)) {
        for &byte in data {
            if self.pending_cr {
                self.pending_cr = false;
                if byte == b'\n' {
                    self.flush_line(emit);
                    continue;
                }
                self.flush_progress(emit);
            }
            match byte {
                b'\r' => self.pending_cr = true,
                b'\n' => self.flush_line(emit),
                _ => self.buf.push(byte),
            }
        }
    }

    pub fn finish(&mut self, emit: &mut dyn FnMut(OutputEvent)) {
        if self.pending_cr || !self.buf.is_empty() {
            self.pending_cr = false;
            self.flush_line(emit);
        }
    }

    fn take_text(&mut self) -> String {
        let text = strip_ansi(&String::from_utf8_lossy(&self.buf));
        self.buf.clear();
        text
    }

    fn flush_line(&mut self, emit: &mut dyn FnMut(OutputEvent)) {
        let text = self.take_text();
        emit(OutputEvent::Line(text.trim_end().to_string()));
    }

    fn flush_progress(&mut self, emit: &mut dyn FnMut(OutputEvent)) {
        let text = self.take_text();
        let text = text.trim();
        // Reine Spinner-Zeichen sind keine sinnvolle Information.
        if !text.is_empty() && !matches!(text, "-" | "\\" | "|" | "/") {
            emit(OutputEvent::Progress(text.to_string()));
        }
    }
}

/// Entfernt ANSI/VT-Steuersequenzen (Farben, Cursor, Taskleisten-Fortschritt).
pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            // CSI: ESC [ … Endzeichen
            Some('[') => {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            // OSC: ESC ] … BEL oder ESC \
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\x07' {
                        break;
                    }
                    if c == '\x1b' {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(chunks: &[&[u8]]) -> Vec<OutputEvent> {
        let mut events = Vec::new();
        let mut splitter = LineSplitter::default();
        for chunk in chunks {
            splitter.feed(chunk, &mut |e| events.push(e));
        }
        splitter.finish(&mut |e| events.push(e));
        events
    }

    #[test]
    fn crlf_is_a_normal_line_end_even_across_chunks() {
        let events = split(&[b"eins\r", b"\nzwei\r\n"]);
        assert_eq!(
            events,
            vec![
                OutputEvent::Line("eins".into()),
                OutputEvent::Line("zwei".into())
            ]
        );
    }

    #[test]
    fn carriage_return_marks_progress_and_spinners_are_dropped() {
        let events = split(&[b"\r   - \r   \\ \r  ##   1 MB / 2 MB\rName  Id\n"]);
        assert_eq!(
            events,
            vec![
                OutputEvent::Progress("##   1 MB / 2 MB".into()),
                OutputEvent::Line("Name  Id".into())
            ]
        );
    }

    #[test]
    fn multibyte_characters_split_across_chunks_survive() {
        let text = "Verfügbar …\n".as_bytes();
        let (a, b) = text.split_at(6);
        assert_eq!(
            split(&[a, b]),
            vec![OutputEvent::Line("Verfügbar …".into())]
        );
    }

    #[test]
    fn ansi_sequences_are_removed() {
        assert_eq!(
            strip_ansi("\x1b[0m\x1b]9;4;1;40\x1b\\Fertig\x1b[?25h"),
            "Fertig"
        );
        assert_eq!(strip_ansi("\x1b]0;Titel\x07Text"), "Text");
    }
}
