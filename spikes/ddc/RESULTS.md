# DDC/CI Spike – Ergebnisse (ADR-006)

Gemessen am 2026-11-16, Windows 11, `cargo run -p ddc-spike`.

## Hardware

3 externe Monitore, alle als "Generic PnP Monitor" gemeldet (Beschreibung ist nutzlos zur Identifikation; Modell steht nur im Capabilities-String):

| idx | GDI-Name | Modell (caps) | Auflösung | MCCS |
|---|---|---|---|---|
| 0 | DISPLAY1 (primär) | CQ32G4VE | 2560x1440 | 2.2 |
| 1 | DISPLAY2 | L24BWBCK-KUDP | 1920x1080 | 2.1 |
| 2 | DISPLAY3 | C27G1 | 1920x1080 | 2.2 |

Kein interner Display (Desktop).

## Welche API funktioniert

- Hochlevel `GetMonitorBrightness` und niedrig `GetVCPFeatureAndVCPFeatureReply(0x10)`: beide funktionieren auf allen 3 Monitoren, liefern identische Werte (min 0, max 100).
- `SetVCPFeature(0x10)` funktioniert auf allen 3, Readback stimmt.
- WMI `root\WMI\WmiMonitorBrightness`: Fehler `0x8004100C` (WBEM_E_NOT_SUPPORTED) – erwartet auf Desktop ohne internes Panel. Wird sauber als "not supported" gemeldet. `WmiSetBrightness` im Spike nicht implementiert (braucht ExecMethod mit Objektpfad + In-Parameter; für Laptops später).

## Latenzen

| Operation | Messung |
|---|---|
| Brightness lesen (hoch/niedrig) | 61–65 ms |
| `SetVCPFeature` (bench n=10, je Monitor) | min 60.6 / avg ~61.9 / max 63.1 ms |
| Caps-String lesen | Mon 0: 5.2 s, Mon 1: 1.1 s, Mon 2: 1.7 s |
| WMI-Query (nicht unterstützt) | ~35 ms |

Beobachtung: jeder DDC-Aufruf blockiert fast konstant ~60 ms (dxva2 erzwingt Wartezeit für das DDC/CI-Protokoll), unabhängig vom Monitor. Der Bench hat jeweils auf den Originalwert zurückgesetzt (verifiziert). Die Caps-Zeit steckt fast komplett in `GetCapabilitiesStringLength`.

## Folgerungen für Coalescing/Throttling

- Alle DDC-Aufrufe in einen Worker-Thread, nie im UI-Thread (60 ms pro Call, Caps bis Sekunden).
- Pro Monitor max. 1 Set in flight; Latest-wins-Coalescing: während ein Set läuft, nur den neuesten Zielwert merken, danach direkt senden.
- Praktisch ergibt das ~16 Sets/s je Monitor. Empfohlenes Mindestintervall pro Monitor: 60–80 ms (faktisch durch Call-Dauer erzwungen), zusätzlich Trailing-Send des Endwerts beim Loslassen des Sliders.
- Monitore parallel in separaten Threads/Tasks ansteuern ist möglich (unterschiedliche Busse), nicht im Spike getestet – bei sequentieller Abarbeitung von 3 Monitoren wären es ~180 ms pro Slider-Tick bei "alle gleichzeitig".
- Capabilities nur einmal asynchron beim Start/Hotplug lesen und cachen (1–5 s), nicht beim Öffnen des Panels blockierend.
- Brightness-Wert beim Start per Lesen holen (~60 ms/Monitor), nicht pollen.

## Offene Punkte

- Stabile Monitor-Identität: Handle/Index ändert sich bei Hotplug; Zuordnung über GDI-Gerätename + EDID/Modell (z. B. via `EnumDisplayDevices` mit `EDD_GET_DEVICE_INTERFACE_NAME`) noch zu klären.
- Verhalten bei Standby/Wake, Eingangswechsel, Monitor-OSD-Konflikten nicht getestet.
- Paralleles Setzen auf mehreren Monitoren nicht gemessen.
- Fehlerfälle (Monitor ohne DDC/CI, per Dock/DisplayLink) mangels Hardware nicht beobachtet; Fehler werden mit `GetLastError` ausgegeben und überspringen den Monitor.
- Laptop-Pfad (WMI lesen/setzen) ungetestet; `WmiSetBrightness` fehlt.
- Dauerhaftes Schreiben von VCP 0x10 kann den Monitor-EEPROM belasten (Herstellerangaben unklar) – Coalescing reduziert das.

## Nachtrag: Capabilities-Flag ist kein Kriterium (2026-11-16)

`ddc-spike list` gibt jetzt zusätzlich `GetMonitorCapabilities` aus (nur lesend). NexPanel hat `caps & MC_CAPS_BRIGHTNESS (0x2)` als Voraussetzung geprüft. Messung per `nexpanel --diagnostics` (Debug-Log):

| Monitor | GetMonitorCapabilities | GetMonitorBrightness |
|---|---|---|
| 27G1G4 | ok=false, caps=0x0, Fehler 0xC0262580, ~1,9 s | min=0 cur=50 max=100 |
| CQ32G4 | ok=false, caps=0x0, Fehler 0xC0262580, ~5,3 s | min=0 cur=60 max=100 |
| MD20831 | ok=true, caps=0x1c17 (Flag 0x2 gesetzt) | min=0 cur=50 max=100 |

Fazit: Der Aufruf scheitert bei zwei Monitoren, obwohl Helligkeit lesbar (und per VCP 0x10 steuerbar) ist; er ist langsam und darf kein Gate sein. Die Quelle (`\.\DISPLAYn`) wurde immer in `EnumDisplayMonitors` gefunden, je 1 physischer Monitor. Erkennung daher: `GetMonitorBrightness`, sonst VCP 0x10.
