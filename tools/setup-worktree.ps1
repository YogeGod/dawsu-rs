[CmdletBinding()]
param(
    [string]$BuildRoot = "",
    [switch]$ForceNpmCi
)

$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($BuildRoot)) {
    $BuildRoot = if (-not [string]::IsNullOrWhiteSpace($env:DAWSU_BUILD_ROOT)) {
        $env:DAWSU_BUILD_ROOT
    } else {
        "E:\dawsu-build"
    }
}

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$Slug = ((Split-Path -Leaf $RepoRoot) -replace '[^a-zA-Z0-9._-]', '-')

$TargetDest = Join-Path $BuildRoot "wt\$Slug\target"
$TmpDir = Join-Path $BuildRoot "tmp"
$SccacheDir = Join-Path $BuildRoot "sccache"
$NpmCacheDir = Join-Path $BuildRoot "npm-cache"
$UiCacheRoot = Join-Path $BuildRoot "ui-node-cache"

foreach ($dir in @($TargetDest, $TmpDir, $SccacheDir, $NpmCacheDir, $UiCacheRoot)) {
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
}

function Normalize-WinPath([string]$PathValue) {
    return [IO.Path]::GetFullPath($PathValue).TrimEnd('\', '/')
}

function New-Junction([string]$LinkPath, [string]$DestPath) {
    & cmd /c mklink /J "$LinkPath" "$DestPath" | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "mklink /J failed ($LASTEXITCODE): $LinkPath -> $DestPath"
    }
}

# 1. target/ junction
$TargetLink = Join-Path $RepoRoot "target"
$TargetItem = Get-Item -LiteralPath $TargetLink -Force -ErrorAction SilentlyContinue
$TargetStatus = ""
if ($null -eq $TargetItem) {
    New-Junction $TargetLink $TargetDest
    $TargetStatus = "linked -> $TargetDest"
} elseif ($TargetItem.Attributes -band [IO.FileAttributes]::ReparsePoint) {
    $curTarget = if ($TargetItem.Target) { ($TargetItem.Target | Select-Object -First 1) } else { $TargetDest }
    $TargetStatus = "kept junction -> $curTarget"
} else {
    Write-Warning "target/ already exists as a regular directory at $TargetLink; leaving intact."
    $TargetStatus = "regular dir (untouched)"
}

# 2. apps/tauri/ui/node_modules cache + junction
$UiDir = Join-Path $RepoRoot "apps\tauri\ui"
$LockPath = Join-Path $UiDir "package-lock.json"
$PkgPath = Join-Path $UiDir "package.json"
if (-not (Test-Path -LiteralPath $LockPath)) {
    throw "Missing lockfile: $LockPath"
}

$LockHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $LockPath).Hash.Substring(0, 12).ToLowerInvariant()
$CacheEntryDir = Join-Path $UiCacheRoot $LockHash
$CacheDir = Join-Path $CacheEntryDir "node_modules"
$CacheLockMarker = Join-Path $CacheDir ".package-lock.json"

