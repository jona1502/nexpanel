# NexPanel

> **Everything in one place.**  
> Ein schnelles, minimalistisches und erweiterbares Windows Control Center – entwickelt mit **Rust**.

**Projektstatus:** Technische Spezifikation v0.2 (16. November 2026)  
**Plattform:** Windows 11 (zunächst x64)  
**Sprache:** Rust  
**UI:** Slint  
**Geplanter Start:** Version 0.1 – Minimal Control Center  
**Primäre Bedienung:** System-Tray / Infobereich rechts in der Windows-Taskleiste, neben Uhrzeit und Datum  
**Implementierungsprinzip:** Win32-Tray-Host + Slint-Popup + entkoppelter Rust-Core

---

## 1. Vision

**NexPanel** vereint häufig benötigte Windows-Funktionen in einer einzigen, leichtgewichtigen Anwendung. Statt mehrere Tools für Monitorhelligkeit, Lautstärke, Timer und Systeminformationen zu öffnen, lassen sich diese Funktionen über ein zentrales Panel direkt aus dem **System-Tray** bedienen.

Der Name setzt sich sinngemäß aus **„Nex“** (angelehnt an *Next* und *Nexus*) und **„Panel“** (Bedienfeld) zusammen: ein zentrales Control Panel der nächsten Generation.

### Kernanforderung: System-Tray statt klassischer Taskleisten-App

**NexPanel ist in erster Linie eine Tray-App.** Das Symbol befindet sich im **Windows-Infobereich (System-Tray)** rechts unten bei den Statussymbolen, neben Uhrzeit und Datum. Es ist **kein dauerhaft sichtbarer, normaler Taskleisten-Button** für ein großes Hauptfenster. Je nach Windows-Einstellung kann das Icon im Überlaufmenü (`^`) liegen; Nutzer können es im sichtbaren Infobereich platzieren.

Erwartetes Verhalten:

1. **Start:** NexPanel läuft unauffällig im Hintergrund und zeigt ein Tray-Icon. Kein großes Fenster wird automatisch geöffnet.
2. **Linksklick auf das Icon:** Ein kleines, rahmenloses Control-Center-Popup erscheint direkt **oberhalb bzw. beim angeklickten Tray-Icon**.
3. **Bedienung:** Monitorhelligkeit, Audio, Timer und weitere Quick Controls sind sofort erreichbar.
4. **Schließen:** `Esc`, erneuter Klick auf das Icon oder Klick außerhalb blendet das Popup aus; **die App läuft weiter**.
5. **Rechtsklick auf das Icon:** Kontextmenü, etwa `NexPanel öffnen`, `Einstellungen`, `Mit Windows starten` und `Beenden`.
6. **Einstellungen:** Bei Bedarf darf sich ein separates größeres Fenster öffnen, das aber nicht die primäre Benutzeroberfläche ist.
7. **Windows-Integration:** Position und Größe berücksichtigen Taskleistenposition, mehrere Monitore, DPI-Skalierung und den tatsächlichen verfügbaren Bildschirmbereich. Windows 11 mit standardmäßig unten angeordneter Taskleiste hat Priorität.

**Produktprinzip:** Ein Klick neben der Uhr öffnet die wichtigsten Windows-Regler; ein Klick außerhalb bringt den Desktop zurück – ohne die Anwendung zu beenden.

### Projektziele

- **Performant:** geringer CPU- und RAM-Verbrauch, schnelle Reaktionen.
- **Minimalistisch:** aufgeräumte Oberfläche mit klaren Bedienelementen.
- **Modern:** Dark Mode, Akzentfarben und dezente Animationen.
- **Modular:** neue Funktionen lassen sich ohne grundlegende Umbauten hinzufügen.
- **Praktisch:** möglichst viele Aufgaben direkt über das Tray-Popup erledigen.
- **Lokal:** Kernfunktionen ohne Konto, Cloud oder dauerhafte Internetverbindung.

## 2. Design

Für die erste Version verwenden wir **Dark Minimal**: ein kompaktes, rahmenloses Popup im Stil von Windows 11 – etwas moderner und farbiger, aber ohne unnötige visuelle Effekte.

