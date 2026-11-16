# Prüfstand v0.1

Lokale Nachweise vom 2026-10-31, Windows x64. Keine Veröffentlichung und kein Remote-CI-Lauf. Am 2026-11-01 auf Commit 3eadb28 wiederholt: fmt, Clippy und 35 Tests erfolgreich. Alle übrigen Nachweise stammen vom 2026-10-31.

Der Gesamtreview über den v0.1-Branch steht aus; bisher liegen nur Reviews je Task und ein gezielter Usability-Review vor.

## Erfolgreich geprüft

- Rust fmt, workspace check, Clippy mit `-D warnings`, 35 Tests (32 Bibliothek, 3 Binary) sowie Debug- und Release-Build mit Lockfile. Fachtests schützen Zeitgrenzen, Pause/Fortsetzen/Reset/einmaligen Ablauf, Konfigurationsvalidierung und echte Produktionslogik für Displayzuordnung, Schreibbündelung, Audio-Gruppierung und Fehlerpfade.
- Lesende reale Displaydiagnose: 27G1G4 und CQ32G4 erkannt, DDC/CI-Helligkeit nicht verfügbar; MD20831 unterstützt und mit 50 % gelesen. Kein internes WMI-Display auf diesem Rechner beobachtet.
- Lesende Audio-Diagnose: echtes Standardgerät, Master und sieben App-/Systemsounds-Gruppen erkannt. Keine Lautstärke-, Mute- oder Helligkeitsänderung ausgeführt.
- Vier echte Slint-Renderer-Aufnahmen (Displays, Audio, Zeit, Einstellungen), 400 × 620, mit realen Gerätewerten: lesbar und unsupported Zeilen korrekt deaktiviert. Debug- und Release-Capture beendeten sich erfolgreich, ohne Einstellungsdatei oder verbleibenden App-Prozess. Capture prüft den Renderer, nicht native Tray-Interaktion.
- Astro check: null Fehler/Warnungen/Hinweise. Statischer Build erfolgreich. Lokaler HTTP-Test: 200, deutscher Titel/Sprache, Viewport, Beispielwert-Kennzeichnung und tatsächlicher Repository-Link. Server danach gestoppt und Nichterreichbarkeit geprüft.
- Release-ZIP erstellt; Inhalt: exe, install.ps1, uninstall.ps1, README.md; externe SHA256-Datei. Installiertes Binary stimmt mit Release-Binary überein, `--help` beendet erfolgreich. Isolierter Test ohne Shortcut: Installation, Upgrade, Deinstallation; settings.json und fremde Sentineldatei bleiben erhalten. Unverwaltete nichtleere Ziele und Deinstallation ohne passende Installationsmarkierung werden abgelehnt. PowerShell-Syntaxprüfung erfolgreich.
- Logging: Logdatei unter `%LOCALAPPDATA%\NexPanel\logs\nexpanel.log` (Rotation >1 MB nach `nexpanel.old.log`). Level per Umgebungsvariable `NEXPANEL_LOG` (`error|warn|info|debug|trace`), Standard Info (Release) bzw. Debug.

## Reproduzieren

Cargo-/Astro-Befehle stehen in [Setup](setup.md). Distribution:

```powershell
./scripts/package.ps1
./scripts/test-package.ps1
```

Der Distributionstest verwendet einen eigenen GUID-Ordner im Temp-Verzeichnis, keine Shortcut-Erstellung und keinen Registry-Schreibtest. Er startet das installierte Binary nur mit `--help`. Seine Testdateien bleiben zur Prüfung im ausgegebenen Temp-Pfad erhalten. Die Deinstallation liest den realen Autostartwert, entfernt ihn aber nur bei exakter Übereinstimmung mit dem temporären Installationspfad. Der Test setzt oder aktiviert keinen Autostart.

## Manuelle Tests Timer (Benachrichtigung, Standby, Systemuhr)

Nur manuell prüfbar; nicht automatisiert.

1. Benachrichtigung: Tab Zeit, Countdown 10 s starten, Panel schließen. Nach Ablauf erscheint die Windows-Benachrichtigung „NexPanel – Timer abgelaufen“ (mit Systemton, außer in Ruhezeit/Nicht stören). Das Panel öffnet sich nicht und stiehlt keinen Fokus: in einer Vollbild-App (Spiel/Video) läuft die Eingabe ungestört weiter. Genau eine Benachrichtigung je Ablauf.
2. Klick auf die Benachrichtigung öffnet das Panel auf dem Tab Zeit mit Ablauf-Banner. Bei bereits sichtbarem Panel erscheint nur das Banner, kein Fokuswechsel.
3. Standby: Countdown 2 min starten, Rechner in Standby, nach > 2 min aufwecken. Nach dem Aufwachen erscheint sofort (ohne bis zu 200 ms Tick abzuwarten) genau eine Benachrichtigung; Log enthält keine Uhrwarnung.
4. Standby kürzer als Restzeit: Restzeit nach dem Aufwachen entspricht der Restzeit minus Standbydauer (Standby zählt mit).
5. Systemuhr: Countdown 2 min und Stoppuhr starten, Systemzeit um 1 h vor bzw. zurück stellen. Restzeit und Stoppuhr laufen unverändert weiter; `nexpanel.log` enthält eine Warnung „Systemuhr sprang um … ms“.

## Noch nicht empirisch bewiesen

Native Tray-Klicks, Esc/Fokusübergänge, mehrere DPI-/Taskleistenkonfigurationen, echte Hardware-Schreibzugriffe einschließlich Wiederherstellung, WMI-Schreiben, Betriebssystem-Schlaf, Hotplug, reale Audio-Gerätewechsel und fremde Rechner. Zwei erkannte Monitore dieses Systems erlauben keine DDC-Helligkeitsregelung; das ist sichtbar und keine behauptete gelöste Treiberbeschränkung.

Langsame synchrone DDC-/WMI-Treiber können den jeweiligen Worker blockieren; kein harter abbrechbarer Treibertimeout. UI bleibt vom Worker getrennt. Audio aktualisiert per Polling. Tray-Positionierung nutzt den Cursor-Monitor als dokumentierten Fallback, nicht private Tray-Icon-Geometrie. Das Panel wird bei hoher DPI oder kleinen Arbeitsbereichen auf die verfügbare Fläche mit Rand verkleinert; Inhalte bleiben scrollbar. Countdown und Stoppuhr laufen auf der monotonen Uhr `QueryInterruptTime` (zählt Standby mit) und folgen Systemzeitsprüngen nicht; Sprünge > 2 s werden als Warnung geloggt. Timerzustand wird nicht über App-Neustarts gespeichert.

Bekannte kleinere Lücken der Distribution: Der Konflikt mit einer fremden Startmenü-Verknüpfung wird erst nach dem Kopieren erkannt. Die Schutzpfade für abweichende Installationsmarkierung, verbotene Ziele, Verzeichnisverknüpfungen und gesperrte Pfade sind nicht alle getestet.

Performance-Zielwerte aus NexPanel.md sind nicht gemessen. Startmenü-/Autostartintegration und fremde Installer-Umgebungen brauchen weitere praktische Prüfung. Die CI-Konfiguration enthält Tests, Release-Build und isolierten Pakettest, wurde aber nicht remote ausgeführt. Browser-Layoutprüfung war ohne Browseroberfläche nicht verfügbar; responsive CSS und HTML wurden gebaut und per HTTP geprüft. Projektlizenz, Slint-/Abhängigkeitsfreigaben (siehe [licensing.md](licensing.md)) und Codesignatur sind vor Veröffentlichung zu klären; es wurde keine Lizenz gewählt.
