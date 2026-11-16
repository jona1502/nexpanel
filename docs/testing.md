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

## Manuelle Tests: Popup-Zustand und Tray-Menü

Zustandsmaschine: `apps/desktop/src/popup_state.rs` (tabellengetriebene Unit-Tests, inkl. Press-and-Hold). Die nativen Abläufe sind nur manuell prüfbar.

| Nr. | Aktion | Erwartung |
|---|---|---|
| P1 | Tray-Icon schnell doppelt klicken (Popup zu) | Popup öffnet und schließt wieder (zwei Aktionen), kein Flackern, kein Hänger |
| P2 | Popup offen, einmal auf das Tray-Icon klicken | Popup schließt und öffnet nicht erneut |
| P3 | Popup offen, Maustaste auf dem Tray-Icon 1–3 s gedrückt halten, dann loslassen | Popup schließt beim Drücken und bleibt nach dem Loslassen zu |
| P4 | Wie P3, aber beim Halten vom Icon wegziehen und außerhalb loslassen; danach einmal klicken | Dieser Klick öffnet das Popup normal (nur beim ersten Klick nach einem Abbruch darf das Popup ausnahmsweise zubleiben, höchstens 5 s lang) |
| P5 | Popup offen, auf den Desktop klicken, danach Tray-Icon klicken | Popup öffnet |
| P6 | Popup offen, Esc | Popup schließt sofort; nächster Tray-Klick öffnet |
| P7 | Popup offen, Button „×“ | Schließt; nächster Tray-Klick öffnet |
| M1 | Rechtsklick auf das Tray-Icon | Menü: Öffnen, Trennlinie, Einstellungen, Mit Windows starten, Trennlinie, Beenden; Klick daneben schließt das Menü |
| M2 | Menü „Öffnen“ | Popup öffnet am Tray-Icon |
| M3 | Menü „Einstellungen“ | Popup öffnet auf dem Tab „Einstellungen“ |
| M4 | Menü „Mit Windows starten“ anklicken | Häkchen wechselt beim nächsten Öffnen des Menüs; Checkbox im Tab „Einstellungen“ zeigt denselben Zustand; Run-Eintrag in der Registry entspricht ihm |
| M5 | Checkbox im Einstellungs-Tab umschalten, dann Menü öffnen | Häkchen im Menü entspricht der Checkbox |
| M6 | Menü „Beenden“ | App beendet sich, Icon verschwindet |
| M7 | Maustasten vertauscht (Windows-Einstellung) und P2/P3 wiederholen | Wie P2/P3 |
| M8 | Autostart aktiv, NexPanel läuft; manuell nochmal `nexpanel.exe --background` starten | Zweite Instanz endet still, Panel der laufenden öffnet sich nicht, Log `nexpanel.log` unverändert und intakt |
| M9 | Run-Wert „NexPanel“ auf alten/falschen Pfad setzen (Testprofil), Einstellungen öffnen | Autostart-Checkbox/Menühäkchen zeigt „aus“; Einschalten schreibt den aktuellen Pfad |

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

## DDC/CI-Erkennung ohne Capabilities-Flag (manuell)

| Schritt | Erwartung |
|---|---|
| Alle drei Monitore (27G1G4, CQ32G4, MD20831) angeschlossen, App starten, Display-Tab öffnen | Alle drei zeigen Status „DDC/CI“ mit Slider, keiner „nicht verfügbar“ |
| Slider jedes Monitors bewegen | Nur dieser Monitor ändert die Helligkeit; Wert bleibt nach Panel-Neuöffnen |
| `nexpanel.exe --diagnostics` (nur lesend) | Alle drei mit `brightness: Some(..)` und Status `DDC/CI` |
| Monitor ohne MC_CAPS_BRIGHTNESS-Flag (`ddc-spike list` zeigt `caps=0x0`, `GetMonitorCapabilities ok=false`) | Trotzdem regelbar; Erkennung stützt sich auf `GetMonitorBrightness`, dann VCP 0x10 |
| Monitor mit DDC/CI im OSD ausschalten | Status „DDC/CI: Monitor antwortet nicht (VCP 0x10)“ statt der allgemeinen Meldung |

## Noch nicht empirisch bewiesen

