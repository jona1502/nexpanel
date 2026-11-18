# Release-Prozess

Releases entstehen über den Workflow `.github/workflows/release.yml` (Windows, Rust 1.96.1). Er legt standardmäßig einen **Entwurf** (Draft) an; veröffentlicht wird manuell.

## Release erstellen

1. Version in `apps/desktop/Cargo.toml` anheben (und `Cargo.lock` aktualisieren). Der Tag muss exakt `v<Version>` lauten, sonst bricht der Workflow ab.
2. `scripts/package.ps1` liest die Version aus `apps/desktop/Cargo.toml` (`[package] version`, Helfer `scripts/common.ps1`) und erzeugt `dist/nexpanel-<Version>-windows-x64.zip` plus `.sha256`, passend zur Prüfung im Workflow. Das Paket enthält `VERSION.txt` und `THIRD-PARTY-NOTICES.md` (erzeugt von `scripts/gen-third-party-notices.ps1` aus `cargo metadata`, inkl. Lizenztexten aus der lokalen Cargo-Registry; die Registry-Quellen müssen vorhanden sein, ggf. vorher `cargo fetch --locked`).
3. Statuszeilen in `README.md`, `docs/testing.md` und `docs/package-readme.md` aktualisieren.
4. Änderungen per PR nach `main` bringen, CI muss grün sein.
5. Tag setzen und pushen: `git tag vX.Y.Z` und `git push origin vX.Y.Z`. Alternativ: Actions → Release → „Run workflow“ mit Tag (und `draft`).
6. Tag-Push erzeugt immer einen Entwurf. Nur der manuelle Lauf mit `draft = false` veröffentlicht direkt. Versionen unter 1.0 werden als Pre-release markiert.
7. Entwurf unter GitHub → Releases prüfen: Notizen (automatisch aus den Commits seit dem letzten Tag, plus fester Hinweisblock), Anhänge (`.zip` und `.sha256`).
8. Zip herunterladen und lokal prüfen:
   ```powershell
   (Get-FileHash .\nexpanel-X.Y.Z-windows-x64.zip -Algorithm SHA256).Hash.ToLower()
   Get-Content .\nexpanel-X.Y.Z-windows-x64.zip.sha256
   ```
   Beide Werte müssen übereinstimmen.
9. Entwurf veröffentlichen („Publish release“).

## Vor dem ersten öffentlichen Release

- [ ] Lizenz von NexPanel gewählt (`LICENSE`-Datei, siehe `docs/licensing.md`).
- [x] Drittlizenz-Datei im Paket (`THIRD-PARTY-NOTICES.md`, automatisch erzeugt).
- [ ] Lizenzen der Abhängigkeiten rechtlich geprüft (`docs/licensing.md`); Slint-Bedingungen und „Über NexPanel“ abgeglichen.
- [ ] Handtests aus `docs/testing.md` durchgeführt.
- [ ] Signierung entschieden: unsigniert (SmartScreen-Warnung) oder Zertifikat.
- [ ] Statusformulierung in `README.md` angepasst (kein „lokal, nicht gepusht“ mehr).

## Auto-Updater

NexPanel kann sich selbst aktualisieren (Einstellungen → Updates). Der Updater arbeitet nur mit dem hier beschriebenen Release-Format; ändert sich Paketname, Inhalt oder Prüfsummenformat, muss `apps/desktop/src/update/` angepasst werden.

### Was der Updater tut

- Prüft beim Start und danach höchstens einmal in 24 Stunden (Schalter „Automatisch prüfen (täglich)“, Standard an; Zeitstempel `last_update_check` in `settings.json`) sowie auf Klick bei „Jetzt prüfen“. Die Prüfung liest `https://api.github.com/repos/jona1502/nexpanel/releases` und nimmt die höchste Version, die neuer als die laufende ist. **Pre-Releases zählen mit** (alle Versionen unter 1.0 sind Pre-Releases). Entwürfe sind für den Updater unsichtbar.
- Lädt und installiert **nie von selbst**: Erst der Klick auf „Jetzt aktualisieren“ startet Download, Prüfung, Installation und Neustart. Beim Neustart gehen laufende Timer verloren (der Timerzustand wird ohnehin nicht über App-Neustarts gespeichert).
- Ersetzt nur Installationen aus dem Setup-Programm oder `scripts/install.ps1` (Ordner mit passender `.nexpanel-install.json`). Ein Entwicklungs-Build oder ein kopierter Ordner bekommt den Hinweis „nicht mit install.ps1 eingerichtet“ und kann nur prüfen.