### Designsystem (Ausgangswerte)

| Element | Vorgabe |
| --- | --- |
| Hintergrund | `#181A20` |
| Oberflächen / Karten | `#252832` |
| Trennlinien / Rahmen | `#343640` |
| Akzentfarbe | `#809CFF` |
| Primärtext | `#F3F4F8` |
| Schrift | Segoe UI / Systemschrift |
| Popup-Breite | ca. 380–420 px |
| Rundungen | ca. 10–14 px |
| Abstände | 8–16 px |
| Animationen | meist 120–180 ms |

**Animationen:** kurze Hover-Effekte, flüssige Slider-Rückmeldungen und animiertes Auf-/Zuklappen von Bereichen. Beim Popup zunächst **Inhalts-Fade + subtile Translation** statt vorausgesetzter OS-Fenstertransparenz; transparente/Layered-Fenster erst nach eigenem Prototyp. Animationen laufen nur bei Interaktionen. `DWMWA_WINDOW_CORNER_PREFERENCE` als Windows-11-Option testen: kein Garant für beliebige rahmenlose/Layered-Fenster.

### Zwei Darstellungen – in Etappen

1. **Minimal (Version 1):** kompakte Zeilen, Slider und aufklappbare Abschnitte für den schnellen Zugriff.
2. **Modular Dashboard (später):** frei konfigurierbare Widgets, verschiedene Größen und individuelle Anordnung.

Beide Designs greifen auf denselben Rust-Kern zurück. Langfristig soll der Nutzer die Darstellungen selbst wählen können. Das Design des Tray-Popups und des großen Hauptfensters kann unabhängig einstellbar sein.

## 3. Funktionen

### Version 1 – Kernfunktionen

#### Display Control

- Angeschlossene Monitore erkennen und auflisten.
- Helligkeit jedes unterstützten Monitors separat anpassen.
- Optional mehrere Monitore gemeinsam steuern.
- Nicht unterstützte Funktionen erkennbar deaktivieren.

**Technischer Hinweis:** Externe Monitore können – falls unterstützt – über DDC/CI gesteuert werden; bei internen Displays kommen je nach Gerät Windows-Schnittstellen wie WMI infrage. Nicht jede Monitor-/Anschlusskombination erlaubt echte Hardware-Helligkeitsregelung.

#### Audio Mixer

- Systemlautstärke einstellen und stummschalten.
- Einzelne Audio-Sessions bzw. Anwendungen erkennen.
- Lautstärke pro Anwendung ändern und stummschalten.
- Später: Audio-Ein- und Ausgabegeräte auswählen.

**Technischer Hinweis:** Eine Anwendung kann mehrere Audio-Sessions haben; die Oberfläche soll diese verständlich darstellen und bei Änderungen aktualisieren.

#### Time Tools

- Countdown-Timer.
- Stoppuhr.
- Start, Pause, Fortsetzen und Zurücksetzen.
- Benachrichtigung nach Ablauf, auch wenn das Popup geschlossen ist.

#### Grundfunktionen

- System-Tray-Icon und Klick zum Öffnen des Panels.
- Popup schließt bei `Esc` bzw. sinnvollerweise bei Fokusverlust.
- Positionierung an der Taskleiste, auch bei mehreren Monitoren und DPI-Skalierung.
- Einstellungen für Akzentfarbe, Verhalten, Start mit Windows und Animationen.
- Lokales Speichern der Einstellungen.

### Spätere Module

- **System Monitor:** CPU, GPU, RAM, Datenträger, Netzwerk, Temperaturen (soweit verfügbar).
- **Quick Actions:** Sperren, Ruhezustand, Energiesparprofil, Screenshots etc.
- **Productivity:** globale Hotkeys, Zwischenablage, Notizen, Fensteraktionen.
- **Profile:** Arbeit, Gaming, Nacht – mehrere Einstellungen mit einem Klick ändern.
- **Automatisierungen:** Aktionen nach Uhrzeit, App-Start oder Gerätewechsel.
- **Developer Tools:** UUID-Generator, JSON-Formatter, Port-Prüfung, Farbauswahl.
- **Modular Dashboard:** Widgets verschieben, skalieren und je nach Profil anordnen.

