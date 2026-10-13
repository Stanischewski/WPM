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
| **Einstellungen** | Mit Windows starten (optional minimiert), Updates mit Administratorrechten (siehe unten), Updates beim Start und regelmäßig im Hintergrund prüfen, Windows-Benachrichtigung bei neuen Updates, Installationsmodus (still/Standard/interaktiv), Installationsbereich (Benutzer/System), Darstellung (hell/dunkel/wie Windows). Außerdem: winget-Quellen anzeigen und aktualisieren, angeheftete Pakete verwalten, Paketliste exportieren/importieren. |
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
| `--upgrade=<ID>` | Intern: Update, das nach dem Neustart mit Adminrechten fortgesetzt wird (`*` = alle). |

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

Gegen Letzteres hilft **„Updates mit Administratorrechten installieren“**
(Einstellungen → Allgemein): Klickt man auf „Aktualisieren“, „Auswahl
aktualisieren“ oder „Alle aktualisieren“, startet WPM sich **einmal** mit
Adminrechten neu (eine Windows-Bestätigung) und setzt genau diese Updates
automatisch fort. Danach fragt kein Installer mehr einzeln, bis WPM geschlossen
wird. Autostart und Update-Suche im Hintergrund laufen weiterhin ganz ohne
Nachfrage. Lehnt man die Bestätigung ab, laufen die Updates wie bisher ohne
Adminrechte. Zusätzlich gibt es in der Statusleiste und bei passenden
Fehlermeldungen „Als Administrator neu starten“ bzw. „Mit Adminrechten erneut
versuchen“.

Bewusst **nicht** umgesetzt: ein Autostart, der WPM ohne Nachfrage mit
Adminrechten startet (z. B. über die Aufgabenplanung). WPM startet winget über
den App-Alias im Benutzerprofil; jedes Programm des Benutzers könnte diesen
austauschen und hätte dann unbemerkt Adminrechte. Adminrechte gibt es deshalb
nur nach einer sichtbaren Windows-Bestätigung – wie bei `winget` in einem
Administrator-Terminal.

Grenzen:

- Nur sinnvoll, wenn das eigene Benutzerkonto ein Administratorkonto ist. Gibt
  man bei der Nachfrage die Zugangsdaten eines *anderen* Kontos ein (oder ist
  der Windows-Administratorschutz aktiv), läuft WPM unter diesem Konto – mit
  dessen Einstellungen und benutzerbezogenen Programmen. WPM erkennt das und
  warnt.
- Manche Installer verweigern Adminrechte (winget-Code `0x8A150056`, z. B.
  Spotify), und benutzerbezogene Programme lassen sich mit Adminrechten nicht
  deinstallieren (`0x8A15007D`). Dafür WPM schließen und normal neu starten.
- Fenster, die keine UAC-Nachfragen sind (Treiber-Vertrauensabfragen,
  Installer, die `--silent` ignorieren, Programme, die nach dem Update neu
  starten), verhindern auch Adminrechte nicht.
- Am besten liegt `wpm.exe` unter `C:\Program Files\WPM`. In beschreibbaren
  Ordnern wie „Downloads“ könnten fremde DLLs neben der EXE mit
  Adminrechten mitgeladen werden (der MSVC-Build aus der GitHub-Action ist
  dagegen gehärtet, siehe `.cargo/config.toml`).

**Fehlermeldungen** zeigen eine Kurzfassung und, wo bekannt, eine Erklärung
mit Abhilfe – z. B. bei Installer-Exitcode **1603** (allgemeiner Fehler, meist
läuft das Programm noch), 1618, 1638 oder 740, oder bei **„kein passendes
Update“** (`0x8A15002B`: die neue Version passt nicht zur Installationsart;
neu installieren oder anheften). Nennt winget ein Installationsprotokoll, gibt
es den Knopf „Protokoll öffnen“, bei nicht passenden Updates „Anheften“. Die
vollständige Ausgabe von winget steht unter „Details“.

**Update installiert, aber weiter in der Liste?** Bei MSIX-Paketen (z. B.
Claude) meldet winget Erfolg, aktiviert die neue Version aber erst, wenn die
Anwendung beendet ist. WPM erkennt das und weist darauf hin. Bleibt das Update
auch danach stehen, erkennt winget die Version falsch – dann das Paket
anheften, vor allem wenn es sich ohnehin selbst aktualisiert.

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

Autostart schreibt den Eintrag `WPM` unter
`HKCU\Software\Microsoft\Windows\CurrentVersion\Run` (keine Adminrechte nötig,
keine Nachfrage beim Anmelden).

## Gut zu wissen

- **winget hat keine maschinenlesbare Ausgabe.** WPM liest die Textausgabe:
  Spalten werden über die Position der Kopfzeile erkannt, nicht über die
  (übersetzten) Spaltennamen. Das funktioniert unabhängig von der
  Windows-Sprache, ist aber naturgemäß empfindlicher als eine echte API.
- **Gekürzte IDs:** Ohne Konsole kürzt winget breite Tabellen auf 120 Zeichen
  und hängt „…“ an. Für solche IDs sucht WPM ohne `--exact` nach dem Anfang
  der ID; ist das nicht eindeutig, meldet winget das.
- **Adminrechte:** WPM selbst braucht keine. Gegen einzelne UAC-Nachfragen
  der Installer hilft „Updates mit Administratorrechten installieren“ (siehe
  oben).
- **Sicherheit:** `winget.exe` wird über seinen vollen Pfad gestartet (App-Alias
  bzw. `PATH` ohne Programmordner und aktuelles Verzeichnis), damit nie eine
  gleichnamige Datei aus dem Programmordner (z. B. dem Download-Ordner) mit
  Adminrechten läuft. Nachgeladene DLLs kommen nur aus `System32`.
- **Hintergrundprüfung** läuft, solange WPM geöffnet ist (auch minimiert).
  Ein Symbol im Infobereich (Tray) gibt es noch nicht.

## Mögliche nächste Schritte

- Tray-Symbol, damit WPM beim Schließen im Hintergrund weiterläuft
- statt Text-Parsing die WinGet-COM-API (`Microsoft.Management.Deployment`)
  oder das PowerShell-Modul `Microsoft.WinGet.Client` nutzen
- weitere Paketmanager wie Scoop oder Chocolatey
- Programmsymbol in der EXE (Ressourcendatei)
