# NexPanel

> **Everything in one place.**
> Ein schnelles, minimalistisches und erweiterbares Windows Control Center – entwickelt mit **Rust**.

**Projektstatus:** v0.1 lokal implementiert und geprüft; Gesamtreview, Push und praktische Hardware-/Native-Tests offen
**Plattform:** Windows 11 (zunächst x64)
**Sprache:** Rust
**UI:** Slint
**Aktueller Stand:** Version 0.1 – Minimal Control Center (lokal, unveröffentlicht)

---

Lokale Nachweise und ausdrücklich offene Prüfungen: [docs/testing.md](docs/testing.md). Abgehakte Funktionspunkte bedeuten implementiert, nicht auf jeder Hardware praktisch validiert. Keine Lizenzwahl oder Veröffentlichung erfolgt.

## 1. Vision

**NexPanel** vereint häufig benötigte Windows-Funktionen in einer einzigen, leichtgewichtigen Anwendung. Statt mehrere Tools für Monitorhelligkeit, Lautstärke, Timer und Systeminformationen zu öffnen, lassen sich diese Funktionen über ein zentrales Panel direkt aus dem **System-Tray** bedienen.

Der Name setzt sich sinngemäß aus **„Nex“** (angelehnt an *Next* und *Nexus*) und **„Panel“** (Bedienfeld) zusammen: ein zentrales Control Panel der nächsten Generation.

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

**Animationen:** sanftes Ein- und Ausblenden des Popups, kurze Hover-Effekte, flüssige Slider-Rückmeldungen und animiertes Auf-/Zuklappen von Bereichen. Animationen sollen nur bei tatsächlichen Interaktionen laufen.

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

## 4. Technischer Stack

| Komponente | Technologie / Ansatz |
| --- | --- |
| Backend & Geschäftslogik | Rust |
| Benutzeroberfläche | Slint |
| Website | Astro |
| Windows-Integration | Win32, COM, Core Audio, ggf. WMI |
| System-Tray | Slint-Integration und bei Bedarf Win32 |
| Animationen | Slint |
| Konfiguration | Lokale JSON-Datei |
| Entwicklungsziel | Windows 11 x64 |

### Architekturprinzip

```text
┌─────────────────────────────────────┐
│             Slint UI                │
│  Tray-Popup · Controls · Settings   │
└─────────────────┬───────────────────┘
                  │ Events / State
┌─────────────────▼───────────────────┐
│             Rust Core               │
│ Config · Commands · Profiles · Bus  │
└───────┬────────────┬──────────┬─────┘
        │            │          │
   Display       Audio      Timer / System
   Service       Service       Services
        │            │          │
   Windows-APIs / angeschlossene Hardware
```

- Die **UI** ist für Darstellung und Eingaben zuständig.
- Der **Rust Core** verwaltet Zustand, Einstellungen und Befehle.
- **Services** kapseln jeweils eine Windows-Funktion und können separat getestet werden.
- Langsame Hardware- oder Systemzugriffe laufen **nicht im UI-Thread**.
- Ereignisbasierte Aktualisierungen werden bevorzugt; Polling wird begrenzt.
- Häufige Slider-Änderungen werden bei Bedarf zusammengefasst, damit Monitor-Hardware nicht mit Anfragen überlastet wird.

## 5. Performance-Ziele

Dies sind **Richtwerte für die Entwicklung**, keine zugesicherten Messwerte:

| Kennzahl | Angestrebter Wert |
| --- | --- |
| CPU im Leerlauf | nahe 0 % |
| RAM im Leerlauf | möglichst unter 50 MB |
| Sichtbares Popup nach Klick | möglichst unter 100 ms |
| Animationen | flüssig, idealerweise 60 FPS |
| Netzwerkbedarf | keiner für Kernfunktionen |

Wir messen diese Werte mit **Release-Builds** auf realer Hardware. Besonders Monitorerkennung, Polling und grafische Effekte können das Ergebnis beeinflussen.

