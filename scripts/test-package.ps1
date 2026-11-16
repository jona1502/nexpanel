$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'common.ps1')
$version = Get-NexPanelVersion
$zipPath = Join-Path (Split-Path -Parent $PSScriptRoot) (Join-Path 'dist' (Get-NexPanelPackageName -Version $version))
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('distribution-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory $testRoot | Out-Null
$unpacked = Join-Path $testRoot 'package'
Expand-Archive -LiteralPath $zipPath -DestinationPath $unpacked
$destination = Join-Path $testRoot 'installed'
foreach ($f in 'THIRD-PARTY-NOTICES.md','VERSION.txt') { if (-not (Test-Path "$unpacked/$f") -or (Get-Item "$unpacked/$f").Length -eq 0) { throw "Paketdatei fehlt/leer: $f" } }
if ((Get-Content "$unpacked/VERSION.txt" -Raw).Trim() -ne $version) { throw 'VERSION.txt passt nicht zu Cargo.toml' }
& "$unpacked/install.ps1" -InstallDirectory $destination -NoShortcut
if (-not (Test-Path "$destination/THIRD-PARTY-NOTICES.md")) { throw 'Notices nicht installiert' }
if ((Get-FileHash "$destination/nexpanel.exe").Hash -ne (Get-FileHash target/release/nexpanel.exe).Hash) { throw 'Binary mismatch' }
$p = Start-Process -FilePath "$destination/nexpanel.exe" -ArgumentList '--help' -Wait -PassThru -WindowStyle Hidden
if ($p.ExitCode -ne 0) { throw 'Installed binary failed' }
Set-Content "$destination/settings.json" '{"sentinel":true}'
Set-Content "$destination/unrelated.txt" 'preserve me'
& "$unpacked/install.ps1" -InstallDirectory $destination -NoShortcut
& "$destination/uninstall.ps1" -NoShortcut
if ((Get-Content "$destination/unrelated.txt" -Raw).Trim() -ne 'preserve me' -or (Get-Content "$destination/settings.json" -Raw).Trim() -ne '{"sentinel":true}') { throw 'Preservation failed' }
if ((Test-Path "$destination/nexpanel.exe") -or (Test-Path "$destination/THIRD-PARTY-NOTICES.md")) { throw 'Uninstall failed' }
$refused=$false
try { & "$unpacked/uninstall.ps1" -NoShortcut } catch { $refused=$true }
if (-not $refused -or -not (Test-Path "$unpacked/install.ps1")) { throw 'Unmanaged uninstall accepted' }
$refused=$false
try { & "$unpacked/install.ps1" -InstallDirectory $destination -NoShortcut } catch { $refused=$true }
if (-not $refused) { throw 'Nonempty unmanaged install accepted' }
Get-ChildItem scripts/*.ps1 | ForEach-Object { $tokens=$null; $errors=$null; [void][System.Management.Automation.Language.Parser]::ParseFile($_.FullName,[ref]$tokens,[ref]$errors); if ($errors) { throw $errors } }
$dummySource = Join-Path $testRoot 'source-scripts'
New-Item -ItemType Directory $dummySource | Out-Null
Copy-Item -LiteralPath scripts/install.ps1,scripts/uninstall.ps1 -Destination $dummySource
$refused=$false
try { & "$dummySource/uninstall.ps1" -NoShortcut } catch { $refused=$true }
if (-not $refused -or -not (Test-Path "$dummySource/install.ps1")) { throw 'Source-directory uninstall accepted' }
'PASS binary/hash/help, upgrade, uninstall preservation, unmanaged/source refusal, syntax'
Write-Host "Testnachweise verbleiben unter $testRoot"
