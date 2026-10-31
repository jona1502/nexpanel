# Lokales Setup

Stand: 2026-10-31. Desktop und Website sind als lokal geprüfte Grundgerüste eingerichtet. Alle folgenden PowerShell-Befehle beginnen im Repository, soweit kein anderes Arbeitsverzeichnis angegeben ist.

## Voraussetzungen

Windows 11 x64, [Git für Windows](https://git-scm.com/download/win), PowerShell, [rustup](https://rustup.rs/) und [Visual Studio Build Tools](https://visualstudio.microsoft.com/downloads/) werden benötigt. Im Visual Studio Installer die Workload „Desktopentwicklung mit C++“ einschließlich MSVC-x64/x86-Buildtools und Windows SDK installieren. Danach rustup über den offiziellen Windows-Installer mit dem MSVC-Host installieren und eine neue PowerShell öffnen. Der lokale vollständige Desktop-Build hat mit den vorhandenen MSVC-Werkzeugen und dem Windows SDK erfolgreich kompiliert und gelinkt. Qt wird nicht benötigt.

Das Repository von seiner tatsächlichen Git-URL klonen (`<Repository-URL>` ersetzen):

```powershell
git clone <Repository-URL> controlpanel
Set-Location controlpanel
```

Für den ersten Build und die Paketinstallation wird eine Internetverbindung benötigt.

`rust-toolchain.toml` legt Rust 1.96.1, MSVC, rustfmt und clippy fest. Im Repository aktiviert rustup diese Version automatisch:

```powershell
rustup toolchain install 1.96.1 --profile minimal --component rustfmt --component clippy --target x86_64-pc-windows-msvc
rustc --version
cargo --version
cargo fmt --version
cargo clippy --version
```

## Node 24 lokal installieren

`.node-version` legt 24.22.2 fest. Diese LTS-Version stammt aus dem [offiziellen Node-Releaseindex](https://nodejs.org/dist/index.json). Sie erfüllt die [Astro-Voraussetzungen](https://docs.astro.build/en/install-and-setup/#prerequisites) (Node ab 22.23.4, keine ungeraden Major-Versionen). Die vorhandene globale Node-25-Installation bleibt erhalten.

Im Repository ausführen; die SHA256-Prüfung erfolgt vor dem Entpacken:

```powershell
$nodeSetupVersion = (Get-Content .node-version -Raw).Trim()
$nodeSetupArchive = "node-v$nodeSetupVersion-win-x64.zip"
$nodeSetupBase = "https://nodejs.org/dist/v$nodeSetupVersion"
New-Item -ItemType Directory -Force .tools | Out-Null
Invoke-WebRequest "$nodeSetupBase/$nodeSetupArchive" -OutFile ".tools/$nodeSetupArchive"
Invoke-WebRequest "$nodeSetupBase/SHASUMS256.txt" -OutFile .tools/SHASUMS256.txt
$nodeSetupEntry = Get-Content .tools/SHASUMS256.txt | Where-Object { $_ -match "  $([regex]::Escape($nodeSetupArchive))$" }
if (-not $nodeSetupEntry) { throw 'Offizielle Prüfsumme fehlt' }
$nodeSetupExpected = ($nodeSetupEntry -split '\s+')[0]
$nodeSetupActual = (Get-FileHash ".tools/$nodeSetupArchive" -Algorithm SHA256).Hash.ToLowerInvariant()
if ($nodeSetupActual -ne $nodeSetupExpected) { throw 'SHA256 stimmt nicht überein' }
Expand-Archive ".tools/$nodeSetupArchive" -DestinationPath .tools
```

Für das geprüfte Windows-x64-Archiv lautet SHA256 `ca1bff5d8ead13c9f2ea404a176db46bd430704d5b5113734049fa365e7799e2`. Offizielle [Downloads und Prüfsummen](https://nodejs.org/dist/v24.22.2/).

In jeder neuen PowerShell aus dem Repository aktivieren:

```powershell
. ./scripts/use-node.ps1
node --version
npm.cmd --version
```

Erwartet: `v24.22.2` und npm `11.19.1`. Das Skript ändert nur den PATH des aktuellen Prozesses. Nach Schließen der Shell gilt wieder deren normaler PATH. Unter Windows `npm.cmd` verwenden, damit eine PowerShell-Ausführungsrichtlinie für `npm.ps1` den Aufruf nicht verhindert. Es werden keine globalen npm-Pakete benötigt.

`.tools`, Cargo-Ausgaben, npm-Abhängigkeiten, Astro-Ausgaben und lokale SDD-Artefakte sind ignoriert. Cargo.lock und apps/website/package-lock.json sind versioniert.

## Struktur und Versionen

```text
Cargo.toml / Cargo.lock        Cargo-Workspace und festgelegte Abhängigkeiten
rust-toolchain.toml            Rust 1.96.1, rustfmt, clippy, Windows MSVC
.node-version                 Node 24.22.2
scripts/use-node.ps1           Node für die aktuelle Shell aktivieren
apps/desktop/                 Rust-Binary nexpanel, build.rs, src/, ui/
apps/website/                 Astro, package.json und package-lock.json
  src/pages/index.astro       Deutsche statische Setup-Seite
  astro.config.mjs            Statische Ausgabe
  tsconfig.json               Astro strict
.github/workflows/ci.yml       Getrennte Desktop- und Website-Prüfung
docs/                         Setup, Entwurf und Umsetzungsplan
NexPanel.md                   Produktanforderungen
```

Rust und Cargo sind auf 1.96.1 festgelegt. Slint und slint-build verwenden beide exakt 1.18.1. Slint nutzt winit und Software-Rendering mit Accessibility; diese Auswahl vermeidet Qt und eine zusätzliche GPU-Renderer-Voraussetzung. Sie ist keine Performance-Aussage. Die Website verwendet Astro 7.4.4, @astrojs/check 0.9.12 und TypeScript 6.0.3. TypeScript 6 liegt im unterstützten Peer-Bereich des Checkers; das neuere TypeScript 7 wurde deshalb nicht ausgewählt.

## Desktop starten, prüfen und bauen

Im Repository ausführen:

```powershell
cargo run -p nexpanel
```

Das Fenster trägt den Titel „NexPanel“ und zeigt „Setup bereit“. Normales Schließen beendet die Anwendung. Es gibt noch kein Tray-Verhalten.

```powershell
cargo fmt --all -- --check
cargo check --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --workspace --locked
```

Der Debug-Build liegt unter `target/debug/nexpanel.exe`. Für einen optimierten Build `cargo build --workspace --release --locked` verwenden; dessen Ausgabe liegt unter `target/release/nexpanel.exe`. Der Release-Build ist hier als Bedienbefehl dokumentiert und wurde bei der Bootstrap-Abnahme nicht separat geprüft.

## Website installieren, starten und bauen

In einer neuen Shell zunächst Node aus dem Repository aktivieren:

```powershell
. ./scripts/use-node.ps1
Set-Location apps/website
npm.cmd ci
npm.cmd run dev
```

Die im Terminal ausgegebene Adresse im Browser öffnen (standardmäßig `http://localhost:4321`). Mit Strg+C den Server beenden. Die folgenden Befehle ebenfalls in `apps/website` ausführen:

```powershell
npm.cmd run check
npm.cmd run build
npm.cmd run preview
```

Der statische Build liegt in `apps/website/dist`. Preview stellt diesen Build lokal bereit; auch diesen Server mit Strg+C beenden. Zum Repository zurückkehren: `Set-Location ../..`. `npm ci` installiert anhand des vorhandenen Lockfiles; für normale Installation keinen neuen Abhängigkeitsstand erzeugen.

## Erbrachte Nachweise und Grenzen

Die lokale Abnahme am 2026-10-31 verwendete Windows MSVC, Rust/Cargo 1.96.1 und das projektlokale Node 24.22.2 mit dem mitgelieferten npm 11.19.1. Eine frische PowerShell bestätigte beide Node/npm-Versionen und den Node-Pfad unter `.tools`; die globale Node-Installation blieb unverändert.

Alle vier oben genannten Cargo-Prüfungen waren erfolgreich und ohne Warnungen. Der initiale Check dauerte 54,43 s, Clippy 8,45 s und der vollständige Debug-Build 42,60 s. Das sind einzelne lokale Bootstrap-Läufe, keine Benchmarks. Ein gestarteter Desktop-Prozess blieb nach drei Sekunden aktiv und besaß ein natives Fenster mit Titel „NexPanel“. `CloseMainWindow()` schloss es innerhalb von fünf Sekunden mit Exitcode 0. Der Start erfolgte versteckt; eine visuelle oder menschliche Prüfung fand nicht statt. Pixelbild, DPI-Skalierung und Ressourcenverbrauch sind ungeprüft.

`npm ci` installierte 264 Pakete; die damalige Prüfung von 265 Paketen meldete keine Schwachstellen. `npm run check` meldete bei drei Dateien null Fehler, Warnungen und Hinweise. `npm run build` erzeugte eine statische Seite. Ein HTTP-Smoke-Test über die Astro-Preview-API auf `127.0.0.1:4327` lieferte Status 200, `lang="de"`, den Titel „NexPanel – Setup“ und den erwarteten Viewport. Danach wurde der Server beendet und ein weiterer HTTP-Zugriff schlug erwartungsgemäß fehl. Eine visuelle Browserprüfung wurde nicht durchgeführt. Sicherheitsprüfungen sind eine Momentaufnahme des damaligen Paketstands.

## CI

Der Workflow läuft bei Push, Pull Request und manuell. Der Windows-Job installiert Rust 1.96.1 einschließlich rustfmt, clippy und MSVC-Ziel und führt die vier dokumentierten Cargo-Prüfungen im Repository aus. Der separate Ubuntu-Job liest die exakte Node-Version aus `.node-version`, prüft Node 24.22.2 und npm 11.19.1 und führt `npm ci`, `npm run check` und `npm run build` in `apps/website` aus. Der npm-Cache verwendet `apps/website/package-lock.json`.

Verwendet werden die am 2026-10-31 anhand der offiziellen Releases geprüften Versionen [actions/checkout 7.0.1](https://github.com/actions/checkout/releases/tag/v7.0.1) und [actions/setup-node 7.1.0](https://github.com/actions/setup-node/releases/tag/v7.1.0). CI prüft Kompilierung, Linting und Website-Ausgabe; die lokale Fenster- und HTTP-Laufzeitprüfung wird nicht als CI-Test ausgegeben. Ohne Push wurde der Workflow noch nicht auf GitHub ausgeführt. Veröffentlichungen, Deployment, Installer sowie Performance- und Hardwareziele sind nicht Bestandteil dieser Abnahme. Dieses Setup legt keine Projektlizenz fest.
