[CmdletBinding()]
param([string]$OutFile)
# Erzeugt THIRD-PARTY-NOTICES.md aus der aufgeloesten Normal-Abhaengigkeitsliste von apps/desktop
# (Ziel x86_64-pc-windows-msvc). Nutzt nur `cargo metadata` (kompiliert nichts) und die Crate-Quellen
# in der lokalen Cargo-Registry (zuvor per cargo fetch/build vorhanden).
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
if (-not $OutFile) { $OutFile = Join-Path $repo 'dist/THIRD-PARTY-NOTICES.md' }
Push-Location $repo
try {
  $json = cargo metadata --format-version 1 --locked --filter-platform x86_64-pc-windows-msvc
  if ($LASTEXITCODE -ne 0) { throw 'cargo metadata fehlgeschlagen.' }
  $meta = ($json -join "`n") | ConvertFrom-Json
} finally { Pop-Location }

$byId = @{}; foreach ($p in $meta.packages) { $byId[$p.id] = $p }
$nodes = @{}; foreach ($n in $meta.resolve.nodes) { $nodes[$n.id] = $n }
$root = $meta.packages | Where-Object { $_.name -eq 'nexpanel' -and $_.manifest_path -like '*apps*desktop*' } | Select-Object -First 1
if (-not $root) { throw 'Paket nexpanel (apps/desktop) nicht gefunden.' }

# Nur normale Abhaengigkeiten (kind = null), transitiv.
$seen = New-Object 'System.Collections.Generic.HashSet[string]'
$queue = New-Object 'System.Collections.Generic.Queue[string]'
$queue.Enqueue($root.id)
while ($queue.Count -gt 0) {
  $id = $queue.Dequeue()
  foreach ($d in $nodes[$id].deps) {
    if (-not ($d.dep_kinds | Where-Object { $null -eq $_.kind })) { continue }
    if ($seen.Add($d.pkg)) { $queue.Enqueue($d.pkg) }
  }
}
$crates = $seen | ForEach-Object { $byId[$_] } | Where-Object { $_.source } | Sort-Object name, version
$workspaceSkipped = $seen.Count - @($crates).Count

$sha = [Security.Cryptography.SHA256]::Create()
$texts = @{}   # hash -> @{ Text; Crates = list }
$noText = New-Object System.Collections.Generic.List[object]
foreach ($c in $crates) {
  $dir = Split-Path -Parent $c.manifest_path
  $files = @()
  if (Test-Path -LiteralPath $dir) {
    $files = @(Get-ChildItem -LiteralPath $dir -File -ErrorAction SilentlyContinue |
      Where-Object { $_.Name -match '^(LICEN[CS]E|COPYING|NOTICE|UNLICENSE)' -and $_.Length -lt 200KB })
  }
  if ($files.Count -eq 0) { $noText.Add($c); continue }
  foreach ($f in $files) {
    $text = ([IO.File]::ReadAllText($f.FullName)).Replace("`r`n", "`n").Trim()
    if (-not $text) { continue }
    $h = [BitConverter]::ToString($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($text))).Replace('-', '')
    if (-not $texts.ContainsKey($h)) { $texts[$h] = @{ Text = $text; Crates = New-Object System.Collections.Generic.List[string]; Name = $f.Name } }
    $label = "$($c.name) $($c.version)"
    if (-not $texts[$h].Crates.Contains($label)) { $texts[$h].Crates.Add($label) }
  }
}

$sb = New-Object System.Text.StringBuilder
[void]$sb.AppendLine('# Third-Party Notices / Drittanbieter-Hinweise')
[void]$sb.AppendLine()
[void]$sb.AppendLine('- NexPanel: Die eigene Lizenz von NexPanel ist noch nicht gewaehlt (kein Nutzungsrecht fuer Dritte bis dahin).')
[void]$sb.AppendLine('- Slint wird unter der Slint Royalty-free License 2.0 verwendet; die geforderte AboutSlint-Namensnennung steht in Einstellungen > "Ueber NexPanel". Details: docs/licensing.md im Repository.')
[void]$sb.AppendLine('- Windows-Systemschriften (z. B. Segoe UI) werden nicht mitgeliefert oder weiterverteilt.')
[void]$sb.AppendLine()
[void]$sb.AppendLine("Die folgende Liste umfasst $(@($crates).Count) Rust-Crates (normale Abhaengigkeiten von apps/desktop, Ziel x86_64-pc-windows-msvc, aus Cargo.lock aufgeloest). Sie ist eine konservative Obergrenze der tatsaechlich gelinkten Crates.")
[void]$sb.AppendLine()
[void]$sb.AppendLine('## Crates')
[void]$sb.AppendLine()
[void]$sb.AppendLine('| Crate | Version | Lizenz (SPDX) | Repository |')
[void]$sb.AppendLine('|---|---|---|---|')
foreach ($c in $crates) {
  $lic = if ($c.license) { $c.license } elseif ($c.license_file) { "siehe $($c.license_file)" } else { 'unbekannt' }
  $repoUrl = if ($c.repository) { $c.repository } else { '-' }
  [void]$sb.AppendLine("| $($c.name) | $($c.version) | ``$($lic -replace '\|','/')`` | $repoUrl |")
}
[void]$sb.AppendLine()
[void]$sb.AppendLine('## Lizenztexte')
[void]$sb.AppendLine()
$i = 0
foreach ($entry in ($texts.GetEnumerator() | Sort-Object { $_.Value.Crates.Count } -Descending)) {
  $i++
  [void]$sb.AppendLine("### Text $i ($($entry.Value.Name))")
  [void]$sb.AppendLine()
  [void]$sb.AppendLine('Gilt fuer: ' + ($entry.Value.Crates -join ', '))
  [void]$sb.AppendLine()
  [void]$sb.AppendLine('~~~~text')
  [void]$sb.AppendLine($entry.Value.Text)
  [void]$sb.AppendLine('~~~~')
  [void]$sb.AppendLine()
}
if ($noText.Count -gt 0) {
  [void]$sb.AppendLine('## Crates ohne gefundene Lizenzdatei')
  [void]$sb.AppendLine()
  [void]$sb.AppendLine('Lizenztext siehe Repository:')
  [void]$sb.AppendLine()
  foreach ($c in $noText) { [void]$sb.AppendLine("- $($c.name) $($c.version): $($c.license), $($c.repository) - Lizenztext siehe Repository") }
}
New-Item -ItemType Directory -Path (Split-Path -Parent $OutFile) -Force | Out-Null
[IO.File]::WriteAllText($OutFile, $sb.ToString(), (New-Object Text.UTF8Encoding($false)))
Write-Host "Notices: $OutFile ($(@($crates).Count) Crates, $($texts.Count) Texte, $($noText.Count) ohne Text, $([int]((Get-Item $OutFile).Length/1KB)) KB)"