## 6. Entwicklungsplan

### Phase 1 – Fundament

- [x] Rust-Projekt mit Slint aufsetzen.
- [x] Designsystem: Farben, Typografie, Icons, Slider und Buttons.
- [x] Tray-Icon und rahmenloses Popup implementieren.
- [ ] Animationen, native Fokusübergänge und Multi-DPI-Positionierung praktisch testen (Implementierung vorhanden).
- [x] Grundlegende Einstellungen lokal speichern.
- [x] UI mit echten Gerätewerten und Renderer-Aufnahmen prüfen.

**Definition of Done:** Das Popup lässt sich zuverlässig öffnen und schließen, funktioniert auf mehreren Displays und bleibt im Leerlauf ressourcenschonend.

### Phase 2 – Monitore

- [x] Monitore erkennen.
- [x] DDC/CI- und weitere geeignete Windows-Zugriffe evaluieren.
- [x] Helligkeit pro unterstütztem Gerät steuern.
- [x] Gekoppelte Regelung ergänzen.
- [x] Fehlerfälle, nicht unterstützte Displays und Hot-Plugging berücksichtigen.

**Definition of Done:** Unterstützte Monitore lassen sich ohne blockierende Oberfläche zuverlässig regeln.

### Phase 3 – Audio

- [x] Systemlautstärke auslesen und ändern.
- [x] Audio-Sessions erkennen und aktualisieren.
- [x] App-Regler und Mute-Funktion integrieren.
- [x] Geräte- und Session-Wechsel robust behandeln.

**Definition of Done:** Die Lautstärke laufender Anwendungen lässt sich getrennt verändern.

### Phase 4 – Timer & erster Release

- [x] Timer und Stoppuhr hinzufügen.
- [x] Ablaufbenachrichtigungen integrieren.
- [x] Konfiguration, Autostart und Bedienbarkeit vervollständigen.
- [ ] Release-Builds messen und nach Messergebnis optimieren.
- [x] Benutzerlokale PowerShell-Distribution und Hinweise zu offener Lizenz-/Veröffentlichungsprüfung vorbereiten.

**Definition of Done:** Eine stabile, installierbare erste Version mit Tray-Popup, Display-, Audio- und Zeitfunktionen.

## 7. Risiken und wichtige Entscheidungen

1. **Monitor-Kompatibilität:** DDC/CI funktioniert nicht mit jedem Gerät oder Adapter. Fähigkeiten müssen pro Monitor erkannt werden; ein Software-Overlay ist kein gleichwertiger Ersatz für Hardware-Helligkeit.
2. **Tray-Fenster:** Positionierung, Fokus, Taskleistenlage, mehrere Monitore und DPI-Skalierung verlangen gezielte Windows-Integration und Tests.
3. **Audio-Sessions:** Sessions können jederzeit starten, verschwinden oder mehrfach pro Prozess existieren.
4. **UI-Reaktionsfähigkeit:** Systemabfragen dürfen Animationen und Eingaben nicht blockieren.
5. **Slint-Lizenz:** Vor Veröffentlichung die dann gültigen Lizenz- und Attribution-Bedingungen prüfen.
6. **Berechtigungen:** Im Normalbetrieb möglichst ohne Administratorrechte auskommen.

## 8. Leitprinzipien

**NexPanel soll sich nicht wie ein großes Hintergrundprogramm anfühlen, sondern wie eine natürliche Erweiterung von Windows.**

- Wenige Klicks statt verschachtelter Menüs.
- Schneller Start statt großer Framework-Last.
- Eine klare Oberfläche statt Feature-Überladung.
- Erweiterbarkeit ohne Verlust an Performance.

---

*Dieses Dokument ist die erste Projektspezifikation. Implementierungsdetails und Zielwerte können nach den ersten Prototypen angepasst werden.*