if ($ForceNpmCi -or -not (Test-Path -LiteralPath $CacheLockMarker)) {
    New-Item -ItemType Directory -Force -Path $CacheEntryDir | Out-Null
    $StageDir = Join-Path $UiCacheRoot ".stage-$LockHash-$PID"
    if (Test-Path -LiteralPath $StageDir) {
        Remove-Item -LiteralPath $StageDir -Recurse -Force
    }
    New-Item -ItemType Directory -Force -Path $StageDir | Out-Null

    try {
        $MainUiDir = "E:\GitHub\dawsu-rs-main\apps\tauri\ui"
        $MainLockPath = Join-Path $MainUiDir "package-lock.json"
        $MainNmPath = Join-Path $MainUiDir "node_modules"
        $MainNmMarker = Join-Path $MainNmPath ".package-lock.json"
        $SeededFromMain = $false

        if (-not $ForceNpmCi -and (Test-Path -LiteralPath $MainLockPath) -and (Test-Path -LiteralPath $MainNmMarker)) {
            $MainLockHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $MainLockPath).Hash.Substring(0, 12).ToLowerInvariant()
            if ($MainLockHash -eq $LockHash) {
                $StageNm = Join-Path $StageDir "node_modules"
                New-Item -ItemType Directory -Force -Path $StageNm | Out-Null
                & robocopy "$MainNmPath" "$StageNm" /E /NFL /NDL /NJH /NJS /NP | Out-Null
                if ($LASTEXITCODE -ge 8) {
                    throw "robocopy from $MainNmPath failed with exit code $LASTEXITCODE"
                }
                $SeededFromMain = $true
            }
        }

        if (-not $SeededFromMain) {
            Copy-Item -LiteralPath $PkgPath -Destination (Join-Path $StageDir "package.json") -Force
            Copy-Item -LiteralPath $LockPath -Destination (Join-Path $StageDir "package-lock.json") -Force
            Push-Location $StageDir
            try {
                & npm ci --prefer-offline --no-audit --no-fund --cache "$NpmCacheDir"
                if ($LASTEXITCODE -ne 0) {
                    throw "npm ci failed with exit code $LASTEXITCODE"
                }
            } finally {
                Pop-Location
            }
        }

        if ((Test-Path -LiteralPath $CacheDir) -and ($ForceNpmCi -or -not (Test-Path -LiteralPath $CacheLockMarker))) {
            Remove-Item -LiteralPath $CacheDir -Recurse -Force
        }
        if (-not (Test-Path -LiteralPath $CacheLockMarker)) {
            try {
                Move-Item -LiteralPath (Join-Path $StageDir "node_modules") -Destination $CacheDir -ErrorAction Stop
            } catch {
                if (-not (Test-Path -LiteralPath $CacheLockMarker)) {
                    throw
                }
            }
        }
    } finally {
        if (Test-Path -LiteralPath $StageDir) {
            Remove-Item -LiteralPath $StageDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

$NodeModulesLink = Join-Path $UiDir "node_modules"
$NmItem = Get-Item -LiteralPath $NodeModulesLink -Force -ErrorAction SilentlyContinue
$NmStatus = ""
if ($null -eq $NmItem) {
    New-Junction $NodeModulesLink $CacheDir
    $NmStatus = "linked ($LockHash) -> $CacheDir"
} elseif ($NmItem.Attributes -band [IO.FileAttributes]::ReparsePoint) {
    $RawTarget = $NmItem.Target | Select-Object -First 1
    $CurNorm = if ($RawTarget) { Normalize-WinPath $RawTarget } else { "" }
    $WantNorm = Normalize-WinPath $CacheDir
    if ($CurNorm -eq $WantNorm) {
        $NmStatus = "kept junction ($LockHash) -> $CacheDir"
    } else {
        & cmd /c rmdir "$NodeModulesLink" | Out-Null
        if ($LASTEXITCODE -ne 0) {
            throw "Failed to remove stale junction $NodeModulesLink ($LASTEXITCODE)"
        }
        New-Junction $NodeModulesLink $CacheDir
        $NmStatus = "relinked ($LockHash) -> $CacheDir"
    }
} else {
    Write-Warning "apps/tauri/ui/node_modules already exists as a regular directory; leaving intact."
    $NmStatus = "regular dir (untouched)"
}

$SccacheCmd = Get-Command sccache -ErrorAction SilentlyContinue
$SccacheExe = if ($SccacheCmd) {
    $SccacheCmd.Source
} else {
    $WingetSccache = if (-not [string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
        Join-Path $env:LOCALAPPDATA "Microsoft\WinGet\Links\sccache.exe"
    } else {
        ""
    }
    if ($WingetSccache -and (Test-Path -LiteralPath $WingetSccache)) { $WingetSccache } else { "" }
}

$SccacheInfo = "not found (dir=$SccacheDir)"
if ($SccacheExe) {
    $prevEnvSccacheDir = $env:SCCACHE_DIR
    $prevPref = $ErrorActionPreference
    $env:SCCACHE_DIR = $SccacheDir
    $ErrorActionPreference = "Continue"
    try {
        & $SccacheExe --start-server 2>&1 | Out-Null
    } finally {
        $global:LASTEXITCODE = 0
        $ErrorActionPreference = $prevPref
        if ($null -eq $prevEnvSccacheDir) {
            Remove-Item Env:SCCACHE_DIR -ErrorAction SilentlyContinue
        } else {
            $env:SCCACHE_DIR = $prevEnvSccacheDir
        }
    }
    $SccacheInfo = "$SccacheExe (dir=$SccacheDir)"
}

$CargoTargetDir = if (-not [string]::IsNullOrWhiteSpace($env:CARGO_TARGET_DIR)) {
    $env:CARGO_TARGET_DIR
} else {
    $userTarget = [Environment]::GetEnvironmentVariable("CARGO_TARGET_DIR", "User")
    if (-not [string]::IsNullOrWhiteSpace($userTarget)) { $userTarget } else { "unset" }
}

$Libclang = if (-not [string]::IsNullOrWhiteSpace($env:LIBCLANG_PATH)) {
    $env:LIBCLANG_PATH
} elseif (Test-Path -LiteralPath "E:/Claude/drum-onnx/venv311/Lib/site-packages/clang/native") {
    "E:/Claude/drum-onnx/venv311/Lib/site-packages/clang/native"
} else {
    "unset"
}

Write-Output "setup-worktree: target=$TargetStatus | node_modules=$NmStatus | CARGO_TARGET_DIR=$CargoTargetDir | sccache=$SccacheInfo | LIBCLANG_PATH=$Libclang"
