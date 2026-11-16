# Setup und Bedienung

Stand: 2026-11-01. Alle Befehle beginnen im Repository, sofern nicht anders angegeben.

## Voraussetzungen

Windows 11 x64, Git, PowerShell und rustup. In Visual Studio Build Tools die Workload „Desktopentwicklung mit C++“ mit MSVC-x64/x86 und Windows SDK installieren. Qt ist nicht erforderlich. Für den ersten Build und npm ci ist Internet nötig.

```powershell
git clone https://github.com/jona1502/nexpanel.git
cd nexpanel
rustup toolchain install 1.96.1 --profile minimal --component rustfmt --component clippy --target x86_64-pc-windows-msvc
cargo run -p nexpanel
```

`rust-toolchain.toml` aktiviert Rust 1.96.1 automatisch. Cargo.lock ist versioniert. Slint/slint-build 1.18.1 nutzen winit und Software-Rendering mit Accessibility. Das ist keine Zusage zum Ressourcenverbrauch.

## Bedienung

Das Panel startet sichtbar; `cargo run -p nexpanel -- --background` startet im Tray. Tray-Klick schaltet es um, das Menü bietet Öffnen und Beenden. Esc, × und Fenster-Schließen blenden aus. Fokusverlust blendet bei aktivierter Einstellung aus. Eine zweite Instanz öffnet die vorhandene Instanz derselben Windows-Sitzung. Bei hoher DPI oder kleinem Arbeitsbereich wird das Panel auf die verfügbare Fläche verkleinert; Inhalte bleiben scrollbar.

Displays zeigen echte erkannte Geräte und deren Fähigkeiten. DDC/CI muss auf externen Monitoren aktiviert und vom Anschluss/Treiber unterstützt sein; interne Helligkeit benötigt einen passenden WMI-Anbieter. Nicht unterstützte oder mehrdeutige Zuordnungen bleiben deaktiviert mit Erklärung. Gekoppelte Regelung wirkt nur auf unterstützte Geräte. Audio steuert das Standard-Wiedergabegerät und zusammengefasste Prozess-Sessions; es gibt keine Geräteauswahl.

Countdown (1 Sekunde bis 24 Stunden) und Stoppuhr laufen bei verborgenem Panel weiter. Ablauf öffnet einen sichtbaren Hinweis mit Systemton; „Neu starten“ beginnt den Countdown erneut mit der gewählten Dauer und löscht den Hinweis. Schlafzeit zählt mit; Systemzeitänderungen beeinflussen die Zeitmessung. Beim App-Neustart werden beide zurückgesetzt. Akzentfarbe, Animationen, Fokusverhalten, Displaykopplung und Autostart stehen in Einstellungen. Autostart nutzt nur den eigenen HKCU-Run-Wert mit `--background`.

Konfiguration: `%LOCALAPPDATA%\NexPanel\settings.json`. Fehlende Datei lädt Defaults; beschädigte Datei bleibt erhalten und wird gemeldet. Ein ausdrücklicher Reset ersetzt Einstellungen. Installation verändert die Konfiguration nicht.

## Release-Paket und Installation

```powershell
./scripts/package.ps1
# Nur wenn der aktuelle Release-Build bereits existiert:
./scripts/package.ps1 -SkipBuild
```

Standardmäßig läuft `cargo build -p nexpanel --release --locked`. Ausgabe: `dist/nexpanel-<Version>-windows-x64.zip` und `.zip.sha256` (Version aus `apps/desktop/Cargo.toml`). Das ZIP enthält exe, `THIRD-PARTY-NOTICES.md`, `VERSION.txt`, Installations-/Deinstallationsskript und deutsche Anleitung. SHA256 des ZIP mit `Get-FileHash -Algorithm SHA256` vergleichen. Die Prüfsumme erkennt Übertragungsfehler, ersetzt aber keine vertrauenswürdige Herkunft oder Signatur.

ZIP in einen eigenen Ordner entpacken, PowerShell dort öffnen:

```powershell
./install.ps1
# Optional: eigener Ordner, keine Startmenü-Verknüpfung
./install.ps1 -InstallDirectory 'C:\MeinOrdner\NexPanel' -NoShortcut
```

Standardziel ist `%LOCALAPPDATA%\NexPanel\bin`. Kein Administratorrecht, kein automatischer Start, keine Aktivierung von Autostart. Unter Windows können Skriptausführungsrichtlinien gelten: Herkunft und Inhalt prüfen und die lokale Richtlinie beachten. Das Projekt ändert keine globale Ausführungsrichtlinie.

Vor Update oder Deinstallation NexPanel über das Tray-Menü beenden. `uninstall.ps1` aus dem installierten Ordner ausführen, bei Installation ohne Shortcut ebenfalls `-NoShortcut`. Entfernt bekannte Paketdateien und nur die auf genau diese exe zeigende Verknüpfung bzw. den passenden eigenen Autostartwert. Einstellungen und fremde Dateien bleiben erhalten; keine rekursive Löschung und kein erzwungenes Prozessende.

## Website

Node 24.22.2 mit npm 11.19.1 verwenden; `.node-version` und package.json legen diese Versionen fest. Eine vorhandene globale Node-Installation wird nicht verändert. Alternativ kann das offizielle Windows-x64-ZIP projektlokal installiert werden:

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
if ((Get-FileHash ".tools/$nodeSetupArchive" -Algorithm SHA256).Hash.ToLowerInvariant() -ne $nodeSetupExpected) { throw 'SHA256 stimmt nicht überein' }
Expand-Archive ".tools/$nodeSetupArchive" -DestinationPath .tools
. ./scripts/use-node.ps1
cd apps/website
npm.cmd ci
npm.cmd run check
npm.cmd run build
npm.cmd run dev
```

Dev-Adresse aus dem Terminal öffnen (normalerweise localhost:4321). Strg+C beendet den Server. `npm.cmd run preview` stellt den statischen Build aus `apps/website/dist` bereit; ebenfalls mit Strg+C beenden. Astro 7.4.4, @astrojs/check 0.9.12 und TypeScript 6.0.3; keine UI-Frameworks oder zusätzlichen Website-Abhängigkeiten. `.tools`, `target`, `dist`, node_modules und lokale SDD-Nachweise sind ignoriert.

## Prüfungen

```powershell
cargo fmt --all -- --check
cargo check --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --release --locked
./target/release/nexpanel.exe --diagnostics
./target/release/nexpanel.exe --capture-preview .superpowers/preview
```

Diagnose liest nur; Capture nutzt reale Gerätewerte und kann persönliche App-Namen enthalten. Solche Bilder nicht ungeprüft veröffentlichen. [Testnachweise und offene praktische Prüfungen](testing.md). CI führt Rust-Prüfungen einschließlich Tests und Release-Build, den isolierten Pakettest sowie Astro check/build aus. Remote-CI wurde ohne Push nicht ausgeführt. Kein Hosting/Deployment, keine Codesignatur oder Lizenzwahl durch diese lokale Umsetzung.