Native Tray-Klicks, Esc/Fokusübergänge, mehrere DPI-/Taskleistenkonfigurationen, echte Hardware-Schreibzugriffe einschließlich Wiederherstellung, WMI-Schreiben, Betriebssystem-Schlaf, Hotplug, reale Audio-Gerätewechsel und fremde Rechner. Die frühere Annahme, 27G1G4 und CQ32G4 erlaubten keine DDC-Regelung, war ein Erkennungsfehler (siehe Abschnitt DDC/CI-Erkennung); Schreibzugriffe auf diese Monitore sind noch nicht manuell bestätigt.

Langsame synchrone DDC-/WMI-Treiber können den jeweiligen Worker blockieren; kein harter abbrechbarer Treibertimeout. UI bleibt vom Worker getrennt. Audio aktualisiert per Polling. Tray-Positionierung nutzt den Cursor-Monitor als dokumentierten Fallback, nicht private Tray-Icon-Geometrie. Das Panel wird bei hoher DPI oder kleinen Arbeitsbereichen auf die verfügbare Fläche mit Rand verkleinert; Inhalte bleiben scrollbar. Countdown und Stoppuhr laufen auf der monotonen Uhr `QueryInterruptTime` (zählt Standby mit) und folgen Systemzeitsprüngen nicht; Sprünge > 2 s werden als Warnung geloggt. Timerzustand wird nicht über App-Neustarts gespeichert.

Bekannte kleinere Lücken der Distribution: Der Konflikt mit einer fremden Startmenü-Verknüpfung wird erst nach dem Kopieren erkannt. Die Schutzpfade für abweichende Installationsmarkierung, verbotene Ziele, Verzeichnisverknüpfungen und gesperrte Pfade sind nicht alle getestet.

Performance-Zielwerte aus NexPanel.md sind nicht gemessen. Startmenü-/Autostartintegration und fremde Installer-Umgebungen brauchen weitere praktische Prüfung. Die CI-Konfiguration enthält Tests, Release-Build und isolierten Pakettest, wurde aber nicht remote ausgeführt. Browser-Layoutprüfung war ohne Browseroberfläche nicht verfügbar; responsive CSS und HTML wurden gebaut und per HTTP geprüft. Projektlizenz, Slint-/Abhängigkeitsfreigaben (siehe [licensing.md](licensing.md)) und Codesignatur sind vor Veröffentlichung zu klären; es wurde keine Lizenz gewählt.

## Akzent-Dropdown und Esc (fix/esc-dropdown)

Die Std-ComboBox ist durch ein eigenes `Dropdown` + `DropdownList` (`ui/dropdown.slint`) ersetzt; die Liste wird als Overlay im Fenster gezeichnet (Slint-1.18-`PopupWindow.is-open` crasht den Compiler im Debug-Build). Der Offen-Zustand steht in `MainWindow.dropdown-open`; der Esc-Handler in `main.rs` schließt bei offener Liste nur diese (`esc_action`), sonst das Panel. Beim Ausblenden des Panels wird die Liste zurückgesetzt.

Manuelle Tests (Einstellungen-Tab):

- [ ] Akzentliste öffnen, Esc: nur die Liste schließt, das Panel bleibt. Zweites Esc blendet das Panel aus.
- [ ] Akzentliste öffnen, außerhalb der Liste klicken: nur die Liste schließt.
- [ ] Eintrag wählen: Akzent ändert sich, Liste schließt, Wert bleibt nach Neustart erhalten.
- [ ] Liste öffnen, Pfeil hoch/runter und Enter wählen einen Eintrag.
- [ ] Liste offen, Panel per Tray-Klick/Fokusverlust ausblenden und neu öffnen: Liste ist geschlossen.
- [ ] Dropdown ist bei `settings-ready == false` (z. B. defekte Einstellungsdatei) nicht bedienbar.

## Audio: Ereignisse statt Polling (feat/audio-events)

Der Audio-Worker pollt nicht mehr jede Sekunde. COM-Callbacks (`IMMNotificationClient`, `IAudioEndpointVolumeCallback`, `IAudioSessionNotification`, je Sitzung `IAudioSessionEvents`) senden nur eine kleine Nachricht (`AudioCommand::Changed`) an den Worker. Dieser bündelt Bursts (40 ms ruhig, höchstens 250 ms), liest nur das Geänderte neu und sendet einen Snapshot nur bei Änderung. Sicherheits-Refresh alle 30 s. Bei verborgenem Panel werden Änderungen gesammelt und beim Öffnen angewandt; Schreibbefehle werden immer sofort verarbeitet.