### Anforderungen an ein Release

Damit ein Release für den Updater sichtbar ist, müssen beide Anhänge exakt so heißen (Tag `vX.Y.Z[-pre]`, `X.Y.Z[-pre]` ist die Version):

| Anhang | Inhalt |
|---|---|
| `nexpanel-X.Y.Z-windows-x64-setup.exe` | Inno-Setup-Installer (`scripts/installer.iss`, gebaut von `package.ps1`, braucht Inno Setup 6; `-SkipInstaller` überspringt ihn). Nur für Menschen, der Updater nutzt ihn nicht |
| `nexpanel-X.Y.Z-windows-x64.zip` | Von `scripts/package.ps1`: `nexpanel.exe`, `install.ps1`, `uninstall.ps1`, `README.md`, `THIRD-PARTY-NOTICES.md`, `VERSION.txt`, alles flach ohne Unterordner |
| `nexpanel-X.Y.Z-windows-x64.zip.sha256` | Eine Zeile `<sha256-hex>  <Dateiname>` (Ausgabe von `package.ps1`) |

Der Updater lehnt ab: Zip mit Pfadanteilen, `..`, Laufwerken oder Streams im Dateinamen (Zip-Slip), fehlendes `nexpanel.exe` oder `VERSION.txt`, `VERSION.txt` ungleich der Release-Version, Version nicht neuer als die laufende (kein Downgrade), Download über 100 MiB, falsche oder fremde Prüfsumme. Die Download-URLs werden aus dem Tag gebildet, nicht aus der API-Antwort übernommen.

### Netzwerk

Nur HTTPS, nur diese Hosts (exakt; Weiterleitungen werden einzeln geprüft, höchstens 5): `api.github.com` (Release-Liste), `github.com` (Download-URL), `objects.githubusercontent.com` und `release-assets.githubusercontent.com` (Ziel der GitHub-Weiterleitung für Release-Anhänge). Umsetzung mit WinHTTP (keine zusätzliche Abhängigkeit), alles im Hintergrund-Thread, Timeouts je Schritt, Gesamtfrist 30 s für die Liste und 10 min für das Paket. Übertragen werden die Anfrage-URL, die IP-Adresse und der Kopf `User-Agent: NexPanel-Updater/<Version>`; keine Kennung, keine Telemetrie. Wer keine Abfrage bei GitHub will, schaltet „Automatisch prüfen“ aus und klickt nie auf „Jetzt prüfen“.

### Sicherheitshinweis

- Die Releases sind **nicht signiert**. Die SHA256-Prüfsumme liegt im selben Release wie das Zip. Sie beweist deshalb nur, dass der Download vollständig und unverändert angekommen ist, **nicht**, dass das Release echt ist: Wer das Release (oder das GitHub-Konto) kontrolliert, kann Zip und Prüfsumme gemeinsam austauschen. Authentizität gäbe erst eine Signatur (Authenticode oder minisign) mit einem fest eingebauten öffentlichen Schlüssel; das ist eine offene Entscheidung (siehe „Signierung“ oben).
- Vom Updater geschriebene Dateien tragen keine Mark-of-the-Web-Kennzeichnung; SmartScreen warnt beim Neustart daher nicht.
- Der Hinweistext im Updates-Abschnitt der Einstellungen sagt dasselbe über die Prüfsumme.

### Austausch der laufenden Datei

Die laufende `nexpanel.exe` kann unter Windows umbenannt, aber nicht überschrieben oder gelöscht werden. Ablauf: alle Dateien als `<Name>.new` schreiben (fehlende Schreibrechte fallen hier auf, ohne etwas zu verändern), dann je Datei `<Name>` → `<Name>.old` und `<Name>.new` → `<Name>`, die `nexpanel.exe` zuletzt. Jeder Fehler macht alle bereits erfolgten Umbenennungen rückgängig. Danach startet die neue exe mit `--wait-pid <PID>` und wartet bis zu 15 s, bis die alte beendet ist (Single-Instance-Mutex); die alte beendet sich über denselben Weg wie „Beenden“ im Tray-Menü. `nexpanel.exe.old` wird beim nächsten Start gelöscht. `.nexpanel-install.json` erhält die neue Version.

Bleibt nach einem Fehler etwas liegen (`*.old`, `*.new` im Installationsordner), räumt der nächste Start auf. Bei einer unvollständigen Rücknahme meldet der Updater das und empfiehlt eine Neuinstallation per `install.ps1`.
