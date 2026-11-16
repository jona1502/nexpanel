# System-Monitor-Tab (CPU / RAM / GPU / VRAM)

Stand: 16.11.2026, Branch `feat/system-monitor` (gestapelt auf #14).

## Umfang
Neuer Tab „System“ mit vier Zeilen: CPU %, RAM (GB + %), GPU %, VRAM (GB). Nur lesend, ohne
Adminrechte, herstellerneutral, ohne Treiber. Nicht im Umfang: Temperaturen (braucht
Hersteller-APIs oder Admin), Datenträger, Netzwerk, Uptime, Tray-Tooltip mit Werten
(`tray.rs` wird parallel geändert; später möglich).

## Datenquellen
- CPU gesamt: `GetSystemTimes`-Deltas. Busy = (kernel + user − idle) / (kernel + user). Kernel enthält Idle.
  Erste Messung nach ca. 250 ms; Delta 0 oder Rücklauf ergibt 0 %.
- RAM: `GlobalMemoryStatusEx` (used = total − avail).
- GPU: PDH `\GPU Engine(*)\Utilization Percentage`, Wildcard per `PdhExpandWildCardPathW` aufgelöst,
  Werte per `PdhGetFormattedCounterArray` (DOUBLE). Instanzname
  `pid_N_luid_0xH_0xL_phys_P_eng_E_engtype_T` → Adapter (luid+phys) und Engine-Typ. Aggregation:
  Summe je (Adapter, Typ), Maximum über Typen je Adapter, Kopfwert = ausgelasteter Adapter (max).
  Wildcards werden alle 10 Samples neu aufgelöst (Prozess-Engines kommen und gehen).
  Erster Sample liefert `PDH_CALC_NEGATIVE_DENOMINATOR`; erst ab zwei Collections gültig.
- VRAM: PDH `\GPU Adapter Memory(*)\Dedicated Usage` (Bytes, je Adapter-luid). Gesamt: DXGI
  `DXGI_ADAPTER_DESC1.DedicatedVideoMemory`, Zuordnung über luid. Fehlt Gesamt, nur „genutzt“.
  Der Adapter für VRAM ist derselbe wie für die GPU-Last (Kopf-Adapter); Name aus DXGI-Beschreibung.

## Worker- und Sichtbarkeitsmodell
`services/system.rs`, Thread `nexpanel-system` über `logging::spawn_worker` + `CrashFlag`.
Befehle `SetActive(bool)` und `Shutdown`. `SetActive(true)` nur, wenn Panel sichtbar UND Tab „System“
aktiv. Inaktiv: `rx.recv()` ohne Timeout, PDH-Query geschlossen → 0 Wakeups. Aktiv: Prime, nach 250 ms
erster Wert, danach 1 s Takt. Snapshots nur bei Änderung (CPU/GPU auf ganze %, RAM auf 0,1 GB gerundet).
PDH-Query und Handles werden beim Deaktivieren/Drop geschlossen. Kein COM nötig (DXGI-Factory einmal pro Aktivierung).

## UI
Zeilen `Label | dünner Balken (3 px) | Wert` im kompakten Audio-Stil. Balken: Spur #343640, Füllung
Akzent, ab 80 % bernstein, ab 95 % rot. GPU-Name als 11-px-Zeile in Grau über der GPU-Zeile.
Aktualisierung über den vorhandenen Tick (200 ms sichtbar), kein neuer Timer.
Tab-Reihenfolge (seit feat/tab-order-and-memory): Audio 0, Display 1, System 2, Zeit 3, Einstellungen 4 (Konstanten + Test).

## Fehler / nicht unterstützt
PDH-GPU-Zähler fehlen (VM, alter Treiber): „GPU: nicht verfügbar“ gedimmt, `log::warn!`, kein Dialog.
CPU/RAM laufen weiter. Worker-Panic: Statuszeile „System-Dienst abgestürzt – siehe Log“.

## Performance-Budget
Verborgen oder anderer Tab: Worker blockiert, 0 Wakeups. Aktiv: ≤ 1 Wakeup/s; ein Sample sind zwei
Syscalls plus ein PDH-Collect, erwartet deutlich < 0,5 % CPU. Nicht gemessen (kein GUI-Lauf möglich).

## Tests
Rein, tabellengetrieben: CPU-Delta (Rücklauf, Division durch 0), RAM-Formatierung, Instanznamen-Parser,
GPU-Aggregation (Summe/Max/Adapterwahl), Balkenfarben-Schwellen, Tab-Konstanten gegen `main.slint`,
Änderungsregel für Snapshots. OS-Aufrufe hinter einem Trait, damit die Aggregation testbar bleibt.
Manuelle Vergleiche mit Task-Manager in `docs/testing.md`.
