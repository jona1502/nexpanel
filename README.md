# NexPanel

Ein lokales Windows-11-x64-Control-Center mit Rust und Slint: Tray-Popup, echte Displayhelligkeit auf unterstützter Hardware, System- und App-Lautstärke, Countdown, Stoppuhr und persistente Einstellungen. Die deutsche Produktseite ist eine statische Astro-Website.

Version 0.2 (Vorabversion): Tray-Popup mit Display- und Audiosteuerung (inkl. Pegelanzeige pro App), Timer, Systemmonitor, NexSearch (Alt+Leertaste: Programme, Rechner, Befehle) und Updater in den Einstellungen. Releases sind nicht signiert und es ist keine Projektlizenz gewählt. Performance-Ziele sind ungemessen; Hardware- und native Bedienprüfungen stehen teilweise aus. [Teststand und Grenzen](docs/testing.md).

```powershell
git clone https://github.com/jona1502/nexpanel.git
cd nexpanel
# Nach Einrichtung von Rust/MSVC gemäß docs/setup.md:
cargo run -p nexpanel
# Release-ZIP samt Installationsskripten und SHA256:
./scripts/package.ps1
```

Das ZIP unter `dist/nexpanel-0.2.0-windows-x64.zip` entpacken und dessen `install.ps1` ausführen. Installation nach `%LOCALAPPDATA%\NexPanel\bin`, optional mit Startmenü-Verknüpfung, ohne automatischen App-Start. Autostart wird nur in den App-Einstellungen aktiviert. Einstellungen unter `%LOCALAPPDATA%\NexPanel\settings.json` bleiben bei Deinstallation erhalten.

Tray-Klick öffnet oder verbirgt das Panel; Esc und Schließen blenden aus. Zum Beenden das Tray-Menü verwenden. `--background` startet verborgen, `--diagnostics` liest reale Geräte ohne Hardwareänderungen, `--capture-preview <Verzeichnis>` erzeugt fünf echte UI-Aufnahmen ohne Einstellungs- oder Hardwareänderungen und beendet die App. Weitere Optionen: `--help`.

Website lokal:

```powershell
. ./scripts/use-node.ps1
cd apps/website
npm.cmd ci
npm.cmd run dev
```

[Lizenzen](docs/licensing.md) · [Release-Prozess](docs/releasing.md) · [Vollständiges Setup](docs/setup.md) · [Produktumfang](NexPanel.md) · [v0.1-Entwurf](docs/superpowers/specs/2026-10-31-v01-design.md) · [Quellcode](https://github.com/jona1502/nexpanel)
