<#
.SYNOPSIS
Builds signed x64 and arm64 MSIX packages of Trayulator and drafts a GitHub release with them.

.DESCRIPTION
Each architecture is built with cargo, staged as a package layout (the exe and Assets) and packed
with `winapp package`. The packages are signed with the maintainer's `sign-msix` profile function,
then attached to a draft release tagged from the version in Package.appxmanifest, which must match
Cargo.toml (1.2.3 in Cargo.toml is 1.2.3.0 in the manifest).

.EXAMPLE
.\build-msix-for-gh.ps1                  # build, sign, draft the release
.\build-msix-for-gh.ps1 -SkipRelease     # build and sign only
#>
[CmdletBinding()]
param(
    [ValidateSet('Release', 'Debug')]
    [string]$Configuration = 'Release',

    # Root of the repository to build. Defaults to the folder this script lives in.
    [string]$RepoRoot,

    # Release notes for the GitHub release. When omitted, GitHub generates notes
    # from the commits/PRs since the previous release.
    [string]$Notes,

    # Mark the GitHub release as a pre-release.
    [switch]$Prerelease,

    # Build and sign only; skip tagging and the GitHub release.
    [switch]$SkipRelease,

    # Allow releasing with uncommitted changes (they would be in the build but not in the tag).
    [switch]$AllowDirty
)

$ErrorActionPreference = 'Stop'

# The repo root holds both the crate and the package manifest.
function Test-RepoRoot {
    param(
        [Parameter(Mandatory)]
        [string]$Root
    )

    (Test-Path -LiteralPath (Join-Path $Root 'Cargo.toml')) -and
        (Test-Path -LiteralPath (Join-Path $Root 'Package.appxmanifest'))
}

# $PSScriptRoot is not reliable in every host (e.g. ISE/VS Code run-selection or
# dot-sourcing report the caller's folder), so resolve the root explicitly: an
# explicit -RepoRoot wins, then the script's own folder, then the git repo of the
# current directory.
if (-not $RepoRoot) {
    $candidates = @(
        $(if ($MyInvocation.MyCommand.Path) { Split-Path -Parent $MyInvocation.MyCommand.Path }),
        $PSScriptRoot,
        $(& git rev-parse --show-toplevel 2>$null),
        (Get-Location).ProviderPath
    ) | Where-Object { $_ }

    $RepoRoot = $candidates |
        Where-Object { Test-RepoRoot -Root $_ } |
        Select-Object -First 1

    if (-not $RepoRoot) {
        throw "Could not locate the repository (a folder with Cargo.toml and Package.appxmanifest; tried: $($candidates -join ', ')). Pass -RepoRoot or run from inside the repo."
    }
}

if (-not (Test-Path -LiteralPath $RepoRoot)) {
    throw "Repository root not found: $RepoRoot"
}

$repoRoot = (Resolve-Path -LiteralPath $RepoRoot).ProviderPath

$appName = 'Trayulator'
$exeName = 'trayulator.exe'
$manifestPath = Join-Path $repoRoot 'Package.appxmanifest'
$assetsPath = Join-Path $repoRoot 'Assets'
$targetRoot = Join-Path $repoRoot 'target'
$outputRoot = Join-Path $repoRoot 'AppPackages\GitHub'

# cargo's profile name, and the folder under target\<triple> it builds into.
$cargoProfile = if ($Configuration -eq 'Release') { 'release' } else { 'dev' }
$profileDir = if ($Configuration -eq 'Release') { 'release' } else { 'debug' }

# Signing is done by the maintainer's local 'sign-msix' profile function, so the
# signing configuration stays out of the repo.
$signCommand = 'sign-msix'

$builds = @(
    @{
        Name = 'x64'
        Target = 'x86_64-pc-windows-msvc'
        LayoutDir = (Join-Path $targetRoot 'msix\x64')
    },
    @{
        Name = 'arm64'
        Target = 'aarch64-pc-windows-msvc'
        LayoutDir = (Join-Path $targetRoot 'msix\arm64')
    }
)

function Get-ReleaseTag {
    param(
        [Parameter(Mandatory)]
        [string]$Version
    )

    # 0.1.0.0 -> v0.1.0 (matches existing tags); keep a non-zero fourth part.
    $parts = $Version.Split('.')
    if ($parts.Count -eq 4 -and $parts[3] -eq '0') {
        $parts = $parts[0..2]
    }
    return 'v' + ($parts -join '.')
}