Manuelle Tests (Panel offen und danach verborgen/neu geöffnet; Log: `NEXPANEL_LOG=debug`):

- Master-Lautstärke und Stumm im Windows-Mixer bzw. mit Lautstärketasten ändern: Slider und Stumm-Schalter folgen sofort.
- Lautstärke/Stumm einer App im Windows-Lautstärkemixer ändern: Zeile folgt.
- Audio-App starten (Browser, Mediaplayer, Ton abspielen) und beenden: Zeile erscheint bzw. verschwindet; mehrere Sitzungen einer App bleiben eine Zeile.
- Standardausgabegerät wechseln (z. B. Lautsprecher zu Kopfhörer) und Gerät ab-/anstecken: Master und Sitzungen wechseln auf das neue Gerät; kein Fehlerstatus dauerhaft.
- Slider im Panel ziehen: kein Springen oder Abbrechen; eigene Änderungen führen nicht zu Flackern.
- App starten, Panel mehrfach hintereinander öffnen und schließen: Liste ohne veraltete Einträge und ohne Flackern (beim Öffnen keine vollständige Neuerkennung, nur Werte und Master werden aufgefrischt).
- Slider einer App ziehen, während dieselbe App einen weiteren Stream öffnet: Zeile bleibt, Ziehen bricht nicht ab, kein Fehler "Audio-Sitzung ist nicht mehr verfügbar" (Gruppen-ID ist `Endpunkt|pid`).
- Leerlauf bei verborgenem Panel: kaum CPU-Wakeups des Audio-Threads (Prozess-Explorer, Thread `nexpanel-audio`).
- Beenden der App: kein Hängen, kein Absturz (Unregistrierung der Callbacks).

## Manuell: Idle-Wakeups und Hotplug (perf/idle-and-hotplug)

Intervalle: UI-Tick 200 ms bei sichtbarem Panel, 1 s bei verborgenem Panel (Countdown-Ablauf per One-Shot-Timer zur exakten Deadline). Display-Sicherheitspoll 30 s (sichtbar) bzw. 60 s (verborgen); Helligkeit wird nur bei sichtbarem Panel gelesen (max. alle 30 s) und einmal beim Öffnen. Hotplug kommt per `WM_DISPLAYCHANGE`/`DBT_DEVNODES_CHANGED`, entprellt mit 300 ms.

| Test | Erwartung |
|---|---|
| Panel verbergen, 60 s warten, Wakeups messen (siehe unten) | Deutlich weniger Kontextwechsel/s als vorher (ca. 1 Tick/s statt 5; Display-Worker nur bei Poll/Ereignis) |
| Countdown 10 s starten, Panel verbergen | Ablauf exakt nach 10 s (Panel öffnet sich, Alarm), kein Verzug von bis zu 1 s |
| Stoppuhr starten, Panel verbergen, nach 10 s öffnen | Anzeige springt sofort auf die richtige Zeit |
| Panel verbergen, Display-Änderung am Systemmonitor (Auflösung wechseln) und Panel öffnen | Zeilen sind sofort aktuell |
| Panel offen: Monitor ein-/ausstecken oder Auflösung ändern | Liste aktualisiert sich nach unter 1 s, nicht erst nach Sekunden |
| Panel verborgen: Monitor ein-/ausstecken, dann Panel öffnen | Liste stimmt sofort; Log zeigt Display-Erkennung beim Hotplug |
| Helligkeit am Monitor-OSD ändern, Panel verborgen lassen, dann öffnen | Slider zeigt beim Öffnen den neuen Wert |
| Monitor aus dem Standby aufwecken | DDC-Zeile erholt sich innerhalb ca. 5 s (zweite Erkennung 3 s nach der ersten, wenn ein Monitor keine Regelung hat) |
| Monitor abziehen, während ein Helligkeits-Slider gezogen wird | Fehlerstatus, kein Dauerfeuer von Erkennungen im Log; neue Erkennung frühestens nach 5 s bzw. beim Hotplug-Ereignis |
| Zweite Instanz starten, während das Panel verborgen ist | Panel öffnet sich innerhalb ca. 1 s |

### Idle-CPU und Wakeups messen

