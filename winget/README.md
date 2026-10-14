# WPM bei winget einreichen

Dieser Ordner enthält das winget-Manifest für die erste Einreichung von WPM
(`Stanischewski.WPM`) im Community-Repository
[microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs). Danach
übernimmt die GitHub-Action die Updates automatisch (siehe unten).

```
winget/manifests/s/Stanischewski/WPM/0.2.2/
├── Stanischewski.WPM.yaml                  Version
├── Stanischewski.WPM.installer.yaml        Download-Adresse und Prüfsumme
├── Stanischewski.WPM.locale.en-US.yaml     Beschreibung (Standard, Englisch)
└── Stanischewski.WPM.locale.de-DE.yaml     Beschreibung (Deutsch)
```

Typ `portable`: winget lädt `wpm.exe` herunter, legt sie unter
`%LOCALAPPDATA%\Microsoft\WinGet\Packages\…` ab und macht sie als Befehl `wpm`
verfügbar. Einen Startmenü-Eintrag legt winget dafür nicht an.

## 1. Vorab prüfen (Windows)

Im Terminal:

```powershell
winget search --moniker wpm
```

Taucht dabei ein anderes Paket auf, in
`Stanischewski.WPM.locale.en-US.yaml` die Zeile `Moniker: wpm` löschen.

Optional: `wpm.exe` vom Release bei [VirusTotal](https://www.virustotal.com)
prüfen lassen. Meldet ein Scanner einen Fehlalarm, ihn vorher bei Microsoft
melden (<https://www.microsoft.com/wdsi/filesubmission>) – einen Fehlalarm im
Prüflauf von winget können die Moderatoren nicht übergehen.

## 2. Lokal testen

In einem Terminal **mit Administratorrechten** einmalig:

```powershell
winget settings --enable LocalManifestFiles
```

Dann in einem normalen Terminal im WPM-Ordner:

```powershell
winget validate --manifest winget\manifests\s\Stanischewski\WPM\0.2.2
winget install --manifest winget\manifests\s\Stanischewski\WPM\0.2.2
```

Danach in einem **neuen** Terminal bzw. über Win+R `wpm` starten und kurz
ausprobieren. Anschließend die Testinstallation wieder entfernen – sonst
entsteht später ein zweiter Eintrag unter „Apps“:

```powershell
winget uninstall --manifest winget\manifests\s\Stanischewski\WPM\0.2.2
```

Gründlicher (optional): mit `Tools\SandboxTest.ps1` aus dem winget-pkgs-Repository
in der Windows-Sandbox testen, auch einmal ohne Grafikkarte
(`<vGPU>Disable</vGPU>` in einer `.wsb`-Datei). Die Prüfung von winget startet
WPM nach der Installation in einer virtuellen Maschine; WPM zeichnet mit
OpenGL, deshalb lohnt der Test.

## 3. Einreichen

Am einfachsten mit Microsofts Werkzeug **wingetcreate**:

```powershell
winget install Microsoft.WingetCreate
wingetcreate submit --prtitle "New package: Stanischewski.WPM version 0.2.2" winget\manifests\s\Stanischewski\WPM\0.2.2
```

Ohne `--token` öffnet sich die GitHub-Anmeldung im Browser. wingetcreate legt
bei Bedarf den Fork `Stanischewski/winget-pkgs` an und öffnet den Pull Request.

Danach im Pull Request:

1. **Beitragsvereinbarung (CLA):** Fragt der Bot danach, mit
   `@microsoft-github-policy-service agree` antworten.
2. **Prüfung abwarten:** Ein Bot prüft Manifest, Download, Virenscan und
   Installation und setzt Labels. Fordert ein Label etwas an, innerhalb von
   5 Tagen antworten – sonst wird der Pull Request geschlossen.
3. **Moderator:** Neue Pakete warten derzeit oft mehrere Wochen bis etwa zwei
   Monate auf die Freigabe. Danach wird automatisch zusammengeführt; kurz
   darauf ist WPM mit `winget install Stanischewski.WPM` installierbar.

## 4. Danach: Updates automatisch einreichen

Sobald die erste Version aufgenommen ist, reicht die Action
(`.github/workflows/build.yml`, Job `winget`) jede neue Version selbst ein.
Dafür im WPM-Repository unter *Settings → Secrets and variables → Actions*:

1. **Secret** `WINGET_TOKEN`: ein klassisches GitHub-Token
   (*Settings → Developer settings → Personal access tokens → Tokens (classic)*)
   mit den Bereichen `public_repo` und `workflow`, am besten mit Ablaufdatum.
2. **Variable** `WINGET_PUBLISH` mit dem Wert `true`.

Der Fork `Stanischewski/winget-pkgs` muss bestehen bleiben. Updates bestehender
Pakete werden meist innerhalb eines Tages freigegeben.

Dieses Manifest wird danach nicht mehr gebraucht; die Action erzeugt die
Manifeste für neue Versionen selbst.