Diese Erweiterungen gehören **nicht** zum ersten Entwicklungsumfang.

## 4. Technischer Stack und Architekturentscheidungen

| Komponente | Entscheidung für v0.1 |
| --- | --- |
| Backend & Logik | Rust, kleine modulare Services |
| UI | Slint, zunächst Windows 11 x64 |
| Windows-Bindings | `windows`-Crate; Win32, COM, Core Audio, ggf. WMI |
| **System-Tray** | **Eigener Win32-Tray-Host mit `Shell_NotifyIconW`** |
| Tray-Popup | Slint-Fenster, Win32-Stile über sein `HWND` |
| Fensterhandle | Slint-Feature `raw-window-handle-06`, Backend-Support im Spike verifizieren |
| DPI | Manifest `PerMonitorV2`, Koordinaten sauber in physisch/logisch trennen |
| Konfiguration | Lokal, z. B. TOML oder JSON |
| Timer | Rust-Zeitverwaltung getrennt von der Slint-Animation |
| Benachrichtigungen | Technologieentscheidung in Phase 0; Windows App SDK gegen Aufwand testen |
| Installation | Früh planen: Autostart, Verknüpfung, Uninstall, Benachrichtigungen |

### ADR-001 – Tray-Technologie (entschieden)

**Wir verwenden für v0.1 einen eigenen kleinen Win32-Tray-Host.** Das bedeutet nicht, dass Slint keine native Tray-Funktion bietet: `SystemTrayIcon` ist **seit Slint 1.17 (24. Juni 2026)** verfügbar. Es liefert Icon, Menü und Linksklick-Callback. Für NexPanel benötigen wir jedoch insbesondere Kontrolle über die eindeutige Icon-ID, `Shell_NotifyIconGetRect`, `TaskbarCreated` und das gesamte Nachrichten-/Fokusverhalten. Bei Slint 1.18.1 wurde im November 2026 ein Problem mit der Wiederherstellung des Icons nach Explorer-Neustart gemeldet (Issue `slint-ui/slint#14839`, am Dokumentdatum offen). Deshalb bevorzugen wir die Win32-Kapselung. **Nicht gleichzeitig zwei Tray-Icons betreiben.**

**Implementierungsplan des Tray-Hosts:**

1. Ein eigenes verborgenes **Top-Level-Nachrichtenfenster** als Empfänger; kein reines `HWND_MESSAGE`, wenn Broadcasts wie `TaskbarCreated` benötigt werden.
2. Tray-Icon über `Shell_NotifyIconW(NIM_ADD)` registrieren, **persistenten GUID** und Callback-Message verwenden, nach Möglichkeit `NIM_SETVERSION` auf aktuelle Shell-Semantik setzen.
3. Linksklick als Event an den Slint-UI-Thread leiten. Rechtsklick mit Windows-Kontextmenü (Settings, Autostart, Beenden).
4. `RegisterWindowMessageW("TaskbarCreated")` behandeln; nach Explorer-Neustart Icon erneut registrieren und bei vorübergehend nicht verfügbarer Taskleiste begrenzt wiederholen.
5. Bei regulärem Beenden `NIM_DELETE` und Windows-Ressourcen aufräumen.
6. Den Win32-Nachrichtenloop des Hosts sauber in den Slint-Lebenszyklus integrieren. Falls eigener Windows-Thread verwendet wird, über `slint::invoke_from_event_loop` zur UI marshalen.

### ADR-002 – Popup-Verhalten (entschieden)

Das eigentliche Control Center ist **kein normales Hauptfenster**:

