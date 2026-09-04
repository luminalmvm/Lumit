<#
.SYNOPSIS
    Build the engine, or just the bridge library the Flutter app loads.

.DESCRIPTION
    Sets the Windows media environment (scripts\win-dev-env.ps1), then runs one
    cargo build. It prints every command before running it, so the raw commands
    stay learnable: nothing here is hidden from you.

    You do not need this to run the app - `flutter run` builds the bridge crate
    itself. You need it before `flutter test`, because the Dart-side frb tests
    load the built library rather than building it.

    See docs/learn/09-DOING-IT-YOURSELF.md for the whole routine.

.PARAMETER Release
    Build with optimisations and no debug assertions. Slower to compile, and
    what the owner measures performance against. A debug build here is not an
    unoptimised one: the four maths crates are optimised in both profiles.

.PARAMETER BridgeOnly
    Build only crates/lumit-bridge (cargo package name `lumit_bridge`, with an
    underscore). This is the one flutter_ui/test/frb/ loads.

    It is not the only thing the app needs: lumit-ofx-broker and
    lumit-aplug-broker are separate executables that open one OFX bundle, or one
    CLAP module, in a process of their own, and a full --workspace
    build makes them. `flutter run` builds all three itself.

.EXAMPLE
    .\scripts\build.ps1
    A debug build of the whole workspace.

.EXAMPLE
    .\scripts\build.ps1 -BridgeOnly
    Just the library the Flutter tests load. Run this before `flutter test`.

.EXAMPLE
    .\scripts\build.ps1 -Release
#>
[CmdletBinding()]
param(
    [switch]$Release,
    [switch]$BridgeOnly
)

$ErrorActionPreference = 'Stop'

# Prints the command, runs it, stops on failure. The line is printed exactly as
# it is run, so what scrolls past is what you would type yourself.
function Run([string]$line) {
    Write-Host "> $line" -ForegroundColor Cyan
    Invoke-Expression $line
    if ($LASTEXITCODE -ne 0) { throw "exited $LASTEXITCODE : $line" }
}

$repo = Split-Path $PSScriptRoot -Parent
. "$PSScriptRoot\win-dev-env.ps1"

$what = if ($BridgeOnly) { '-p lumit_bridge' } else { '--workspace' }
$how = if ($Release) { ' --release' } else { '' }

Push-Location $repo
try {
    Run "cargo build $what$how"
    $profileDir = if ($Release) { 'release' } else { 'debug' }
    Write-Host "Built into target\$profileDir\." -ForegroundColor Green
} finally {
    Pop-Location
}
