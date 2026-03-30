#Requires -Version 5.1
<#
.SYNOPSIS
    Compares a Ferrite carve output directory against source drives.

.DESCRIPTION
    Builds an index of all files on the source drives (by filename, case-insensitive),
    then matches each carved file against it.  Produces four output files:

      matched.csv       - carved file whose name + size exactly match a source file
      size_mismatch.csv - carved file whose name matches but size differs (possible truncation)
      unrecognised.csv  - carved file with no name match (fallback-named or from a 3rd source)
      not_recovered.csv - source file whose name never appears in the carve output

    Run time: O(N) lookups using hashtables - typically completes in < 2 minutes
    for 30 000 carved files against 40 000 source files.

.PARAMETER CarveDir
    Root of the Ferrite carve output.  Default: O:\Carved\carving-2tb

.PARAMETER SourceDrives
    One or more drive roots to treat as ground-truth source.  Default: E:\, F:\

.PARAMETER OutDir
    Directory to write the four CSV reports.  Default: same as CarveDir.
#>
param(
    [string]$CarveDir    = 'O:\Carved\carving-2tb',
    [string[]]$SourceDrives = @('E:\', 'F:\'),
    [string]$OutDir      = ''
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ($OutDir -eq '') { $OutDir = $CarveDir }
if (-not (Test-Path $OutDir)) { New-Item -ItemType Directory -Path $OutDir | Out-Null }

# -- Helper --------------------------------------------------------------------

function Write-Status([string]$msg) {
    Write-Host "$(Get-Date -Format 'HH:mm:ss')  $msg"
}

# -- 1. Index source drives -----------------------------------------------------

Write-Status "Indexing source drives: $($SourceDrives -join ', ') ..."

# Key  : lower-case basename  (e.g. "photo.jpg")
# Value: list of [FullPath, Length] - there may be duplicates across drives
$sourceIndex = @{}   # [string] -> [PSCustomObject[]]

foreach ($root in $SourceDrives) {
    if (-not (Test-Path $root)) {
        Write-Warning "Source drive not accessible: $root - skipping"
        continue
    }
    $files = Get-ChildItem $root -Recurse -File -ErrorAction SilentlyContinue
    Write-Status "  $root : $($files.Count) files"
    foreach ($f in $files) {
        $key = $f.Name.ToLowerInvariant()
        if (-not $sourceIndex.ContainsKey($key)) {
            $sourceIndex[$key] = [System.Collections.Generic.List[psobject]]::new()
        }
        $sourceIndex[$key].Add([pscustomobject]@{
            SourcePath = $f.FullName
            SourceSize = $f.Length
            Drive      = $root
            Matched    = $false   # will be set when a carved file matches
        })
    }
}
Write-Status "Source index: $($sourceIndex.Count) distinct filenames across $(@($SourceDrives | Where-Object { Test-Path $_ }).Count) drives"

# -- 2. Scan carved output ------------------------------------------------------

Write-Status "Scanning carved output: $CarveDir ..."
$carvedFiles = Get-ChildItem $CarveDir -Recurse -File -ErrorAction SilentlyContinue
Write-Status "Carved files: $($carvedFiles.Count)"

$matched      = [System.Collections.Generic.List[psobject]]::new()
$sizeMismatch = [System.Collections.Generic.List[psobject]]::new()
$unrecognised = [System.Collections.Generic.List[psobject]]::new()

foreach ($cf in $carvedFiles) {
    $key = $cf.Name.ToLowerInvariant()
    if ($sourceIndex.ContainsKey($key)) {
        $candidates = $sourceIndex[$key]
        # Find the best candidate: prefer exact size match
        $exact = $candidates | Where-Object { $_.SourceSize -eq $cf.Length } | Select-Object -First 1
        if ($exact) {
            $exact.Matched = $true
            $matched.Add([pscustomobject]@{
                CarvedPath  = $cf.FullName
                CarvedSize  = $cf.Length
                SourcePath  = $exact.SourcePath
                SourceSize  = $exact.SourceSize
                Drive       = $exact.Drive
            })
        } else {
            # Name matches but size differs - flag every candidate
            foreach ($c in $candidates) { $c.Matched = $true }
            $best = $candidates | Sort-Object { [Math]::Abs($_.SourceSize - $cf.Length) } | Select-Object -First 1
            $sizeMismatch.Add([pscustomobject]@{
                CarvedPath  = $cf.FullName
                CarvedSize  = $cf.Length
                SourcePath  = $best.SourcePath
                SourceSize  = $best.SourceSize
                Drive       = $best.Drive
                SizeDelta   = $cf.Length - $best.SourceSize
            })
        }
    } else {
        $unrecognised.Add([pscustomobject]@{
            CarvedPath = $cf.FullName
            CarvedSize = $cf.Length
            Extension  = $cf.Extension.ToLowerInvariant()
        })
    }
}

# -- 3. Find source files never recovered --------------------------------------

$notRecovered = [System.Collections.Generic.List[psobject]]::new()
foreach ($key in $sourceIndex.Keys) {
    foreach ($entry in $sourceIndex[$key]) {
        if (-not $entry.Matched) {
            $notRecovered.Add([pscustomobject]@{
                SourcePath = $entry.SourcePath
                SourceSize = $entry.SourceSize
                Drive      = $entry.Drive
            })
        }
    }
}

# -- 4. Write CSV reports -------------------------------------------------------

$matchedPath      = Join-Path $OutDir 'matched.csv'
$mismatchPath     = Join-Path $OutDir 'size_mismatch.csv'
$unrecognisedPath = Join-Path $OutDir 'unrecognised.csv'
$notRecoveredPath = Join-Path $OutDir 'not_recovered.csv'

$matched      | Export-Csv $matchedPath      -NoTypeInformation -Encoding UTF8
$sizeMismatch | Export-Csv $mismatchPath     -NoTypeInformation -Encoding UTF8
$unrecognised | Export-Csv $unrecognisedPath -NoTypeInformation -Encoding UTF8
$notRecovered | Export-Csv $notRecoveredPath -NoTypeInformation -Encoding UTF8

# -- 5. Summary -----------------------------------------------------------------

$totalSource  = ($sourceIndex.Values | ForEach-Object { $_.Count } | Measure-Object -Sum).Sum
$recoveryPct  = if ($totalSource -gt 0) { [Math]::Round(($matched.Count + $sizeMismatch.Count) / $totalSource * 100, 1) } else { 0 }

Write-Host ''
Write-Host '======================================================' -ForegroundColor Cyan
Write-Host '  Carve vs Source Comparison Report' -ForegroundColor Cyan
Write-Host '======================================================' -ForegroundColor Cyan
Write-Host ''
Write-Host "  Source files total      : $totalSource"
Write-Host "  Carved files total      : $($carvedFiles.Count)"
Write-Host ''
Write-Host "  OK Exact match (name+size): $($matched.Count)" -ForegroundColor Green
Write-Host "  ~ Name match, size diff  : $($sizeMismatch.Count)" -ForegroundColor Yellow
Write-Host "  ? Unrecognised (no match): $($unrecognised.Count)" -ForegroundColor DarkGray
Write-Host "  X Not recovered          : $($notRecovered.Count)" -ForegroundColor Red
Write-Host ''
Write-Host "  Recovery rate (name hit) : $recoveryPct %" -ForegroundColor Cyan
Write-Host ''
Write-Host '  Reports written to:' -ForegroundColor Cyan
Write-Host "    $matchedPath"
Write-Host "    $mismatchPath"
Write-Host "    $unrecognisedPath"
Write-Host "    $notRecoveredPath"
Write-Host ''

# -- 6. Extension breakdown for unrecognised carved files ----------------------

if ($unrecognised.Count -gt 0) {
    Write-Host '  Unrecognised carved files by extension:' -ForegroundColor DarkGray
    $unrecognised | Group-Object Extension | Sort-Object Count -Descending | Select-Object -First 20 |
        ForEach-Object { Write-Host "    $($_.Name.PadRight(10)) $($_.Count)" -ForegroundColor DarkGray }
    Write-Host ''
}

# -- 7. Not-recovered breakdown by drive ---------------------------------------

if ($notRecovered.Count -gt 0) {
    Write-Host '  Not-recovered source files by drive:' -ForegroundColor Red
    $notRecovered | Group-Object Drive | Sort-Object Count -Descending |
        ForEach-Object { Write-Host "    $($_.Name.PadRight(6)) $($_.Count)" -ForegroundColor Red }
    Write-Host ''
}