- `no-frame` in Slint **plus** `WS_EX_TOOLWINDOW` am tatsächlichen Win32-`HWND`; `WS_EX_APPWINDOW` darf nicht entgegenwirken. Stile vor der ersten Sichtbarkeit bzw. nach Verbergen setzen und testen.
- Kein Taskleisten-Button und kein eigener Alt-Tab-Eintrag für das Popup. Settings können separat geöffnet werden.
- Einmal beim Start erstellen und nach Möglichkeit initialisieren/vorwärmen, anschließend **anzeigen/verbergen statt ständig neu erzeugen**. `HWND` kann erst nach initialer Fenstererstellung verfügbar sein.
- **Zustandsautomat** mit `Closed / Opening / Open / Closing`: Klick auf Tray-Icon bei offenem Popup soll *schließen*, bei geschlossenem *öffnen*.
- **Toggle-Race vermeiden:** Fokusverlust (beim Klick auf das Tray-Icon) und anschließender Click-Callback können in ungünstiger Reihenfolge eintreffen. Fokus-Hide kurz verzögern/gegen einen aktiven Tray-Klick abgleichen; eng begrenzter Debounce (z. B. anfänglich 150–250 ms, als Testwert) statt blind jeden Folgeklick zu ignorieren. `Esc` soll sofort schließen.
- Aktivierung und Tastaturfokus explizit testen: Das Popup muss bedienbar bleiben, z. B. mit Tastatur, ohne direkt wieder zu verschwinden. Deshalb nicht pauschal `WS_EX_NOACTIVATE` setzen.

### ADR-003 – Positionierung und DPI (entschieden)

- Nach Tray-Klick das echte Icon-Rechteck via `Shell_NotifyIconGetRect` mit **unserem GUID** ermitteln.
- Falls nicht verfügbar (z. B. Overflow-Menü): Cursorposition/Monitorbezug als bevorzugten Kontext nutzen; `SHAppBarMessage(ABM_GETTASKBARPOS)` nur als begrenzten Fallback (nicht als exakte Iconposition und ggf. nicht für alle sekundären Taskleisten). Die Position immer in die nutzbare Monitorfläche begrenzen.
- **Manifest mit `PerMonitorV2`** von Anfang an einbinden. DPI-Wechsel, Monitorwechsel, 100/125/150/200 % Skalierung, veränderte Work Areas und mehrere Monitore testen.
- Zielort ist die Uhr-/Tray-Region unten rechts, aber das Fenster muss sich an den tatsächlichen Tray-Klick und den jeweiligen Monitor anpassen.

### ADR-004 – Single Instance und Lebenszyklus (entschieden)

- `CreateMutexW` mit stabil benanntem lokalen Mutex (`Local\NexPanel.Panel`) verhindert die zweite Instanz. Mutex über die Lebenszeit halten.
- Bei zweitem Programmstart kein zweites Tray-Icon erzeugen. Optional die erste Instanz über einen einfachen lokalen IPC-Kanal auffordern, das Popup zu öffnen; sonst die zweite Instanz kontrolliert beenden.
- Bei Autostart plus manuellem Start und bei mehrfachen Doppelklicks testen.
- Beim Explorer-Neustart läuft NexPanel weiter; sein Tray-Icon erscheint anschließend wieder.

### ADR-005 – Rendering und Animationen (Prototypentscheidung)

- Slint-Renderer (**Software / FemtoVG / Skia**) anhand **gemessener** Startzeit, RAM, CPU und Animationen vergleichen; keine pauschale Speicherzusage.
- Für v0.1 vorzugsweise opake Hintergrundfläche und animierter **UI-Inhalt** statt teurer animierter Fenster-Opazität. Transparenz/Blur nur ergänzen, wenn der Win11-Prototyp stabil ist.
- `DwmSetWindowAttribute(DWMWA_WINDOW_CORNER_PREFERENCE)` testen; bei eigenem Borderless-Fenster ggf. Slint-gezeichnete Ecken als Alternative.

### Architekturprinzip

```text
  Win32 Tray Host (Shell_NotifyIconW, GUID, TaskbarCreated)
       | Icon-Klick, Kontextmenü, Positionierung
       v
  Slint UI Thread (vorerzeugtes Popup, Animationen)
       | Ereignisse / Zustand / slint::invoke_from_event_loop
       v
  Rust Core (Commands, Config, State, Timer, Services)
       |                |                 |
   Display Worker   Audio Worker      Notification Adapter
       |                |                 |
   DDC/CI + WMI      Core Audio       Windows-Notifications
```

