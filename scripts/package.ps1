[CmdletBinding()]
param([switch]$SkipBuild)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
Push-Location $repo
try {
  if (-not $SkipBuild) { cargo build -p nexpanel --release --locked; if ($LASTEXITCODE -ne 0) { throw 'Release-Build fehlgeschlagen.' } }
  $binary = Join-Path $repo 'target/release/nexpanel.exe'
  if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) { throw 'Release-Binary fehlt. Zuerst ohne -SkipBuild ausführen.' }
  $dist = Join-Path $repo 'dist'
  New-Item -ItemType Directory -Path $dist -Force | Out-Null
  $stage = Join-Path $dist ('package-' + [guid]::NewGuid().ToString('N'))
  New-Item -ItemType Directory -Path $stage | Out-Null
  try {
    Copy-Item -LiteralPath $binary -Destination $stage
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'install.ps1'),(Join-Path $PSScriptRoot 'uninstall.ps1') -Destination $stage
    Copy-Item -LiteralPath (Join-Path $repo 'docs/package-readme.md') -Destination (Join-Path $stage 'README.md')
    $zip = Join-Path $dist 'nexpanel-0.1.0-windows-x64.zip'
    Compress-Archive -LiteralPath (Get-ChildItem -LiteralPath $stage -File).FullName -DestinationPath $zip -Force
    $hash = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $([IO.Path]::GetFileName($zip))" | Set-Content -LiteralPath "$zip.sha256" -Encoding ascii
    Write-Host "Paket: $zip"
    Write-Host "SHA256: $hash"
  } finally {
    # Nur eigene bekannte Dateien in der frisch erzeugten Stage löschen.
    foreach ($name in @('nexpanel.exe','install.ps1','uninstall.ps1','README.md')) { $path = Join-Path $stage $name; if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path } }
    Remove-Item -LiteralPath $stage
  }
} finally { Pop-Location }
