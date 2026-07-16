<#
.SYNOPSIS
    Set up the environment variables Luminal's media crate needs to build on Windows.

.DESCRIPTION
    luminal-media links FFmpeg 7.1 through rsmpeg. On Windows, rusty_ffmpeg's build
    script links the import libraries named by FFMPEG_LIBS_DIR / FFMPEG_INCLUDE_DIR
    and generates its FFI with bindgen, which needs libclang (LIBCLANG_PATH). At run
    time the FFmpeg DLLs (and the ffmpeg CLI used by the test fixtures) must be on PATH.

    This script finds a BtbN FFmpeg 7.1 "shared, GPL" build and an LLVM install, then
    exports those variables for the current shell. Pass -Persist to also write them to
    your user environment so every new terminal has them.

    Nothing here hard-codes a machine-specific path: the FFmpeg build is discovered by
    convention (see -FfmpegDir) and LLVM at its standard install location.

.PARAMETER FfmpegDir
    The extracted FFmpeg build directory (the folder containing bin\, lib\, include\).
    If omitted, the script searches, in order:
      $env:KIRIKO_FFMPEG_DIR
      %USERPROFILE%\ffmpeg\ffmpeg-n7.1-*-win64-gpl-shared-*
      C:\ffmpeg\ffmpeg-n7.1-*-win64-gpl-shared-*

.PARAMETER Persist
    Also store the variables at User scope (like setx) so future shells inherit them.

.EXAMPLE
    . .\scripts\win-dev-env.ps1
    Dot-source it (note the leading dot) to set the variables in your current shell.

.EXAMPLE
    . .\scripts\win-dev-env.ps1 -Persist
    Set them for this shell and persist them for all future shells.

.NOTES
    Get the FFmpeg build from https://github.com/BtbN/FFmpeg-Builds/releases (the
    "ffmpeg-n7.1-latest-win64-gpl-shared-7.1.zip" asset) and extract it under
    %USERPROFILE%\ffmpeg\. Install LLVM 18 (winget install LLVM.LLVM --version 18.1.8):
    bindgen 0.71 mis-generates opaque structs against very new libclang, so pin 18.
    See docs/GUIDE.md "Building on Windows" and docs/impl/phase-0-kickoff.md slice 4.
#>
[CmdletBinding()]
param(
    [string]$FfmpegDir,
    [switch]$Persist
)

function Find-FfmpegDir {
    param([string]$Explicit)

    $candidates = @()
    if ($Explicit)               { $candidates += $Explicit }
    if ($env:KIRIKO_FFMPEG_DIR)  { $candidates += $env:KIRIKO_FFMPEG_DIR }

    foreach ($base in @("$env:USERPROFILE\ffmpeg", 'C:\ffmpeg')) {
        if (Test-Path $base) {
            $match = Get-ChildItem -Path $base -Directory -ErrorAction SilentlyContinue |
                Where-Object { $_.Name -like 'ffmpeg-n7.1-*win64-gpl-shared*' } |
                Sort-Object Name -Descending | Select-Object -First 1
            if ($match) { $candidates += $match.FullName }
        }
    }

    foreach ($c in $candidates) {
        if ($c -and (Test-Path "$c\lib") -and (Test-Path "$c\include") -and (Test-Path "$c\bin")) {
            return (Resolve-Path $c).Path
        }
    }
    return $null
}

function Find-LibclangDir {
    $candidates = @()
    if ($env:LIBCLANG_PATH) { $candidates += $env:LIBCLANG_PATH }
    $candidates += 'C:\Program Files\LLVM\bin'
    $candidates += 'C:\Program Files\LLVM\lib'
    foreach ($c in $candidates) {
        if ($c -and (Test-Path (Join-Path $c 'libclang.dll'))) { return $c }
    }
    return $null
}

$ff = Find-FfmpegDir -Explicit $FfmpegDir
if (-not $ff) {
    Write-Error @"
Could not find an FFmpeg 7.1 shared/GPL build.
Download ffmpeg-n7.1-latest-win64-gpl-shared-7.1.zip from
https://github.com/BtbN/FFmpeg-Builds/releases and extract it under %USERPROFILE%\ffmpeg\,
or pass -FfmpegDir <path-to-the-extracted-folder>.
"@
    return
}

$clang = Find-LibclangDir
if (-not $clang) {
    Write-Error @"
Could not find libclang.dll (needed by bindgen).
Install LLVM 18:  winget install LLVM.LLVM --version 18.1.8
(Very new libclang, e.g. 22, makes bindgen 0.71 emit broken opaque bindings — pin 18.)
"@
    return
}

$libs    = "$ff\lib"
$include = "$ff\include"
$bin     = "$ff\bin"

$env:FFMPEG_LIBS_DIR    = $libs
$env:FFMPEG_INCLUDE_DIR = $include
$env:LIBCLANG_PATH      = $clang
if (($env:Path -split ';') -notcontains $bin) {
    $env:Path = "$bin;$env:Path"
}

Write-Host "Luminal Windows dev environment set for this shell:" -ForegroundColor Green
Write-Host "  FFMPEG_LIBS_DIR    = $libs"
Write-Host "  FFMPEG_INCLUDE_DIR = $include"
Write-Host "  LIBCLANG_PATH      = $clang"
Write-Host "  PATH              += $bin"

if ($Persist) {
    [Environment]::SetEnvironmentVariable('FFMPEG_LIBS_DIR',    $libs,    'User')
    [Environment]::SetEnvironmentVariable('FFMPEG_INCLUDE_DIR', $include, 'User')
    [Environment]::SetEnvironmentVariable('LIBCLANG_PATH',      $clang,   'User')
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (($userPath -split ';') -notcontains $bin) {
        $newPath = if ([string]::IsNullOrEmpty($userPath)) { $bin } else { "$userPath;$bin" }
        [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
    }
    Write-Host "Persisted to your user environment (new terminals will inherit them)." -ForegroundColor Green
}