- **UI** rendert und verarbeitet Eingaben; die **Rust-Services** kapseln Windows-Zugriffe.
- Hardwarekommunikation und potenziell blockierende Abfragen **nicht** im UI-Thread durchführen.
- Alle Workerergebnisse und COM-Callbacks über den UI-Event-Loop zurückmelden; keine Slint-Objekte von Fremdthreads direkt verändern.
- Prozesse und Gerätelisten sind dynamisch: Disconnects, Fehler und Hotplug als normale Zustände behandeln.

### ADR-006 – Display Control und früher DDC-Spike

**Noch vor dem vollständigen Monitor-Modul** prüfen wir mit einem separaten Spike, ob mindestens ein echter externer Monitor regelbar ist. Schritte:

1. Logische Monitore über `EnumDisplayMonitors` / `HMONITOR` erfassen.
2. Über `GetNumberOfPhysicalMonitorsFromHMONITOR` und `GetPhysicalMonitorsFromHMONITOR` zu **physischen Monitorhandles** gelangen; korrekte Freigabe mit `DestroyPhysicalMonitor(s)` sicherstellen.
3. Helligkeits-VCP **0x10** bzw. unterstützte Windows-Monitor-Konfigurationsfunktionen lesen/schreiben; reale Dauer und Fehlermeldungen messen.
4. Internes Display separat via WMI-Klassen `WmiMonitorBrightness` / `WmiMonitorBrightnessMethods` testen, soweit vom Gerät/Displaytreiber unterstützt.
5. **Throttling/Coalescing pro Monitor** einsetzen: Drag-Bewegungen sofort visuell spiegeln, veraltete Hardwarebefehle zusammenfassen. Die genannten 40–50 ms pro DDC-Set sind nur Erfahrungswerte, **keine garantierte Latenz**.
6. Displays und Konfigurationsfähigkeiten pro Gerät/Verbindung erkennen; Monitore ohne Steuerungsmöglichkeit verständlich kennzeichnen.

### ADR-007 – Audio Control und COM

- Pro Audio-Endpoint `IAudioSessionManager2` / `IAudioSessionEnumerator` sowie Änderungsbenachrichtigungen (`IAudioSessionNotification`, `IMMNotificationClient`, je nach Zweck zusätzliche Session-Events) verwenden.
- COM-Apartmentmodell und Callback-Lebenszyklus festlegen; **keine UI-Updates direkt aus fremden COM-Threads**. Änderungen mittels `slint::invoke_from_event_loop` an das UI senden.
- Audio-Sessions können mehreren Prozessen zugeordnet sein oder kein auslesbares Prozessicon haben. Anzeigenamen und Icons best-effort ermitteln, Fallbacks vorsehen; Fehlzugriffe nicht als Absturz behandeln.
- Dynamische Erstellung/Entfernung von Sessions, Gerätewechsel und mehrere Sessions einer Anwendung testen.

### ADR-008 – Timer, Standby und Notifications (teilweise offen)

- Countdown anhand einer **Deadline**, nicht durch wiederholtes `--seconds` bei UI-Ticks verwalten. `Instant` für laufzeitinterne Messung/Animation verwenden; Countdown-Semantik bei Standby zusätzlich definieren.
- **Produktvorgabe:** Nach dem Aufwachen aus Standby soll ein abgelaufener Countdown als abgelaufen erkannt und genau **eine** Benachrichtigung ausgelöst werden. Der Prozess muss bei Resume bzw. der nächsten Ausführung erneut prüfen. Systemuhrverstellungen gesondert testen.
- Toast-Implementierung **in Phase 0 auswählen**: Der ältere WinRT-Weg benötigt typischerweise eine AUMID/Startmenü-Verknüpfung. Der aktuelle Windows App SDK `AppNotificationManager` kann auch unpaketierte Apps bedienen und richtet bei Registrierung COM-Aktivierung ein. Ob sich die SDK-Variante sinnvoll in eine kleine Rust-App integrieren lässt, wird im Spike entschieden.
- Den Installer **früh** berücksichtigen, auch falls Benachrichtigungen erst später geliefert werden; kein stillschweigend vorausgesetzter Paketstatus.

