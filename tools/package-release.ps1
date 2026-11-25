<#
.SYNOPSIS
    Builds and packages dawsu! for Windows x64 GitHub Releases.

.DESCRIPTION
    1. Optionally builds the Svelte UI bundle and `dawsu-tauri.exe` (via `cargo build --locked`).
    2. Stages `dawsu-tauri.exe`, `dawsu_vst3_host.exe` (if built), docs, and any default assets.
    3. Produces `dawsu-v<version>-windows-x64-portable.zip`.
    4. Invokes Inno Setup (`ISCC.exe`) if installed to produce `dawsu-v<version>-windows-x64-setup.exe`.
    5. Writes `SHA256SUMS.txt` covering every release artifact in the output directory.

.EXAMPLE
    pwsh tools/package-release.ps1
    pwsh tools/package-release.ps1 -SkipBuild -Profile debug
#>
[CmdletBinding()]
param(
    [string]$Version = "",
    [ValidateSet("release", "debug")]
    [string]$Profile = "release",
    [string]$OutputDir = "",
    [switch]$SkipBuild,
    [switch]$SkipInstaller,
    [switch]$RequireVst3Host
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$CargoTomlPath = Join-Path $RepoRoot "apps\tauri\Cargo.toml"

$TauriConfPath = Join-Path $RepoRoot "apps/tauri/tauri.conf.json"

$toml = Get-Content -Raw $CargoTomlPath
if ($toml -match '(?m)^version\s*=\s*"([^"]+)"') {
    $CargoVersion = $Matches[1]
} else {
    throw "Could not read version from $CargoTomlPath"
}
$ConfVersion = (Get-Content -Raw $TauriConfPath | ConvertFrom-Json).version
if (-not $ConfVersion) {
    throw "Could not read version from $TauriConfPath"
}
if ($CargoVersion -ne $ConfVersion) {
    throw "Version mismatch: Cargo.toml has $CargoVersion but tauri.conf.json has $ConfVersion."
}
if (-not $Version) {
    $Version = $CargoVersion
} elseif ($Version.TrimStart("v", "V") -ne $CargoVersion) {
    throw "Version mismatch: -Version $Version but Cargo.toml/tauri.conf.json have $CargoVersion."
}
$Version = $Version.TrimStart("v", "V")

if (-not $OutputDir) {
    $OutputDir = Join-Path $RepoRoot "dist\release"
}
$StageDir = Join-Path $RepoRoot "dist\stage\windows-x64"

Write-Host "==> Packaging dawsu! v$Version ($Profile)"

if (-not $SkipBuild) {
    Write-Host "==> Building UI bundle..."
    & npm --prefix (Join-Path $RepoRoot "apps\tauri\ui") run build
    if ($LASTEXITCODE -ne 0) { throw "UI build failed with exit code $LASTEXITCODE" }

    Write-Host "==> Building dawsu-tauri ($Profile)..."
    $cargoArgs = @("build", "-p", "dawsu-tauri", "--locked")
    if ($Profile -eq "release") {
        $cargoArgs += "--release"
    }
    Push-Location $RepoRoot
    try {
        & cargo @cargoArgs
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE" }
    } finally {
        Pop-Location
    }
}

$TauriExe = Join-Path $RepoRoot "target\$Profile\dawsu-tauri.exe"
if (-not (Test-Path $TauriExe)) {
    throw "Expected binary not found at $TauriExe (run without -SkipBuild first)."
}

# Locate optional VST3 sidecar host next to target binary or in native/vst3-host build dirs.
$Vst3HostCandidates = @(
    (Join-Path $RepoRoot "target\$Profile\dawsu_vst3_host.exe"),
    (Join-Path $RepoRoot "native\vst3-host\build\Release\dawsu_vst3_host.exe"),
    (Join-Path $RepoRoot "native\vst3-host\build\dawsu_vst3_host.exe")
)
$Vst3HostExe = $Vst3HostCandidates | Where-Object { Test-Path $_ } | Select-Object -First 1

# Prepare clean staging directory
if (Test-Path $StageDir) {
    Remove-Item -Recurse -Force $StageDir
}
New-Item -ItemType Directory -Force -Path $StageDir | Out-Null
New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null

Copy-Item $TauriExe (Join-Path $StageDir "dawsu-tauri.exe")
if ($Vst3HostExe) {
    Write-Host "==> Bundling VST3 sidecar: $Vst3HostExe"
    Copy-Item $Vst3HostExe (Join-Path $StageDir "dawsu_vst3_host.exe")
} elseif ($RequireVst3Host) {
    throw "dawsu_vst3_host.exe not found (checked: $($Vst3HostCandidates -join ', ')) and -RequireVst3Host was set."
} else {
    Write-Warning "dawsu_vst3_host.exe not found; packaging without VST3 sidecar."
}

foreach ($doc in @("README.md", "THIRD-PARTY-NOTICES.md")) {
    $docPath = Join-Path $RepoRoot $doc
    if (Test-Path $docPath) {
        Copy-Item $docPath (Join-Path $StageDir $doc)
    }
}

$DefaultAssets = Join-Path $RepoRoot "assets"
if (Test-Path $DefaultAssets) {
    Copy-Item -Recurse -Force $DefaultAssets (Join-Path $StageDir "assets")
}

# Build flat portable ZIP (updater.rs enforces flat archives with no subdirectories and checks portable.marker)
$PortableStageDir = Join-Path $OutputDir "portable-stage"
if (Test-Path $PortableStageDir) { Remove-Item -Recurse -Force $PortableStageDir }
New-Item -ItemType Directory -Force -Path $PortableStageDir | Out-Null
Get-ChildItem -Path $StageDir -File | Copy-Item -Destination $PortableStageDir
[System.IO.File]::WriteAllText((Join-Path $PortableStageDir "portable.marker"), "portable`n")

$PortableName = "dawsu-v$Version-windows-x64-portable.zip"
$PortableZip = Join-Path $OutputDir $PortableName
if (Test-Path $PortableZip) {
    Remove-Item -Force $PortableZip
}
Write-Host "==> Creating portable archive: $PortableZip"
Compress-Archive -Path (Join-Path $PortableStageDir "*") -DestinationPath $PortableZip -CompressionLevel Optimal
Remove-Item -Recurse -Force $PortableStageDir

# Optional Inno Setup installer
if (-not $SkipInstaller) {
    $IsccCandidates = @(
        (Get-Command "ISCC.exe" -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source -ErrorAction SilentlyContinue),
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
    ) | Where-Object { $_ -and (Test-Path $_) }
    $Iscc = $IsccCandidates | Select-Object -First 1

    if ($Iscc) {
        $IssPath = Join-Path $RepoRoot "installer\dawsu.iss"
        Write-Host "==> Building Inno Setup installer via $Iscc"
        & $Iscc "/DMyAppVersion=$Version" "/DSourceDir=$StageDir" "/DOutputDir=$OutputDir" $IssPath
        if ($LASTEXITCODE -ne 0) { throw "ISCC.exe failed with exit code $LASTEXITCODE" }
    } else {
        Write-Host "==> Inno Setup (ISCC.exe) not found; skipping installer build."
    }
}

# Generate SHA256SUMS.txt for all release artifacts in OutputDir
$Artifacts = Get-ChildItem -Path $OutputDir -File |
    Where-Object { $_.Extension -in @(".zip", ".exe") } |
    Sort-Object Name

$SumLines = foreach ($artifact in $Artifacts) {
    $hash = (Get-FileHash -Algorithm SHA256 -Path $artifact.FullName).Hash.ToLowerInvariant()
    "$hash  $($artifact.Name)"
}

$SumsPath = Join-Path $OutputDir "SHA256SUMS.txt"
$Utf8NoBom = New-Object System.Text.UTF8Encoding($false)
[System.IO.File]::WriteAllText($SumsPath, (($SumLines -join "`n") + "`n"), $Utf8NoBom)

Write-Host "==> Release artifacts ready in $OutputDir"
Get-Content $SumsPath | ForEach-Object { Write-Host "    $_" }
