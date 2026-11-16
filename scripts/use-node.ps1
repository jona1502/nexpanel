# Dot-source this script: . ./scripts/use-node.ps1
$nodeProjectRoot = Split-Path -Parent $PSScriptRoot
$nodeProjectVersion = (Get-Content -LiteralPath (Join-Path $nodeProjectRoot '.node-version') -Raw).Trim()
$nodeProjectDirectory = Join-Path $nodeProjectRoot ".tools/node-v$nodeProjectVersion-win-x64"
if (-not (Test-Path -LiteralPath (Join-Path $nodeProjectDirectory 'node.exe'))) {
  throw "Node $nodeProjectVersion fehlt unter $nodeProjectDirectory. Siehe docs/setup.md."
}
$env:PATH = $nodeProjectDirectory + [IO.Path]::PathSeparator + (($env:PATH -split [IO.Path]::PathSeparator | Where-Object { $_ -ne $nodeProjectDirectory }) -join [IO.Path]::PathSeparator)
Write-Host "Node $nodeProjectVersion fuer diese Shell aktiviert. npm.cmd verwenden."
