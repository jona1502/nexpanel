# Lokales Setup

Stand: 2026-10-31. Die Werkzeugbasis ist eingerichtet; Desktop und Website werden in den folgenden Bootstrap-Schritten ergänzt. Start- und Buildbefehle werden dann mit den tatsächlichen Ergebnissen dokumentiert.

## Voraussetzungen

Windows x64, Git, PowerShell und [rustup](https://rustup.rs/) sowie Visual Studio Build Tools mit MSVC-C++-Werkzeugen und Windows SDK. Der tatsächliche Desktop-Build prüft die native Toolchain im nächsten Schritt.

`rust-toolchain.toml` legt Rust 1.96.1, MSVC, rustfmt und clippy fest. Im Repository aktiviert rustup diese Version automatisch:

```powershell
rustup toolchain install 1.96.1 --profile minimal --component rustfmt --component clippy
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

`.tools`, Cargo-Ausgaben, npm-Abhängigkeiten, Astro-Ausgaben und lokale SDD-Artefakte sind ignoriert. Cargo.lock und package-lock.json werden mit den jeweiligen Grundgerüsten versioniert.
