# Backlog nach v0.1-Review (16.11.2026)

Ergebnis eines Code-Reviews von v0.1 (Stand `8c88fe5`) gegen die
[v0.2-Spezifikation](superpowers/specs/2026-11-16-v02-windows-integration.md).
Build, 35 Tests, Clippy und fmt waren sauber. Die Einträge sind nach Priorität sortiert.
Erledigte Punkte abhaken und auf den PR verweisen.

## P0 – Bugs

- [x] **Tray-Icon nach Explorer-Neustart:** Slints `SystemTrayIcon` (1.18.1) nutzt ein
  `HWND_MESSAGE`-Fenster, das den Broadcast `TaskbarCreated` nicht empfängt. Das Icon
  bleibt danach verschwunden. Fix: eigener Win32-Tray-Host gemäß ADR-001.
- [ ] **Slider-Drag:** Audio sendet jede Sekunde einen Snapshot, Display nach jedem Schreibvorgang.
  `main.rs` setzt dann jedes Mal ein neues `VecModel`, wodurch ein laufender Drag vermutlich
  abbricht. Fix: nur bei Änderung senden und das Modell inkrementell aktualisieren
  (`set_row_data`). Vorher manuell bestätigen.
- [ ] **Stille Worker-Panics:** Es gibt keinen Panic-Hook und kein Logging. Stirbt der Display- oder
  Audio-Worker, bleibt die UI auf „wird erkannt …“ stehen.

## P1 – Fehlende Spec-Anforderungen

- [x] `WS_EX_TOOLWINDOW` am Popup-HWND setzen, damit es keinen Alt-Tab-Eintrag gibt (ADR-002).
- [x] Manifest mit `PerMonitorV2`, Common Controls v6 und supportedOS einbinden (ADR-003).
- [x] Popup per `Shell_NotifyIconGetRect` positionieren, mit Fallback auf Cursor bzw. `ABM_GETTASKBARPOS`.
  Beim Timer-Ablauf und bei Tastaturaktivierung nicht am Cursor öffnen.
- [x] `NIM_SETVERSION` v4 setzen und `NIN_SELECT`/`NIN_KEYSELECT` verarbeiten. Persistente GUID je Build-Variante,
  da die GUID bei unsignierten Exes an den Pfad gebunden ist.
- [ ] Toggle-Race über eine Zustandsmaschine statt der 250-ms-Heuristik lösen; Press-and-Hold abdecken.
- [ ] Audio über Events statt Polling: `IMMNotificationClient`, `IAudioSessionNotification`
  (vorher `GetSessionEnumerator` aufrufen), `IAudioEndpointVolumeCallback`. Kein Voll-Refresh pro Write.
- [ ] Timer: robust gegen Sprünge der Systemuhr machen und `WM_POWERBROADCAST` als Resume-Trigger nutzen.
  Echte Benachrichtigung statt erzwungenem Popup, das den Fokus stiehlt (ADR-008).
- [ ] Kontextmenü um Einstellungen und Autostart erweitern.
- [ ] Logging in eine Datei unter `%LOCALAPPDATA%\NexPanel`.
- [ ] Slint-Lizenzpfad festlegen, „Über“-Bereich mit `AboutSlint` (ADR-009).

## P2 – Verbesserungen

- [ ] Position vor `show()` setzen, um ein Flackern beim ersten Frame zu vermeiden.
- [ ] Idle-Wakeups reduzieren: Der 200-ms-Tick läuft auch bei verborgenem Panel. Mutexname ohne Version.
- [ ] Hotplug über `WM_DISPLAYCHANGE` statt 4-s-Polling erkennen.
- [ ] `StartupApproved` auswerten, `deny_unknown_fields` lockern, vor dem Reset ein Backup der korrupten Datei anlegen.
- [ ] Esc soll offene ComboBox-Popups schließen, nicht das ganze Panel.
- [ ] Tray-Icon in mehreren Größen bereitstellen (16/20/24/32).
- [ ] Display- und Audio-Worker beim Beenden joinen.
- [ ] Integrationstests ergänzen.

## Messdaten

DDC/CI auf drei externen Monitoren: siehe [spikes/ddc/RESULTS.md](../spikes/ddc/RESULTS.md).
Lesen und Setzen dauern je ca. 62 ms, der Capabilities-String 1–5 s.
