<#
.SYNOPSIS
    Tags a new Ferrite release and pushes it to GitHub, triggering the release workflow.

.DESCRIPTION
    1. Verifies the working tree is clean.
    2. Bumps the version in all workspace Cargo.toml files.
    3. Creates an annotated git tag (v<version>).
    4. Pushes the tag to origin, which triggers .github/workflows/release.yml.

.PARAMETER Version
    The release version in semver format, e.g. "0.3.0" (without a leading "v").

.EXAMPLE
    .\scripts\release\tag-release.ps1 -Version 0.3.0
#>
param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^\d+\.\d+\.\d+$')]
    [string]$Version
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..")
Set-Location $RepoRoot

# ── 1. Working tree must be clean ─────────────────────────────────────────
$status = git status --porcelain
if ($status) {
    Write-Error "Working tree is not clean. Commit or stash your changes first."
    exit 1
}

# ── 2. Bump version in workspace Cargo.toml ───────────────────────────────
$workspaceToml = Join-Path $RepoRoot "Cargo.toml"
$content = Get-Content $workspaceToml -Raw

# Match the version line inside [workspace.package]
if ($content -notmatch 'version\s*=\s*"(\d+\.\d+\.\d+)"') {
    Write-Error "Could not find version in $workspaceToml"
    exit 1
}
$currentVersion = $Matches[1]
if ($currentVersion -eq $Version) {
    Write-Host "Version is already $Version — skipping Cargo.toml update."
} else {
    Write-Host "Bumping version $currentVersion -> $Version"
    $updated = $content -replace "version\s*=\s*`"$currentVersion`"", "version     = `"$Version`""
    Set-Content -Path $workspaceToml -Value $updated -NoNewline

    # Regenerate Cargo.lock so it reflects the new version.
    cargo generate-lockfile | Out-Null

    git add Cargo.toml Cargo.lock
    git commit -m "chore: bump version to $Version"
}

# ── 3. Create annotated tag ───────────────────────────────────────────────
$tag = "v$Version"
Write-Host "Creating tag $tag"
git tag -a $tag -m "Release $tag"

# ── 4. Push commit + tag ──────────────────────────────────────────────────
Write-Host "Pushing to origin..."
git push origin master
git push origin $tag

Write-Host ""
Write-Host "Done! GitHub Actions will now build and publish the release."
Write-Host "Track progress at: https://github.com/arozz7/ferrite/actions"