function Assert-ReleaseReady {
    param(
        [Parameter(Mandatory)]
        [string]$Tag
    )

    foreach ($tool in 'git', 'gh') {
        if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
            throw "$tool is required but was not found on PATH."
        }
    }

    & gh auth status *> $null
    if ($LASTEXITCODE -ne 0) {
        throw 'gh is not authenticated. Run "gh auth login" first.'
    }

    $branch = (& git -C $repoRoot rev-parse --abbrev-ref HEAD).Trim()
    if ($LASTEXITCODE -ne 0 -or $branch -eq 'HEAD') {
        throw 'Could not determine the current branch (detached HEAD?).'
    }

    if (-not $AllowDirty) {
        $status = & git -C $repoRoot status --porcelain
        if ($status) {
            throw "Working tree has uncommitted changes; commit/stash them or pass -AllowDirty.`n$($status -join "`n")"
        }
    }

    & git -C $repoRoot fetch origin --quiet
    if ($LASTEXITCODE -ne 0) {
        throw 'git fetch origin failed.'
    }

    $upstream = & git -C $repoRoot rev-parse --abbrev-ref '@{u}' 2>$null
    if ($LASTEXITCODE -ne 0) {
        throw "Branch '$branch' has no upstream. Push it first (git push -u origin $branch)."
    }
    $ahead = [int](& git -C $repoRoot rev-list --count '@{u}..HEAD')
    if ($ahead -gt 0) {
        throw "Branch '$branch' is $ahead commit(s) ahead of $upstream. Push before releasing."
    }

    & git -C $repoRoot rev-parse -q --verify "refs/tags/$Tag" *> $null
    if ($LASTEXITCODE -eq 0) {
        throw "Tag $Tag already exists locally. Bump the version in Cargo.toml and Package.appxmanifest."
    }
    $remoteTag = & git -C $repoRoot ls-remote --tags origin "refs/tags/$Tag"
    if ($remoteTag) {
        throw "Tag $Tag already exists on origin. Bump the version in Cargo.toml and Package.appxmanifest."
    }

    # Drafts have no tag on origin yet, so also check for an existing (draft) release.
    Push-Location -LiteralPath $repoRoot
    try {
        # Filter in PowerShell rather than --jq: Windows PowerShell 5.1 strips embedded quotes from native args.
        $releasesJson = & gh release list --limit 100 --json tagName
        if ($LASTEXITCODE -ne 0) {
            throw 'gh release list failed.'
        }
    }
    finally {
        Pop-Location
    }
    $existingRelease = ($releasesJson | Out-String | ConvertFrom-Json) | Where-Object { $_.tagName -eq $Tag }
    if ($existingRelease) {
        throw "A release for $Tag already exists on GitHub (possibly a draft). Delete it or bump the version."
    }

    return $branch
}

function Invoke-GitHubMsixBuild {
    param(
        [Parameter(Mandatory)]
        [hashtable]$Build,

        [Parameter(Mandatory)]
        [string]$OutputFile
    )

    Write-Host "Building GitHub MSIX for $($Build.Name)..."
    & cargo build --manifest-path (Join-Path $repoRoot 'Cargo.toml') --profile $cargoProfile --target $Build.Target --locked
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build failed for $($Build.Name)."
    }

    # winapp packs the whole folder it's given, so stage just the exe and Assets.
    if (Test-Path -LiteralPath $Build.LayoutDir) {
        Remove-Item -LiteralPath $Build.LayoutDir -Recurse -Force
    }
    New-Item -ItemType Directory -Path $Build.LayoutDir -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $targetRoot "$($Build.Target)\$profileDir\$exeName") -Destination $Build.LayoutDir
    Copy-Item -LiteralPath $assetsPath -Destination $Build.LayoutDir -Recurse

    # Unsigned here; the packages are signed together once both are built.
    & winapp package $Build.LayoutDir --manifest $manifestPath --output $OutputFile --no-sign
    if ($LASTEXITCODE -ne 0) {
        throw "winapp package failed for $($Build.Name)."
    }
}

function Invoke-MsixSigning {
    param(
        [Parameter(Mandatory)]
        [System.IO.FileInfo[]]$Files
    )

    & $signCommand -Path $Files.FullName

    # Don't rely on the signing command's error reporting: confirm every package is signed.
    foreach ($file in $Files) {
        $signature = Get-AuthenticodeSignature -LiteralPath $file.FullName
        if ($signature.Status -ne 'Valid') {
            throw "$($file.Name) is not validly signed (status: $($signature.Status))."
        }
        Write-Host "  Verified signature on $($file.Name): $($signature.SignerCertificate.Subject)"
    }
}

$scriptFailed = $false
try {
    foreach ($tool in 'cargo', 'rustup', 'winapp') {
        if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
            throw "$tool is required but was not found on PATH."
        }
    }

    foreach ($requiredFile in $manifestPath, $assetsPath) {
        if (-not (Test-Path -LiteralPath $requiredFile)) {
            throw "Required file not found: $requiredFile"
        }
    }

    # A fresh toolchain only has the host target.
    $installedTargets = @(& rustup target list --installed)
    foreach ($build in $builds) {
        if ($installedTargets -notcontains $build.Target) {
            throw "Rust target $($build.Target) is not installed. Run: rustup target add $($build.Target)"
        }
    }

    if (-not (Get-Command $signCommand -ErrorAction SilentlyContinue)) {
        throw "'$signCommand' was not found. It is defined in the maintainer's PowerShell profile; run this script from a pwsh session that loaded it."
    }

    $manifestContent = [System.IO.File]::ReadAllText($manifestPath)
    $versionMatch = [regex]::Match($manifestContent, '(?s)<Identity\b[^>]*?\bVersion="([^"]+)"')
    if (-not $versionMatch.Success) {
        throw "Could not find Version attribute on <Identity> in $manifestPath."
    }
    $version = $versionMatch.Groups[1].Value

    # build.rs stamps the crate version into the exe, so it must agree with the package version.
    $metadataJson = & cargo metadata --manifest-path (Join-Path $repoRoot 'Cargo.toml') --no-deps --format-version 1
    if ($LASTEXITCODE -ne 0) {
        throw 'cargo metadata failed.'
    }
    $crateVersion = ($metadataJson | Out-String | ConvertFrom-Json).packages[0].version
    if ($version -ne "$crateVersion.0") {
        throw "Package.appxmanifest has version $version but Cargo.toml has $crateVersion. Set the manifest to $crateVersion.0 (or bump both)."
    }

    $tag = Get-ReleaseTag -Version $version
    $versionedOutputDir = Join-Path $outputRoot "$version-gh"

    Write-Host "Project: $appName ($repoRoot), version $version"

    # Check git/gh state before the (slow) build so problems surface immediately.
    if (-not $SkipRelease) {
        $branch = Assert-ReleaseReady -Tag $tag
        Write-Host "Releasing $tag from branch '$branch'."
    }

    if (Test-Path -LiteralPath $versionedOutputDir) {
        Get-ChildItem -LiteralPath $versionedOutputDir -Filter '*.msix' -File -ErrorAction SilentlyContinue |
            Remove-Item -Force
    }
    else {
        New-Item -ItemType Directory -Path $versionedOutputDir -Force | Out-Null
    }

    foreach ($build in $builds) {
        Invoke-GitHubMsixBuild -Build $build -OutputFile (Join-Path $versionedOutputDir "${appName}_${version}_$($build.Name).msix")
    }

    $packages = @(Get-ChildItem -LiteralPath $versionedOutputDir -Filter '*.msix' -File)
    if ($packages.Count -ne $builds.Count) {
        throw "Expected $($builds.Count) packages under $versionedOutputDir, found: $(($packages | ForEach-Object Name) -join ', ')."
    }
    Invoke-MsixSigning -Files $packages
    Write-Host "Signed MSIX packages are under $versionedOutputDir"

    if (-not $SkipRelease) {
        $commit = (& git -C $repoRoot rev-parse HEAD).Trim()

        # Local-only tag; GitHub creates the remote tag at $commit when the draft is published.
        Write-Host "Creating local tag $tag at HEAD of '$branch' ($commit)..."
        & git -C $repoRoot tag $tag $commit
        if ($LASTEXITCODE -ne 0) {
            throw "git tag $tag failed."
        }

        # Always a draft: review and publish it manually on GitHub.
        $ghArguments = @('release', 'create', $tag) + $packages.FullName + @('--title', $tag, '--target', $commit, '--draft')
        $notesFile = $null
        if ($Notes) {
            # Pass notes via a file so quotes/newlines survive Windows PowerShell 5.1 native arg passing.
            $notesFile = [System.IO.Path]::GetTempFileName()
            [System.IO.File]::WriteAllText($notesFile, $Notes, [System.Text.UTF8Encoding]::new($false))
            $ghArguments += @('--notes-file', $notesFile)
        }
        else {
            $ghArguments += '--generate-notes'
        }
        if ($Prerelease) {
            $ghArguments += '--prerelease'
        }
        Write-Host "Creating draft GitHub release $tag..."
        Push-Location -LiteralPath $repoRoot
        try {
            & gh @ghArguments
            if ($LASTEXITCODE -ne 0) {
                throw "gh release create failed. Remove the local tag before retrying: git tag -d $tag"
            }
        }
        finally {
            Pop-Location
            if ($notesFile) {
                Remove-Item -LiteralPath $notesFile -Force -ErrorAction SilentlyContinue
            }
        }

        Write-Host "Done. Draft release $tag is saved; publishing it on GitHub will create the tag at $commit."
    }
    else {
        Write-Host 'Done.'
    }
}
catch {
    $scriptFailed = $true
    Write-Host ''
    Write-Host "ERROR: $($_.Exception.Message)" -ForegroundColor Red
    Write-Host $_.ScriptStackTrace -ForegroundColor DarkGray
}

if ($scriptFailed -and $Host.Name -eq 'ConsoleHost') {
    Write-Host ''
    Read-Host 'Build failed. Press Enter to close this window'
}

if ($scriptFailed) {
    exit 1
}
