# Gemeinsame Helfer fuer Release-Skripte (per Dot-Sourcing laden).
function Get-NexPanelVersion {
  param([string]$Repo = (Split-Path -Parent $PSScriptRoot))
  $manifest = Join-Path $Repo 'apps/desktop/Cargo.toml'
  if (-not (Test-Path -LiteralPath $manifest -PathType Leaf)) { throw "Cargo.toml fehlt: $manifest" }
  $inPackage = $false
  foreach ($line in Get-Content -LiteralPath $manifest) {
    if ($line -match '^\s*\[(.+?)\]\s*(#.*)?$') { $inPackage = ($Matches[1].Trim() -eq 'package'); continue }
    if ($inPackage -and $line -match '^\s*version\s*=\s*"([^"]+)"') { return $Matches[1] }
  }
  throw "Keine [package] version in $manifest gefunden."
}
function Get-NexPanelPackageName {
  param([string]$Version = (Get-NexPanelVersion))
  "nexpanel-$Version-windows-x64.zip"
}
