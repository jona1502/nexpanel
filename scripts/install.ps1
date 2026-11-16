[CmdletBinding()]
param([string]$InstallDirectory = (Join-Path $env:LOCALAPPDATA 'NexPanel/bin'), [switch]$NoShortcut)
$ErrorActionPreference = 'Stop'
$destination = [IO.Path]::GetFullPath($InstallDirectory).TrimEnd('\','/')
# Verzeichnisverknüpfungen/Junctions nicht als Installationspfad verfolgen.
$ancestor = $destination
while ($ancestor) {
  if (Test-Path -LiteralPath $ancestor) {
    if ((Get-Item -LiteralPath $ancestor -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Installationspfad enthält eine Verzeichnisverknüpfung.' }
  }
  $parent = Split-Path -Parent $ancestor
  if ($parent -eq $ancestor) { break }
  $ancestor = $parent
}
foreach ($systemDirectory in @($env:WINDIR, $env:ProgramFiles, ${env:ProgramFiles(x86)})) {
  if ($systemDirectory -and ($destination -ieq $systemDirectory -or $destination.StartsWith($systemDirectory.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase))) { throw 'Systemverzeichnisse sind kein benutzerlokales Installationsziel.' }
}
$root = [IO.Path]::GetPathRoot($destination).TrimEnd('\','/')
if ($destination -eq $root -or $destination -eq [IO.Path]::GetFullPath($env:USERPROFILE).TrimEnd('\','/') -or $destination -eq [IO.Path]::GetFullPath($env:LOCALAPPDATA).TrimEnd('\','/')) { throw 'Unsicheres Installationsverzeichnis.' }
if ($destination -eq [IO.Path]::GetFullPath($PSScriptRoot).TrimEnd('\','/')) { throw 'Paket und Installationsziel müssen verschieden sein.' }
$markerPath = Join-Path $destination '.nexpanel-install.json'
if ((Test-Path -LiteralPath $destination) -and (Get-ChildItem -LiteralPath $destination -Force | Select-Object -First 1)) {
  if (-not (Test-Path -LiteralPath $markerPath -PathType Leaf)) { throw 'Nicht leeres, unverwaltetes Ziel. Bitte einen eigenen leeren Ordner wählen.' }
  $marker = Get-Content -LiteralPath $markerPath -Raw | ConvertFrom-Json
  if ($marker.product -ne 'NexPanel' -or $marker.directory -ine $destination) { throw 'Installationsmarkierung passt nicht zum Ziel.' }
}
$files = @('nexpanel.exe','install.ps1','uninstall.ps1','README.md')
foreach ($file in $files) { if (-not (Test-Path -LiteralPath (Join-Path $PSScriptRoot $file) -PathType Leaf)) { throw "Paketdatei fehlt: $file" } }
New-Item -ItemType Directory -Path $destination -Force | Out-Null
try { foreach ($file in $files) { Copy-Item -LiteralPath (Join-Path $PSScriptRoot $file) -Destination (Join-Path $destination $file) -Force } }
catch { throw "Installation fehlgeschlagen. NexPanel bitte schließen und erneut versuchen. $($_.Exception.Message)" }
@{ product = 'NexPanel'; directory = $destination; version = '0.1.0' } | ConvertTo-Json | Set-Content -LiteralPath $markerPath -Encoding utf8
if (-not $NoShortcut) {
  $shortcutPath = Join-Path ([Environment]::GetFolderPath('Programs')) 'NexPanel.lnk'
  $shell = New-Object -ComObject WScript.Shell
  $shortcut = $shell.CreateShortcut($shortcutPath)
  if ((Test-Path -LiteralPath $shortcutPath) -and $shortcut.TargetPath -and $shortcut.TargetPath -ne (Join-Path $destination 'nexpanel.exe')) { throw 'Vorhandene NexPanel-Verknüpfung gehört zu einer anderen Installation. -NoShortcut verwenden.' }
  $shortcut.TargetPath = Join-Path $destination 'nexpanel.exe'
  $shortcut.WorkingDirectory = $destination
  $shortcut.Description = 'NexPanel – Windows Control Center'
  $shortcut.Save()
}
Write-Host "NexPanel installiert: $destination. Die Anwendung wurde nicht gestartet. Einstellungen und Autostart bleiben unverändert."
