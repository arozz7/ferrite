#Requires -Version 5.1
<#
.SYNOPSIS
    SHA-256 hash comparison between carved output and source drives.

.DESCRIPTION
    Stage 1: Hash every file on SourceDrives and build a lookup table
             SHA256 -> [SourcePath, Size].  Saved to hash_cache.json so
             it can be reused on subsequent runs.

    Stage 2: Hash every fallback-named carved file (ferrite_*) and the
             size-mismatch group, then look each up in the source table.

    Results:
      hash_matched.csv     - exact hash hit: file recovered intact
      hash_corrupt.csv     - name/path match exists but hash differs (damaged)
      hash_unmatched.csv   - no source hash match (third partition or total loss)
      hash_mismatch_check.csv - re-evaluation of the 252 size-mismatch files

.PARAMETER CarveDir
    Root of the Ferrite carve output.  Default: O:\Carved\carving-2tb

.PARAMETER SourceDrives
    Source drive roots.  Default: E:\, F:\

.PARAMETER CacheFile
    Where to save/load the source hash cache.  Default: <CarveDir>\hash_cache.json

.PARAMETER SkipCacheBuild
    If the cache file already exists and this switch is set, skip re-hashing sources.
#>
param(
    [string]$CarveDir     = 'O:\Carved\carving-2tb',
    [string[]]$SourceDrives = @('E:\', 'F:\'),
    [string]$CacheFile    = '',
    [switch]$SkipCacheBuild
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ($CacheFile -eq '') { $CacheFile = Join-Path $CarveDir 'hash_cache.json' }

function Write-Status([string]$msg) {
    Write-Host "$(Get-Date -Format 'HH:mm:ss')  $msg"
}

function Get-SHA256([string]$path) {
    try {
        $sha = [System.Security.Cryptography.SHA256]::Create()
        $stream = [System.IO.File]::OpenRead($path)
        try {
            $bytes = $sha.ComputeHash($stream)
            return [System.BitConverter]::ToString($bytes).Replace('-','').ToLowerInvariant()
        } finally {
            $stream.Dispose()
            $sha.Dispose()
        }
    } catch {
        return $null
    }
}

# -- Stage 1: Build or load source hash cache -----------------------------------

$sourceByHash = @{}   # SHA256 -> list of @{Path; Size}
$sourceByName = @{}   # lower basename -> list of @{Path; Size; Hash}

if ($SkipCacheBuild -and (Test-Path $CacheFile)) {
    Write-Status "Loading source hash cache from $CacheFile ..."
    $cached = Get-Content $CacheFile -Raw | ConvertFrom-Json
    foreach ($entry in $cached) {
        $h = $entry.Hash
        $n = [System.IO.Path]::GetFileName($entry.Path).ToLowerInvariant()
        if (-not $sourceByHash.ContainsKey($h)) { $sourceByHash[$h] = [System.Collections.Generic.List[psobject]]::new() }
        $sourceByHash[$h].Add([pscustomobject]@{ Path = $entry.Path; Size = $entry.Size })
        if (-not $sourceByName.ContainsKey($n))  { $sourceByName[$n]  = [System.Collections.Generic.List[psobject]]::new() }
        $sourceByName[$n].Add([pscustomobject]@{ Path = $entry.Path; Size = $entry.Size; Hash = $h })
    }
    Write-Status "Cache loaded: $($sourceByHash.Count) distinct hashes"
} else {
    Write-Status "Hashing source drives: $($SourceDrives -join ', ') ..."
    $allSource = foreach ($root in $SourceDrives) {
        if (-not (Test-Path $root)) { Write-Warning "Not accessible: $root"; continue }
        Get-ChildItem $root -Recurse -File -ErrorAction SilentlyContinue
    }
    $total = @($allSource).Count
    Write-Status "  $total source files to hash ..."

    $cacheList = [System.Collections.Generic.List[psobject]]::new()
    $done = 0
    foreach ($f in $allSource) {
        $done++
        if ($done % 1000 -eq 0) { Write-Status "  hashed $done / $total ..." }
        $h = Get-SHA256 $f.FullName
        if ($null -eq $h) { continue }
        $n = $f.Name.ToLowerInvariant()

        if (-not $sourceByHash.ContainsKey($h)) { $sourceByHash[$h] = [System.Collections.Generic.List[psobject]]::new() }
        $sourceByHash[$h].Add([pscustomobject]@{ Path = $f.FullName; Size = $f.Length })
        if (-not $sourceByName.ContainsKey($n))  { $sourceByName[$n]  = [System.Collections.Generic.List[psobject]]::new() }
        $sourceByName[$n].Add([pscustomobject]@{ Path = $f.FullName; Size = $f.Length; Hash = $h })

        $cacheList.Add([pscustomobject]@{ Path = $f.FullName; Size = $f.Length; Hash = $h })
    }

    Write-Status "Saving cache to $CacheFile ..."
    try {
        $cacheList | ConvertTo-Json -Compress | Set-Content $CacheFile -Encoding UTF8
        Write-Status "Cache saved."
    } catch {
        Write-Warning "Could not save cache (file in use?): $_  -- continuing without cache."
    }
    Write-Status "Source hashing complete: $($sourceByHash.Count) distinct hashes from $done files"
}

# -- Stage 2: Hash carved files -------------------------------------------------

# 2a: Fallback-named files  (ferrite_<ext>_<offset>.<ext>)
Write-Status "Scanning carved output for fallback-named files ..."
$fallback = Get-ChildItem $CarveDir -Recurse -File -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -match '^ferrite_[a-zA-Z0-9]+_\d+\.' }
Write-Status "  Fallback-named files to hash: $($fallback.Count)"

# 2b: Size-mismatch files from previous run
$mismatchCsv = Join-Path $CarveDir 'size_mismatch.csv'
$mismatchFiles = @()
if (Test-Path $mismatchCsv) {
    $mismatchFiles = Import-Csv $mismatchCsv
    Write-Status "  Size-mismatch files to re-check: $($mismatchFiles.Count)"
}

# -- Stage 3: Match fallback files ----------------------------------------------

Write-Status "Matching fallback-named carved files against source hashes ..."

$hashMatched   = [System.Collections.Generic.List[psobject]]::new()
$hashUnmatched = [System.Collections.Generic.List[psobject]]::new()
$done = 0
$total = $fallback.Count

foreach ($cf in $fallback) {
    $done++
    if ($done % 500 -eq 0) { Write-Status "  $done / $total ..." }

    $h = Get-SHA256 $cf.FullName
    if ($null -eq $h) {
        $hashUnmatched.Add([pscustomobject]@{
            CarvedPath = $cf.FullName; CarvedSize = $cf.Length; Reason = 'read_error'; Hash = ''
        })
        continue
    }

    if ($sourceByHash.ContainsKey($h)) {
        $src = $sourceByHash[$h][0]
        $hashMatched.Add([pscustomobject]@{
            CarvedPath = $cf.FullName; CarvedSize = $cf.Length
            SourcePath = $src.Path;   SourceSize = $src.Size
            Hash = $h
        })
    } else {
        $hashUnmatched.Add([pscustomobject]@{
            CarvedPath = $cf.FullName; CarvedSize = $cf.Length
            Reason = 'no_source_match'; Hash = $h
        })
    }
}

# -- Stage 4: Re-check size-mismatch files --------------------------------------

Write-Status "Re-checking size-mismatch files with hashes ..."

$mismatchCheck = [System.Collections.Generic.List[psobject]]::new()
foreach ($row in $mismatchFiles) {
    $carvedH = Get-SHA256 $row.CarvedPath
    $sourceH = if (Test-Path $row.SourcePath) { Get-SHA256 $row.SourcePath } else { $null }
    $verdict = switch ($true) {
        ($null -eq $carvedH)            { 'carved_unreadable' }
        ($null -eq $sourceH)            { 'source_missing' }
        ($carvedH -eq $sourceH)         { 'identical' }
        ($sourceByHash.ContainsKey($carvedH)) {
            $altSrc = $sourceByHash[$carvedH][0].Path; "hash_match_alt_source: $altSrc"
        }
        default                         { 'damaged' }
    }
    $mismatchCheck.Add([pscustomobject]@{
        CarvedPath  = $row.CarvedPath
        CarvedSize  = $row.CarvedSize
        SourcePath  = $row.SourcePath
        SourceSize  = $row.SourceSize
        SizeDelta   = $row.SizeDelta
        CarvedHash  = if ($carvedH) { $carvedH } else { '' }
        SourceHash  = if ($sourceH) { $sourceH } else { '' }
        Verdict     = $verdict
    })
}

# -- Stage 5: Write reports -----------------------------------------------------

$hashMatchedPath    = Join-Path $CarveDir 'hash_matched.csv'
$hashUnmatchedPath  = Join-Path $CarveDir 'hash_unmatched.csv'
$mismatchCheckPath  = Join-Path $CarveDir 'hash_mismatch_check.csv'

$hashMatched    | Export-Csv $hashMatchedPath   -NoTypeInformation -Encoding UTF8
$hashUnmatched  | Export-Csv $hashUnmatchedPath -NoTypeInformation -Encoding UTF8
$mismatchCheck  | Export-Csv $mismatchCheckPath -NoTypeInformation -Encoding UTF8

# -- Stage 6: Summary -----------------------------------------------------------

$damaged    = @($mismatchCheck | Where-Object { $_.Verdict -eq 'damaged' }).Count
$identical  = @($mismatchCheck | Where-Object { $_.Verdict -eq 'identical' }).Count
$altMatch   = @($mismatchCheck | Where-Object { $_.Verdict -like 'hash_match_alt*' }).Count

Write-Host ''
Write-Host '======================================================' -ForegroundColor Cyan
Write-Host '  Hash Comparison Report' -ForegroundColor Cyan
Write-Host '======================================================' -ForegroundColor Cyan
Write-Host ''
Write-Host "  Fallback-named carved files     : $($fallback.Count)"
Write-Host "    Hash-matched to source        : $($hashMatched.Count)" -ForegroundColor Green
Write-Host "    No source match (unrecovered) : $($hashUnmatched.Count)" -ForegroundColor Red
Write-Host ''
Write-Host "  Size-mismatch files re-checked  : $($mismatchFiles.Count)"
Write-Host "    Identical content (false alarm): $identical" -ForegroundColor Green
Write-Host "    Alt-source match              : $altMatch" -ForegroundColor Yellow
Write-Host "    Genuinely damaged             : $damaged" -ForegroundColor Red
Write-Host "    Other                         : $($mismatchCheck.Count - $identical - $altMatch - $damaged)" -ForegroundColor DarkGray
Write-Host ''

$totalCarved = $fallback.Count + ($mismatchFiles.Count)
$totalMatched = $hashMatched.Count + $identical + $altMatch
if ($totalCarved -gt 0) {
    $pct = [Math]::Round($totalMatched / $totalCarved * 100, 1)
    Write-Host "  Content recovery rate         : $pct %" -ForegroundColor Cyan
}
Write-Host ''
Write-Host '  Reports written:' -ForegroundColor Cyan
Write-Host "    $hashMatchedPath"
Write-Host "    $hashUnmatchedPath"
Write-Host "    $mismatchCheckPath"
Write-Host ''

# Breakdown of unmatched by extension
if ($hashUnmatched.Count -gt 0) {
    Write-Host '  Unmatched fallback files by extension:' -ForegroundColor DarkGray
    $hashUnmatched |
        ForEach-Object { [System.IO.Path]::GetExtension($_.CarvedPath).ToLowerInvariant() } |
        Group-Object | Sort-Object Count -Descending | Select-Object -First 15 |
        ForEach-Object { Write-Host "    $($_.Name.PadRight(10)) $($_.Count)" -ForegroundColor DarkGray }
}