### ADR-009 – Slint-Lizenz (vor Implementierung klären)

Slint steht unter unterschiedlichen Lizenzen. Für ein proprietäres oder anders lizenziertes Desktop-Tool kommt die **Slint Royalty-free License 2.0** infrage, sofern deren Bedingungen erfüllt werden. Sie fordert nach aktuellem Lizenztext u. a. **entweder** das `AboutSlint`-Widget in einem erreichbaren „Über“-Dialog **oder** ein gut sichtbares Slint-Badge auf einer öffentlichen Produktseite. Alternativen: GPLv3 oder kostenpflichtige Slint-Lizenz. **Vor der ersten öffentlichen Distribution** Lizenzpfad und übrige Abhängigkeiten verbindlich dokumentieren; nicht erst in Release-Hektik prüfen.

**Vorläufige Empfehlung:** Royalty-free mit einem erreichbaren „Über NexPanel“-Bereich, der `AboutSlint` korrekt einbindet. Eine endgültige Entscheidung über die **Lizenz von NexPanel selbst** steht noch aus.

## 5. Performance-Ziele

Dies sind **Richtwerte für die Entwicklung**, keine zugesicherten Messwerte:

| Kennzahl | Angestrebter Wert |
| --- | --- |
| CPU im Leerlauf | nahe 0 % |
| RAM im Leerlauf | möglichst unter 50 MB |
| Sichtbares Popup nach Klick | möglichst unter 100 ms |
| Animationen | flüssig, idealerweise 60 FPS |
| Netzwerkbedarf | keiner für Kernfunktionen |

Wir messen diese Werte mit **Release-Builds** auf realer Hardware. Besonders Monitorerkennung, Polling und grafische Effekte können das Ergebnis beeinflussen. **Unter 50 MB RAM ist ein ambitioniertes Optimierungsziel, kein Abnahmekriterium ohne Renderer-Messung.** Bei >100 ms Popup-Latenz zuerst Fenster-Lebenszyklus, Synchronisation und Renderer profilen.

## 6. Entwicklungsplan mit Quality Gates

### Phase 0 – Windows-Integration und Machbarkeits-Spikes (Pflicht vor UI-Ausbau)

- [ ] Entscheidung für eigene `Shell_NotifyIconW`-Implementierung dokumentieren; Slint `SystemTrayIcon` als bekanntes Alternativkonzept erfassen.
- [ ] Eigener versteckter Top-Level-Host: Linksklick, Rechtsklick-Menü und Icon-GUID funktionieren.
- [ ] `TaskbarCreated` empfangen und Icon nach Explorer-Neustart automatisch wiederherstellen.
- [ ] Single-Instance via Named Mutex; zweiter Start erzeugt kein zweites Icon.
- [ ] Slint-Popup ohne Taskleisten-/Alt-Tab-Eintrag über HWND + `WS_EX_TOOLWINDOW`; Stile vor erstem Anzeigen prüfen.
- [ ] Popup einmal erzeugen, ohne Neuerstellung anzeigen/verbergen; Fensterhandle über Raw-Window-Handle verfügbar.
- [ ] Positionierung: Icon-Rechteck, Overflow-Fallback, zweiter Monitor, Per-Monitor-DPI-v2.
- [ ] Toggle-Race unter schnellem Tray-Klicken und Fokusverlust reproduzieren und beseitigen.
- [ ] Renderer mit Startzeit, Ruhe-RAM, CPU und Popup-Latenz vergleichen.
- [ ] **DDC/CI-Spike** mit externem Monitor: Mapping, VCP-Lese-/Schreibzugriff, gemessene Dauer.
- [ ] Notification-API-Integration/Installer-Abhängigkeit evaluieren.
- [ ] Slint-Lizenzweg und Attribution für NexPanel dokumentieren.

