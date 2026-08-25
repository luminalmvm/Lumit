# Builds the Windows installer (K-252): a release build of the app, then the
# Inno Setup compile of packaging/windows/lumit.iss. Output lands in
# packaging/windows/dist/.
#
# Needs Inno Setup 6 on the PATH (or in its default location):
#   winget install JRSoftware.InnoSetup
#
# -Version overrides the installer's version (the release workflow passes the
# tag); without it the .iss default stands.
param([string]$Version)

$ErrorActionPreference = 'Stop'
$root = Resolve-Path "$PSScriptRoot\..\.."

Push-Location "$root\flutter_ui"
try {
    flutter build windows --release
    if ($LASTEXITCODE -ne 0) { throw "flutter build failed" }
} finally {
    Pop-Location
}

# The bridge links FFmpeg's *shared* build (scripts/win-dev-env.ps1), so the
# DLLs must ship beside the exe — a clean machine has no FFmpeg on its PATH.
$release = "$root\flutter_ui\build\windows\x64\runner\Release"
$ffbin = $null
if ($env:FFMPEG_LIBS_DIR) {
    $candidate = Join-Path (Split-Path $env:FFMPEG_LIBS_DIR) 'bin'
    if (Test-Path $candidate) { $ffbin = $candidate }
}
if ($null -eq $ffbin) {
    $av = Get-Command 'avcodec-*.dll' -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($null -ne $av) { $ffbin = Split-Path $av.Source }
}
if ($null -eq $ffbin) {
    Write-Error ("FFmpeg shared DLLs not found - set FFMPEG_LIBS_DIR " +
        "(scripts/win-dev-env.ps1) or put avcodec-*.dll on PATH")
    exit 1
}
Copy-Item "$ffbin\*.dll" $release -Force
Write-Host "Bundled FFmpeg DLLs from $ffbin"

$iscc = Get-Command iscc -ErrorAction SilentlyContinue
if ($null -ne $iscc) {
    $iscc = $iscc.Source
} else {
    # Machine-wide and per-user (winget default) install locations. The
    # ${env:ProgramFiles(x86)} braces are required — the parens are part of
    # the variable's name.
    $candidates = @(
        (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe'),
        (Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe')
    )
    $iscc = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
    if ($null -eq $iscc) {
        Write-Error ("Inno Setup not found. Install it with: " +
            "winget install JRSoftware.InnoSetup")
        exit 1
    }
}

$isccArgs = @()
if ($Version) { $isccArgs += "/DMyAppVersion=$Version" }
$isccArgs += "$PSScriptRoot\lumit.iss"
& $iscc @isccArgs
if ($LASTEXITCODE -ne 0) { throw "iscc failed" }
Write-Host "Installer written to $PSScriptRoot\dist"
