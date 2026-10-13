# WPM – grafische Oberfläche für winget

Eine einfache, schnelle Oberfläche für den Windows-Paketmanager
[winget](https://learn.microsoft.com/windows/package-manager/winget/),
geschrieben in Rust mit [egui](https://github.com/emilk/egui). Die Idee:
so unkompliziert wie die Paketverwaltung unter Linux – Updates sehen,
mit einem Klick installieren, fertig.

![WPM im Demo-Modus](docs/screenshot.png)

## Funktionen

| Reiter | Was geht |
|---|---|
| **Updates** | Verfügbare Updates anzeigen, einzeln, ausgewählt oder alle installieren. Pakete „anheften“ (Update ignorieren) bzw. wieder lösen. |
| **Installiert** | Alle installierten Programme, filterbar nach Name/ID, mit Details, Aktualisieren und Deinstallieren (mit Rückfrage). |
| **Suchen** | Neue Pakete in winget und/oder Microsoft Store finden und installieren. Bereits installierte Pakete sind markiert. |
| **Einstellungen** | Mit Windows starten (optional minimiert), Administrator-Modus (siehe unten), Updates beim Start und regelmäßig im Hintergrund prüfen, Windows-Benachrichtigung bei neuen Updates, Installationsmodus (still/Standard/interaktiv), Installationsbereich (Benutzer/System), Darstellung (hell/dunkel/wie Windows). Außerdem: winget-Quellen anzeigen und aktualisieren, angeheftete Pakete verwalten, Paketliste exportieren/importieren. |
| **Protokoll** | Jeder ausgeführte winget-Befehl mit seiner vollständigen Ausgabe. |

Aufträge landen in einer Warteschlange und laufen nacheinander (winget mag
keine parallelen Installationen). Die Statusleiste zeigt den Fortschritt live,
laufende Vorgänge lassen sich abbrechen.

## Starten

Voraussetzung: Windows 10/11 mit installiertem **App-Installer** (enthält
`winget`, ist auf aktuellen Systemen vorinstalliert).

```powershell
cargo run --release
```

Fertige EXE-Dateien baut die GitHub-Action `Build` bei jedem Push – unter
*Actions → Build → Artifacts → wpm-windows-x64*.

### Kommandozeilenoptionen

| Option | Wirkung |
|---|---|
| `--demo` | Demo-Modus mit Beispieldaten – kein winget nötig, läuft auch unter Linux/macOS. Gut zum Ausprobieren der Oberfläche. |
| `--minimized` | Minimiert starten. |
| `--autostart` | Wird vom Autostart-Eintrag gesetzt; startet minimiert, wenn das in den Einstellungen so gewählt ist. |
| `--elevated` | Intern: kennzeichnet den mit Adminrechten neu gestarteten Prozess. |

WPM läuft nur einmal. Ein zweiter Start meldet, dass WPM bereits läuft (beim
Autostart ohne Meldung).

## Weniger Nachfragen beim Aktualisieren

Zwei Arten von Fenstern können während Updates aufpoppen:

1. **Installer-Fenster** – im Installationsmodus **„Still“** (Standard,
   `--silent`) zeigen die meisten Installer nichts an. Einzelne Installer
   öffnen trotzdem ein Fenster oder starten das Programm nach dem Update neu.
2. **Windows-Bestätigungen (UAC)** – jeder Installer, der systemweit
   installiert, fragt einzeln nach Adminrechten. Wird eine Nachfrage abgelehnt
   oder übersehen, bricht das Update ab (winget-Code `0x8A15010C`, Installer-
   Exitcode 1602). Manche MSIX-Pakete brauchen zwingend Adminrechte
   (`0x80073D28`).

Gegen Letzteres hilft der **Administrator-Modus** (Einstellungen → Allgemein →
„Mit Administratorrechten ausführen“): WPM startet sich mit Adminrechten neu,
Windows fragt also **einmal beim Start** statt bei jedem Installer. Alternativ
gibt es in der Statusleiste und bei passenden Fehlermeldungen den Knopf
„Als Administrator neu starten“.

Ist zusätzlich „Mit Windows starten“ aktiv, richtet WPM statt des Run-Eintrags
eine Aufgabe **„WPM Autostart“** in der Aufgabenplanung ein (beim Anmelden, mit
höchsten Rechten). Dann erscheint auch beim Anmelden keine Nachfrage. Diese
Aufgabe kann nur mit Adminrechten angelegt werden – WPM macht das automatisch,
sobald es einmal als Administrator läuft.

Der Administrator-Modus ist nur sinnvoll, wenn das eigene Benutzerkonto ein
Administratorkonto ist. Gibt man bei der Nachfrage die Zugangsdaten eines
*anderen* Kontos ein, läuft WPM unter diesem Konto – mit dessen Einstellungen
und benutzerbezogenen Programmen.

### Cross-Compile unter Linux

```sh
rustup target add x86_64-pc-windows-gnu
sudo apt install gcc-mingw-w64-x86-64
CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc \
  cargo build --release --target x86_64-pc-windows-gnu
```

## Was man mit winget alles abfragen kann

WPM nutzt diese Befehle (alle mit `--accept-source-agreements` und
`--disable-interactivity`, damit winget ohne Konsole nie auf eine Eingabe wartet):

| winget-Befehl | Verwendung in WPM |
|---|---|
| `winget upgrade` | verfügbare Updates (inkl. angehefteter Pakete) |
| `winget list` | installierte Programme |
| `winget search --query …` | Pakete suchen |
| `winget show --id …` | Paketdetails (Beschreibung, Herausgeber, Lizenz, Homepage …) |
| `winget install` / `upgrade` / `uninstall --id … --exact` | Pakete verwalten |
| `winget pin add/remove/list` | Updates für einzelne Pakete ignorieren |
| `winget source list/update` | Paketquellen anzeigen und aktualisieren |
| `winget export` / `import` | Paketliste sichern bzw. auf einem neuen PC wiederherstellen |
| `winget --version` | Version anzeigen / prüfen, ob winget vorhanden ist |

Weitere Möglichkeiten, die sich später ergänzen ließen:

- `winget download` – Installer nur herunterladen
- `winget repair` – beschädigte Installation reparieren
- `winget configure` – ganze Rechner-Konfigurationen (DSC) anwenden
- `winget settings` – winget-eigene Einstellungen (z. B. Download-Verhalten)
- `winget source add` – eigene/firmeninterne Paketquellen

## Aufbau

```
src/
├── main.rs            Startparameter, Fenster
├── app/               Oberfläche (ein Modul je Reiter)
├── winget/
│   ├── mod.rs         Befehle bauen, Ergebnisse auswerten, Fehlercodes
│   ├── parse.rs       Parser für die Tabellen-Ausgabe von winget
│   └── demo.rs        simuliertes winget für --demo und Tests
├── runner.rs          winget als Prozess starten, Ausgabe live lesen
├── worker.rs          Warteschlange im Hintergrund-Thread
├── scheduler.rs       regelmäßige Update-Prüfung
├── settings.rs        Einstellungen (%APPDATA%\WPM\settings.json)
└── system.rs          Autostart, Adminrechte, Einzelinstanz, Benachrichtigungen
```

Autostart schreibt normalerweise den Eintrag `WPM` unter
`HKCU\Software\Microsoft\Windows\CurrentVersion\Run` (keine Adminrechte nötig),
im Administrator-Modus stattdessen die Aufgabe „WPM Autostart“.

## Gut zu wissen

- **winget hat keine maschinenlesbare Ausgabe.** WPM liest die Textausgabe:
  Spalten werden über die Position der Kopfzeile erkannt, nicht über die
  (übersetzten) Spaltennamen. Das funktioniert unabhängig von der
  Windows-Sprache, ist aber naturgemäß empfindlicher als eine echte API.
- **Gekürzte IDs:** Ohne Konsole kürzt winget breite Tabellen auf 120 Zeichen
  und hängt „…“ an. Für solche IDs sucht WPM ohne `--exact` nach dem Anfang
  der ID; ist das nicht eindeutig, meldet winget das.
- **Adminrechte:** WPM selbst braucht keine. Gegen einzelne UAC-Nachfragen
  der Installer hilft der Administrator-Modus (siehe oben).
- **Sicherheit:** `winget.exe` und `schtasks.exe` werden über ihren vollen
  Pfad gestartet, damit nie eine gleichnamige Datei aus dem Programmordner
  (z. B. dem Download-Ordner) mit Adminrechten läuft.
- **Hintergrundprüfung** läuft, solange WPM geöffnet ist (auch minimiert).
  Ein Symbol im Infobereich (Tray) gibt es noch nicht.

## Mögliche nächste Schritte

- Tray-Symbol, damit WPM beim Schließen im Hintergrund weiterläuft
- statt Text-Parsing die WinGet-COM-API (`Microsoft.Management.Deployment`)
  oder das PowerShell-Modul `Microsoft.WinGet.Client` nutzen
- weitere Paketmanager wie Scoop oder Chocolatey
- Programmsymbol in der EXE (Ressourcendatei)