**Quality Gate 0:** 50 wiederholte Öffnen-/Schließen-Vorgänge ohne Race; kein Taskleisten-Button; korrektes Verhalten bei 100/150/200 % und zwei Bildschirmen; Explorer-Neustart überlebt; zweite Instanz blockiert; Monitor-/Benachrichtigungsspikes sind dokumentiert. Alle Ergebnisse per Release-Build messen.

### Phase 1 – Minimales Control Center

- [ ] Rust-Projekt, Slint-Popup, Theme und wiederverwendbare UI-Controls erstellen.
- [ ] Tray-Host, Popup-Zustandsmaschine und Positionierungsmodul sauber zusammenführen.
- [ ] Minimal-Design mit Farben, Segoe UI, Icons, Slider, Modulen und sparsamen Animationen.
- [ ] Einstellungen lokal speichern: Akzentfarbe, Animationen, Startverhalten.
- [ ] Testdaten und Fehler-/Ladezustände in Display-/Audio-Widgets anzeigen.
- [ ] Autostart und sichere Beendigung integrieren.

**Quality Gate 1:** Alltagstaugliches Tray-Popup, das ohne echte Hardwareintegration zuverlässig bedienbar ist.

### Phase 2 – Monitore

- [ ] DDC-Spike in wiederverwendbaren Display-Service überführen.
- [ ] Unterstützte physische und interne Monitore erkennen.
- [ ] Hardware-Helligkeit lesen/setzen; zusammengefasste Regler-Events implementieren.
- [ ] Gekoppelte Regelung, Hotplugging und klare Fehler-/Unsupported-Zustände ergänzen.

**Quality Gate 2:** Unterstützte Monitore lassen sich ohne UI-Blockade steuern; Grenzfälle reproduzierbar getestet.

### Phase 3 – Audio

- [ ] Audio-Endpoints und Sessions über Core Audio erkennen.
- [ ] Master-Lautstärke, per-Session-Lautstärke und Mute implementieren.
- [ ] COM-Callback-Lebensdauer und UI-Thread-Marshalling absichern.
- [ ] Prozessnamen/-icons, neue und beendete Sessions sowie Gerätewechsel behandeln.

**Quality Gate 3:** Audio-Control funktioniert nach Start/Stop von Apps und Gerätewechsel zuverlässig.

### Phase 4 – Timer, Benachrichtigungen und erster Release

- [ ] Timer/Stoppuhr sowie Deadline-/Standby-Verhalten umsetzen.
- [ ] Die in Phase 0 ausgewählte Notification-Integration fertigstellen.
- [ ] Installer, Autostart, Uninstall, Produkticon und rechtliche Hinweise liefern.
- [ ] Release-Profiling, Accessibility, Crash-/Recovery-Tests und Dokumentation durchführen.

**Quality Gate 4:** Installierbare v0.1 mit Tray, Display, Audio und Timer, stabil bei Explorer-Neustart und Standby.

### Kompakte Testmatrix

| Szenario | Erwartung |
| --- | --- |
| Icon normal / hinter `^` | Öffnung am richtigen Ort oder im nachvollziehbaren Fallback |
| Linksklick 2× schnell | Popup endet zuverlässig geschlossen |
| Tray-Klick während Fokusverlust | Kein unbeabsichtigtes Wiederöffnen |
| `Esc` / Außenklick | Popup schließt, Prozess und Icon bleiben |
| Explorer beenden und starten | Icon registriert sich erneut; Prozess läuft weiter |
| Autostart + manueller Start | Genau eine Instanz und ein Icon |
| Zwei Monitore, verschiedene DPI | Popup auf richtigem Monitor und vollständig sichtbar |
| DDC/CI sehr langsam / nicht unterstützt | UI bleibt reaktionsfähig, sauberer Fehlerzustand |
| Audio-App startet oder beendet sich | Liste aktualisiert sich ohne Fehler |
| Standby länger als Countdown | Timer nach Resume abgelaufen, eine Benachrichtigung |

## 7. Risiken, Prioritäten und offene Entscheidungen

