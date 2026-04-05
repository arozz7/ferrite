<#
.SYNOPSIS
    Analyze a Ferrite carving output directory by file extension.

.DESCRIPTION
    Groups all carved files by extension and reports:
      - Count, total size, average size, min size, max size per type
      - Top N largest individual files overall
      - Any files that look suspiciously large for their type

.PARAMETER OutputDir
    Path to the carving output directory (e.g. M:\carved\carving-4TB).

.PARAMETER TopFiles
    How many largest individual files to list at the end. Default 20.

.PARAMETER ThresholdMiB
    Files larger than this (MiB) are flagged in the per-extension table. Default 500.

.EXAMPLE
    .\analyze_carved.ps1 -OutputDir M:\carved\carving-4TB
    .\analyze_carved.ps1 -OutputDir M:\carved\carving-4TB -TopFiles 30 -ThresholdMiB 200
#>

param(
    [Parameter(Mandatory)]
    [string]$OutputDir,

    [int]$TopFiles = 20,

    [double]$ThresholdMiB = 500
)

$ErrorActionPreference = 'Stop'

if (-not (Test-Path $OutputDir)) {
    Write-Error "Directory not found: $OutputDir"
    exit 1
}

# ── Helper ────────────────────────────────────────────────────────────────────

function fmt([long]$bytes) {
    if ($bytes -ge 1GB) { return "{0:F2} GiB" -f ($bytes / 1GB) }
    if ($bytes -ge 1MB) { return "{0:F1} MiB" -f ($bytes / 1MB) }
    if ($bytes -ge 1KB) { return "{0:F1} KiB" -f ($bytes / 1KB) }
    return "$bytes B"
}

Write-Host "`nScanning $OutputDir ..." -ForegroundColor Cyan
$all = Get-ChildItem -Path $OutputDir -Recurse -File

if ($all.Count -eq 0) {
    Write-Host "No files found." -ForegroundColor Yellow
    exit 0
}

$totalBytes = ($all | Measure-Object Length -Sum).Sum
$thresholdBytes = $ThresholdMiB * 1MB

Write-Host ("Found {0:N0} files  |  Total: {1}" -f $all.Count, (fmt $totalBytes))

# ── Group by extension ────────────────────────────────────────────────────────

$groups = $all | Group-Object Extension | ForEach-Object {
    $ext   = if ($_.Name) { $_.Name.TrimStart('.').ToLower() } else { '(none)' }
    $sizes = $_.Group | ForEach-Object { $_.Length }
    $sum   = ($sizes | Measure-Object -Sum).Sum
    $avg   = [long]($sum / $sizes.Count)
    $max   = ($sizes | Measure-Object -Maximum).Maximum
    $min   = ($sizes | Measure-Object -Minimum).Minimum
    [PSCustomObject]@{
        Ext       = $ext
        Count     = $_.Count
        TotalB    = $sum
        AvgB      = $avg
        MaxB      = $max
        MinB      = $min
        PctTotal  = [math]::Round($sum / $totalBytes * 100, 1)
        Flagged   = ($max -gt $thresholdBytes)
    }
} | Sort-Object TotalB -Descending

# ── Print per-extension table ─────────────────────────────────────────────────

Write-Host "`n── By Extension (sorted by total size) ─────────────────────────────────────────" -ForegroundColor Cyan
Write-Host ("{0,-8}  {1,6}  {2,10}  {3,7}%  {4,10}  {5,10}  {6,10}  {7}" -f
    "Ext", "Count", "Total", "Total", "Avg", "Max", "Min", "Flag")
Write-Host ("{0,-8}  {1,6}  {2,10}  {3,7}  {4,10}  {5,10}  {6,10}  {7}" -f
    "---", "-----", "-----", "-----", "---", "---", "---", "----")

foreach ($g in $groups) {
    $flag = if ($g.Flagged) { "<< LARGE" } else { "" }
    $color = if ($g.Flagged) { 'Yellow' } else { 'White' }
    Write-Host ("{0,-8}  {1,6}  {2,10}  {3,6}%  {4,10}  {5,10}  {6,10}  {7}" -f
        $g.Ext,
        $g.Count,
        (fmt $g.TotalB),
        $g.PctTotal,
        (fmt $g.AvgB),
        (fmt $g.MaxB),
        (fmt $g.MinB),
        $flag
    ) -ForegroundColor $color
}

# ── Top N largest individual files ───────────────────────────────────────────

Write-Host "`n── Top $TopFiles Largest Files ──────────────────────────────────────────────────────" -ForegroundColor Cyan
$all | Sort-Object Length -Descending | Select-Object -First $TopFiles | ForEach-Object {
    $rel = $_.FullName.Substring($OutputDir.TrimEnd('\').Length + 1)
    Write-Host ("{0,10}  {1}" -f (fmt $_.Length), $rel) -ForegroundColor $(
        if ($_.Length -gt $thresholdBytes) { 'Yellow' } else { 'White' }
    )
}

# ── Files above threshold, grouped by extension ───────────────────────────────

$large = $all | Where-Object { $_.Length -gt $thresholdBytes } | Sort-Object Length -Descending
if ($large.Count -gt 0) {
    Write-Host "`n── Files > $(fmt $thresholdBytes) ($($large.Count) files) ──────────────────────────────────────" -ForegroundColor Yellow
    $large | Group-Object Extension | Sort-Object { ($_.Group | Measure-Object Length -Sum).Sum } -Descending | ForEach-Object {
        $ext = if ($_.Name) { $_.Name } else { '(none)' }
        $sz  = ($_.Group | Measure-Object Length -Sum).Sum
        Write-Host ("  {0,-8}  {1,4} files  total {2}" -f $ext, $_.Count, (fmt $sz)) -ForegroundColor Yellow
    }
}

Write-Host "`nDone.`n" -ForegroundColor Cyan
