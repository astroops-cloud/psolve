# psolve vs astap_cli on the capture machine itself.
#
# Frames are COPIED into the test directory first: astap_cli writes its .ini
# sidecar next to the frame it solved, and the capture tree is not ours to
# write into. Originals are never touched.

# NOT "Stop": PowerShell treats any native-command stderr write as a
# terminating error under it, and psolve writes its progress line to stderr.
# That is psolve behaving correctly and PowerShell being surprising.
$ErrorActionPreference = "Continue"
$root    = "C:\Users\nrf\psolve-test"
$frames  = Join-Path $root "frames"
$psolve  = Join-Path $root "psolve.exe"
$index   = Join-Path $root "index\gaia-dr3-g14-allsky-nside64.psidx"
$astap   = "C:\Program Files\astap\astap_cli.exe"
$astapdb = "C:\Program Files\astap"
$perTarget = [int]$args[0]
if (-not $perTarget) { $perTarget = 4 }

# --- select frames: stratified, a few per target, so one big target cannot
#     dominate the result the way it would with a flat sample.
$all = Get-ChildItem C:\Users\nrf -Filter *.fits -Recurse -ErrorAction SilentlyContinue |
       Where-Object { $_.FullName -match "LIGHT" -and $_.FullName -notmatch "psolve-test" }
$sel = $all | Group-Object { Split-Path (Split-Path $_.FullName -Parent) -Parent } |
       ForEach-Object { $_.Group | Sort-Object Name | Select-Object -First $perTarget }
Write-Host ("selected " + $sel.Count + " frames from " + ($sel | Group-Object DirectoryName).Count + " sessions")

Get-ChildItem $frames -Filter *.* -ErrorAction SilentlyContinue | Remove-Item -Force
$i = 0
foreach ($f in $sel) {
  $i++
  Copy-Item $f.FullName (Join-Path $frames ("{0:d3}_{1}" -f $i, $f.Name)) -Force
}
$work = Get-ChildItem $frames -Filter *.fits | Sort-Object Name
Write-Host ("copied " + $work.Count + " frames")

$results = @()
foreach ($f in $work) {
  $row = [ordered]@{ frame = $f.Name }

  # --- psolve
  $sw = [Diagnostics.Stopwatch]::StartNew()
  $out = (& $psolve solve $f.FullName --index $index 2>&1 | Where-Object { $_ -isnot [System.Management.Automation.ErrorRecord] }) -join "`n"
  $sw.Stop()
  $row.psolve_ms = [math]::Round($sw.Elapsed.TotalMilliseconds, 1)
  try {
    $j = $out | ConvertFrom-Json
    $row.psolve_solved = $j.solved
    $row.psolve_reason = $j.reason
    $row.psolve_ra  = if ($j.solved) { [math]::Round($j.field.center.ra, 6) }  else { $null }
    $row.psolve_dec = if ($j.solved) { [math]::Round($j.field.center.dec, 6) } else { $null }
    $row.psolve_rms = if ($j.solved) { [math]::Round($j.fit.rms_arcsec, 3) }   else { $null }
    $row.stars_used = if ($j.solved) { $j.stars.used } else { $j.stars.used }
  } catch {
    $row.psolve_solved = $false; $row.psolve_reason = "PARSE_FAIL"
  }

  # --- astap_cli, same frame, its own local d50 database
  $ini = [IO.Path]::ChangeExtension($f.FullName, ".ini")
  if (Test-Path $ini) { Remove-Item $ini -Force }
  $sw = [Diagnostics.Stopwatch]::StartNew()
  & $astap -f $f.FullName -r 30 -fov 0 -d $astapdb 2>&1 | Out-Null
  $sw.Stop()
  $row.astap_ms = [math]::Round($sw.Elapsed.TotalMilliseconds, 1)
  $row.astap_solved = $false; $row.astap_ra = $null; $row.astap_dec = $null
  if (Test-Path $ini) {
    $kv = @{}
    Get-Content $ini | ForEach-Object { if ($_ -match "^([A-Z0-9_]+)=(.*)$") { $kv[$Matches[1]] = $Matches[2] } }
    if ($kv["PLTSOLVD"] -eq "T") {
      $row.astap_solved = $true
      $row.astap_ra  = [math]::Round([double]$kv["CRVAL1"], 6)
      $row.astap_dec = [math]::Round([double]$kv["CRVAL2"], 6)
    }
    $row.astap_error = $kv["ERROR"]
  }

  # Separation is deliberately NOT computed here: an earlier version got it
  # wrong and reported 0" for every frame, which looked like perfect agreement.
  # The CSV carries both coordinate pairs; the caller computes it where the
  # arithmetic can be checked against a second implementation.
  $row.sep_arcsec = $null

  # Probe exposures are pointing checks, not imaging. They are a legitimate
  # part of the workload but must be counted separately -- a sample that is
  # half probes says little about a night's real frames.
  $row.is_probe = ($f.Name -match "_L_15\.00s_" -or $f.Name -match "PROBE")

  $results += [pscustomobject]$row
  Write-Host ("  " + $f.Name.Substring(0, [math]::Min(40, $f.Name.Length)).PadRight(42) +
              " psolve=" + $row.psolve_solved + "/" + $row.psolve_ms + "ms" +
              "  astap=" + $row.astap_solved + "/" + $row.astap_ms + "ms" +
              $(if ($row.is_probe) { "  [probe]" } else { "" }))
}

$results | Export-Csv -NoTypeInformation -Path (Join-Path $root "out\bench.csv")
Write-Host ""
Write-Host "=== SUMMARY ==="
$n = $results.Count
$ps = ($results | Where-Object { $_.psolve_solved }).Count
$as = ($results | Where-Object { $_.astap_solved }).Count
Write-Host ("frames            : " + $n)
Write-Host ("psolve solved     : " + $ps + "  (" + [math]::Round(100*$ps/$n,1) + "%)")
Write-Host ("astap solved      : " + $as + "  (" + [math]::Round(100*$as/$n,1) + "%)")
$pm = ($results | Where-Object { $_.psolve_solved } | Measure-Object psolve_ms -Average -Maximum)
$am = ($results | Where-Object { $_.astap_solved } | Measure-Object astap_ms  -Average -Maximum)
if ($pm.Count) { Write-Host ("psolve ms (solved): mean " + [math]::Round($pm.Average,0) + "  max " + [math]::Round($pm.Maximum,0)) }
if ($am.Count) { Write-Host ("astap  ms (solved): mean " + [math]::Round($am.Average,0) + "  max " + [math]::Round($am.Maximum,0)) }
foreach ($grp in @($true,$false)) {
  $g = $results | Where-Object { $_.is_probe -eq $grp }
  if (-not $g.Count) { continue }
  $label = if ($grp) { "probe frames  " } else { "science frames" }
  $gp = ($g | Where-Object { $_.psolve_solved }).Count
  $ga = ($g | Where-Object { $_.astap_solved }).Count
  Write-Host ("  " + $label + " n=" + $g.Count + "  psolve " + $gp + " (" + [math]::Round(100*$gp/$g.Count,0) + "%)  astap " + $ga + " (" + [math]::Round(100*$ga/$g.Count,0) + "%)")
}
$results | Where-Object { -not $_.psolve_solved } | Group-Object psolve_reason |
  ForEach-Object { Write-Host ("psolve failures   : " + $_.Name + " x" + $_.Count) }