| ID | Risiko | Auswirkung | Status / Umgang |
| --- | --- | --- | --- |
| R1 | Tray-Lebenszyklus, Explorer-Neustart | Kritisch | Eigener Win32-Host, `TaskbarCreated`, Phase-0-Test |
| R2 | Popup-Race und Fensterstile | Kritisch | Zustandsautomat, Debounce/Fokuskoordination, `WS_EX_TOOLWINDOW` |
| R3 | Positionierung und Mixed DPI | Hoch | GUID-Rect, Cursor-/Work-Area-Fallback, `PerMonitorV2` |
| R4 | DDC/CI-Hardwarekompatibilität und Latenz | Hoch | Spike in Phase 0, Capability-Erkennung, Coalescing |
| R5 | COM-Threading bei Audio | Mittel/Hoch | Separater Worker, Callback-Marshalling |
| R6 | RAM-Ziel / Fade und Renderer | Mittel | Renderervarianten messen, Transparenz optional |
| R7 | Timer nach Standby / Systemzeitänderung | Mittel | Deadline-/Resume-Semantik und Tests |
| R8 | Benachrichtigungen und Packaging | Mittel | API-/Installer-Spike Phase 0 |
| R9 | Slint-Lizenz und NexPanel-Lizenz | Hoch vor Vertrieb | Lizenzweg vor Entwicklung bestätigen |

**Noch offen und erst anhand eines Spikes zu entscheiden:**

- Welcher Renderer auf realer Windows-11-Hardware unser Qualitätsziel am besten erfüllt.
- Welche Benachrichtigungs-API in Rust mit möglichst geringem Packaging- und RAM-Aufwand praktikabel ist.
- Ob der Timer bei manueller Systemzeitverschiebung absolute Kalenderzeit oder unbeeinflusste Countdown-Dauer priorisiert; das Standby-Verhalten ist bereits definiert.
- Welche endgültige Lizenz NexPanel selbst erhält (proprietär, permissives Open Source, GPL-kompatibel etc.).

### Quellenstand zur Technik (16. November 2026)

- Slint `SystemTrayIcon`: https://docs.slint.dev/latest/docs/slint/reference/window/systemtrayicon/
- Slint 1.17 Release (Tray eingeführt): https://slint.dev/blog/slint-1.17-released
- Slint Explorer-Issue #14839: https://github.com/slint-ui/slint/issues/14839
- Slint Fensterhandle: https://docs.slint.dev/latest/docs/rust/slint/struct.Window
- Windows Taskleistenfenster: https://learn.microsoft.com/en-us/windows/win32/shell/taskbar
- `Shell_NotifyIconGetRect`: https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shell_notifyicongetrect
- DPI-Manifest: https://learn.microsoft.com/en-us/windows/win32/sbscs/application-manifests
- DWM abgerundete Ecken: https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/ne-dwmapi-dwm_window_corner_preference
- Physische Monitorhandles: https://learn.microsoft.com/en-us/windows/win32/api/physicalmonitorenumerationapi/nf-physicalmonitorenumerationapi-getphysicalmonitorsfromhmonitor
- Windows Notifications: https://learn.microsoft.com/en-us/windows/apps/develop/notifications/
- Alte AUMID-/Shortcut-Anforderungen: https://learn.microsoft.com/en-us/windows/win32/shell/enable-desktop-toast-with-appusermodelid
- Slint-Lizenzbedingungen: https://slint.dev/terms-and-conditions

## 8. Leitprinzipien

**NexPanel soll sich nicht wie ein großes Hintergrundprogramm anfühlen, sondern wie eine natürliche Erweiterung von Windows.**

- Wenige Klicks statt verschachtelter Menüs.
- Schneller Start statt großer Framework-Last.
- Eine klare Oberfläche statt Feature-Überladung.
- Erweiterbarkeit ohne Verlust an Performance.

---

*Spezifikation v0.2. Architekturentscheidungen wurden nach technischer Risikoanalyse konkretisiert. Implementierungsdetails und Performanceziele werden anhand von Phase-0-Prototypen validiert.*
