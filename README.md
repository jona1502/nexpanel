# NexPanel

Windows-Anwendung mit Rust und Slint sowie eine statische Astro-Website in einem Repository.

Das Grundgerüst zeigt ein NexPanel-Setup-Fenster und eine neutrale deutsche Setup-Seite. Tray, Monitorsteuerung, Audio, Timer, Installer und die gestaltete öffentliche Website folgen später.

Verwendet werden Rust/Cargo 1.96.1 (Windows MSVC), Slint/slint-build 1.18.1, Node 24.22.2 mit npm 11.19.1, Astro 7.4.4, @astrojs/check 0.9.12 und TypeScript 6.0.3. Beide Lockfiles sind versioniert.

Nach der vollständigen Installation in der Setup-Anleitung im Repository ausführen:

```powershell
# Desktop starten
cargo run -p nexpanel

# Website starten
. ./scripts/use-node.ps1
Set-Location apps/website
npm.cmd ci
npm.cmd run dev
```

Am 2026-10-31 waren Rust-Formatierung, Check, Clippy und Build sowie npm ci, Astro-Check und statischer Build lokal erfolgreich. Das native Fenster startete und schloss sauber; die gebaute Website lieferte HTTP 200 mit erwartetem Titel, Sprache und Viewport. Visuelle Darstellung, DPI-Skalierung und Performance sind noch ungeprüft. Der [CI-Workflow](.github/workflows/ci.yml) bildet die Build-Prüfungen ab; ohne Push liegt kein Remote-CI-Ergebnis vor.

Installation und Aktivierung: [docs/setup.md](docs/setup.md). Architektur und freigegebener Umfang: [Bootstrap-Entwurf](docs/superpowers/specs/2026-10-31-project-bootstrap-design.md). Produktanforderungen: [NexPanel.md](NexPanel.md).