- Process Explorer: `nexpanel.exe` > Properties > Threads; Spalte „Context Switch Delta" bzw. Performance-Graph. Panel verborgen, 60 s beobachten.
- PowerShell (Kontextwechsel/s des Prozesses):

```powershell
$p = Get-Process nexpanel
Get-Counter "\Process($($p.ProcessName))\Context Switches/sec" -SampleInterval 5 -MaxSamples 12
Get-Counter "\Process($($p.ProcessName))\% Processor Time" -SampleInterval 5 -MaxSamples 12
```

- Vergleich: Stand `feat/logging` (200-ms-Tick ohne Unterbrechung, Display-Worker mit 100-ms-Wakeup) gegen diesen Branch, jeweils Panel verborgen und ohne Timer.


## Manuell: kompaktes UI, Audio-Icons, Zeitfeld (feat/audio-compact-ui)

Panel ist 392 x 440 px (vorher 400 x 620). Vorschau ohne Hardwarezugriff: `nexpanel.exe --capture-preview <Ordner>`.

| Prüfung | Erwartung |
| --- | --- |
| Audio-Tab öffnen | Kopfzeile mit Name des Standardgeräts (Fallback „Lautstärke“) und Pfeil rechts; Pfeil klappt die App-Liste ein/aus |
| Master-Zeile | Lautsprecher-Icon links, dünner Slider, große Zahl rechts; keine „Stumm“-Checkbox |
| Klick auf Lautsprecher-Icon (ganze 32-px-Fläche, Hover-Highlight) | Master stumm: Icon mit Kreuz, Slider und Zahl gedimmt; erneuter Klick hebt auf |
| Klick auf App-Icon (28-px-Fläche, Hover-Highlight) | App stumm: Icon gedimmt mit rotem x, Slider und Zahl gedimmt; Slider bleibt ziehbar |
| Mute nur über Icons | Nirgends Checkbox oder Button für Stumm im Audio-Tab |
| App-Icons | Echte Exe-Icons; Systemsounds/Store-Apps/ohne Zugriff: Kachel mit Anfangsbuchstabe |
| Slider ziehen (Audio, Display) | Wert folgt der Maus ohne Zucken, Zahl rechts aktualisiert; nach Loslassen kein Zurückspringen innerhalb 1,5 s |
| Klick auf die Spur | Thumb springt an die Position |
| Viele Sitzungen (> 7) | App-Liste scrollt, Master bleibt sichtbar |
| Display-Tab | Kopfzeile (Koppeln-Chip, Refresh-Icon), eine Zeile je Monitor: Monitor-Icon, Slider, Zahl; Beschriftung mit Namen nur bei mehr als einem Monitor; nicht regelbar: gedimmt, „—“, Grund in der Beschriftung |
| Koppeln-Chip | Schaltet verbundene Helligkeit; Zustand bleibt nach Neustart (Einstellungen) |
| Zeit-Tab: Ziffern 1, 5, 3 tippen | 00:00:01 → 00:00:15 → 00:01:53 |
| Rücktaste / Entf | Letzte Ziffer entfernen / alles auf 00:00:00 |
| Pfeil auf/ab, Mausrad über Segment | Segment +/- 1 mit Übertrag; Pfeil links/rechts wechselt das Segment; Klick fokussiert das Segment |
| Eingabe 9 0 (00:00:90), Enter | Start; Anzeige normalisiert auf 00:01:30 |
| Während der Countdown läuft oder pausiert | Große Anzeige zeigt Restzeit (HH:MM:SS), nicht editierbar; „Zurücksetzen“ macht das Feld wieder editierbar |
| Esc im Zeitfeld | Esc schließt weiterhin das Panel (wird vor Slint behandelt, nicht abgefangen) |

## Manuell: Tray und Popup-Robustheit (fix/tray-popup-robustness)

