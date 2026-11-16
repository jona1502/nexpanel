[CmdletBinding()]
param([string]$InstallDirectory = $PSScriptRoot, [switch]$NoShortcut)
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
$forbidden = @($root, $env:USERPROFILE, $env:LOCALAPPDATA, $env:WINDIR, $env:ProgramFiles, ${env:ProgramFiles(x86)}) | Where-Object { $_ } | ForEach-Object { [IO.Path]::GetFullPath($_).TrimEnd('\','/') }
if ($destination -in $forbidden) { throw 'Unsicheres Deinstallationsverzeichnis.' }
$markerPath = Join-Path $destination '.nexpanel-install.json'
if (-not (Test-Path -LiteralPath $markerPath -PathType Leaf)) { throw 'Kein verwaltetes NexPanel-Installationsverzeichnis.' }
$marker = Get-Content -LiteralPath $markerPath -Raw | ConvertFrom-Json
if ($marker.product -ne 'NexPanel' -or $marker.directory -ine $destination) { throw 'Installationsmarkierung passt nicht zum Ziel.' }
$exe = Join-Path $destination 'nexpanel.exe'
# Nur ein Verzeichnis mit genau dem mitgelieferten Deinstallationsskript akzeptieren.
$installedScript = Join-Path $destination 'uninstall.ps1'
if (-not (Test-Path -LiteralPath $installedScript -PathType Leaf) -or (Get-FileHash -LiteralPath $installedScript).Hash -ne (Get-FileHash -LiteralPath $PSCommandPath).Hash) { throw 'Kein passendes NexPanel-Installationsverzeichnis.' }
# Lock-Prüfung vor jeder Änderung; niemals fremde Prozesse beenden.
if (Test-Path -LiteralPath $exe) {
  try { $handle = [IO.File]::Open($exe, 'Open', 'ReadWrite', 'None'); $handle.Dispose() }
  catch { throw 'NexPanel bitte über das Tray-Menü beenden und die Deinstallation erneut starten.' }
}
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
if (Test-Path -LiteralPath $runKey) {
  $properties = Get-ItemProperty -LiteralPath $runKey
  $value = $properties.PSObject.Properties['NexPanel'].Value
  if ($value -is [string] -and $value.Trim() -ieq ('"' + $exe + '" --background')) { Remove-ItemProperty -LiteralPath $runKey -Name 'NexPanel' }
}
if (-not $NoShortcut) {
  $shortcutPath = Join-Path ([Environment]::GetFolderPath('Programs')) 'NexPanel.lnk'
  if (Test-Path -LiteralPath $shortcutPath) {
    $shell = New-Object -ComObject WScript.Shell
    if ($shell.CreateShortcut($shortcutPath).TargetPath -ieq $exe) { Remove-Item -LiteralPath $shortcutPath }
  }
}
foreach ($file in @('nexpanel.exe','install.ps1','README.md','uninstall.ps1','.nexpanel-install.json')) {
  $path = Join-Path $destination $file
  if (Test-Path -LiteralPath $path -PathType Leaf) { Remove-Item -LiteralPath $path -Force }
}
# Keine rekursive Entfernung: Einstellungen und fremde Dateien bleiben erhalten.
if (-not (Get-ChildItem -LiteralPath $destination -Force | Select-Object -First 1)) { Remove-Item -LiteralPath $destination }
Write-Host 'NexPanel deinstalliert. Einstellungen und andere Dateien bleiben erhalten.'