| Test | Erwartung |
|---|---|
| Panel offen, schneller Doppelklick (< 100 ms) auf das Tray-Icon | Panel endet geschlossen (beide Klicks zählen) |
| Leertaste/Enter auf fokussiertem Tray-Icon | Genau ein Toggle (NIN_SELECT/NIN_KEYSELECT werden zusammengefasst) |
| Fokusassistent (Nicht stören) an, Countdown ablaufen lassen | Piepton bzw. Hinweis; beim nächsten Öffnen steht das Panel auf dem Timer-Tab mit Banner "Dein Timer ist abgelaufen" |
| Panel per Tray-Klick öffnen, Fokus sofort anderswohin legen (z. B. Explorer aktivieren), ohne dass Blur ankommt | Panel schließt spätestens nach ca. 0,4 s; Esc und Klick daneben schließen es sonst weiterhin |
| Maustaste auf Tray-Icon drücken, Panel schließt per Blur, Maus außerhalb des Icons loslassen, dann innerhalb 5 s auf das Icon klicken | Panel oeffnet sich beim ersten Klick (kein verschluckter Klick) |
| Panel von aussen verbergen (z. B. Fenster per Tool ausblenden), dann Tray-Klick | Panel oeffnet sich mit einem Klick |
| Standby/Ruhezustand während ein Countdown abläuft, Resume | Kein Absturz; Timer-Ablauf wird gemeldet |
| Autostart-Haken im Tray-Menü direkt nach dem Start (bevor Einstellungen geladen sind) | Keine Änderung, kein falscher Haken |
| Start mit gesperrtem Fenster-Backend/ohne Tray (Fehlerfall) | Fehlerdialog "NexPanel – Start fehlgeschlagen" und Logeintrag statt stillem Beenden |

### Tray-Icon-Größen

| Schritt | Erwartung |
|---|---|
| Skalierung 100 / 125 / 150 / 200 % einstellen (Anzeige ab-/anmelden bzw. Explorer neu starten) | Tray-Icon jeweils scharf, nicht unscharf skaliert (16/20/24/32 px) |
| Skalierung bei laufender App ändern | Icon wird neu gewählt, kein doppeltes Icon |

## Manuell: System-Tab (feat/system-monitor)

Vorschau: `nexpanel.exe --capture-preview <Ordner>` speichert jetzt fünf Tabs (`04-system.png` ist der System-Tab). Reihenfolge der Tabs: Displays, Audio, Zeit, System, Einstellungen.

| Prüfung | Erwartung |
| --- | --- |
| System-Tab öffnen, Task-Manager daneben (Registerkarte Leistung) | CPU in % innerhalb weniger Prozentpunkte des Task-Managers (Gesamtauslastung) |
| RAM | „genutzt / gesamt GB“ und % entsprechen „In Verwendung“ / „Insgesamt“ im Task-Manager (auf 0,1 GB gerundet) |
| GPU-Last | Entspricht dem höchsten Engine-Wert (meist 3D) der ausgelasteten GPU im Task-Manager; Last erzeugen z. B. mit Video im Browser oder einem Spiel |
| VRAM | „Dedizierter GPU-Speicher“ des Task-Managers; Gesamtwert aus DXGI |
| Balkenfarbe | Akzent unter 80 %, bernstein ab 80 %, rot ab 95 % |
| GPU-Name | Kleine graue Zeile über der GPU-Zeile |
| Tab wechseln oder Panel verbergen | Messung stoppt: Thread `nexpanel-system` im Process Explorer (Threads) zeigt 0 Zyklen/Wakeups; zurück im System-Tab nach ca. 0,3 s wieder Werte |
| Aktiver System-Tab | Höchstens ein Wakeup pro Sekunde, CPU des Threads unter 0,5 % |
| Notebook mit iGPU + dGPU | Kopfzeile zeigt die ausgelastetere GPU; bei Gleichstand die mit mehr VRAM; kein Flackern bei Last auf einer GPU |
| VM / alter Treiber ohne GPU-Zähler | „GPU: nicht verfügbar“ gedimmt, CPU und RAM laufen weiter, keine Fehlermeldung, Warnung im Log |
| Timer-Benachrichtigung anklicken | Öffnet Tab „Zeit“ (Index 2); Tray-Menü „Einstellungen“ öffnet Tab 4 |
| Lange laufen lassen (> 1 min mit GPU-Last wechselnder Prozesse) | Werte bleiben plausibel (PDH-Neuaufbau alle 10 Messungen), kein Speicher- oder Handle-Wachstum |

## Feste Panelgröße (fix/audio-collapse-resize)

| Aktion | Erwartung |
|---|---|
| Alle Tabs nacheinander öffnen | Audio-Tab hat keine Einklapp-Funktion; Panelgröße bleibt in allen Tabs 392×440 |
