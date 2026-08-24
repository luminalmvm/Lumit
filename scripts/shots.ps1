<#
.SYNOPSIS
    Retake the manual's screenshots by driving the real application.

.DESCRIPTION
    A sweep is not a test. It is the application itself, started on a different
    entry point: it boots the same way lib/main.dart does, stages a project
    through the real engine, photographs its own window from outside, and quits.
    What the manual shows is therefore the program, not a harness impersonating
    it.

    This script does the three things a sweep needs:

      1. cargo build -p lumit_bridge   - the library the app loads
      2. $env:LUMIT_SHOTS = '1'        - the guard. Without it a sweep prints one
                                         line and exits, so nothing automatic can
                                         find itself driving the editor and
                                         overwriting the manual's pictures.
      3. flutter run -d windows -t tool/shots/shots_<sweep>.dart

    Finished pictures land in web-docs/src/assets/shots/ unless you pass -Out.

    The sweeps need media fixtures in C:/tmp/lumit-shots (Gameplay.mp4,
    Title card.mp4, Music.wav, Logo.png). They are not committed - a repository
    is not the place for video - so a fresh machine makes them with ffmpeg once.

    See docs/learn/09-DOING-IT-YOURSELF.md.

.PARAMETER Sweep
    Which sweep to run: 1 to 7, or retakes, or round_v2. What each one covers is
    written at the top of its own file in flutter_ui/tool/shots/.

.PARAMETER Shape
    The theme shape to photograph in: sharp (the default) or round. The manual is
    shot in the look it documents, so this is set once for a whole pass.

.PARAMETER Out
    Write the pictures somewhere else - useful for reviewing a pass before it
    overwrites the manual's own.

.PARAMETER NoCrop
    Keep the whole window instead of cropping to the panel. A first pass when a
    crop is landing in the wrong place.

.EXAMPLE
    .\scripts\shots.ps1 -Sweep 2

.EXAMPLE
    .\scripts\shots.ps1 -Sweep retakes -Shape round -Out C:\tmp\shots-review
#>
[CmdletBinding()]
param(
    # The name is pasted into the command line that actually runs, so it is
    # checked rather than trusted - and checked again against the files below.
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[A-Za-z0-9_]+$')]
    [string]$Sweep,
    [ValidateSet('sharp', 'round')]
    [string]$Shape = 'sharp',
    [string]$Out,
    [switch]$NoCrop
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

$target = "tool/shots/shots_$Sweep.dart"
if (-not (Test-Path "$repo\flutter_ui\$target")) {
    $known = (Get-ChildItem "$repo\flutter_ui\tool\shots\shots_*.dart" |
        Where-Object { $_.BaseName -ne 'shots_common' } |
        ForEach-Object { $_.BaseName -replace '^shots_', '' }) -join ', '
    throw "No sweep named '$Sweep'. Known sweeps: $known"
}

Push-Location $repo
try {
    Run 'cargo build -p lumit_bridge'
} finally {
    Pop-Location
}

$env:LUMIT_SHOTS = '1'
$env:LUMIT_SHOTS_SHAPE = $Shape
if ($Out) {
    # A sweep writes a picture the moment it has one and does not create the
    # folder first, so an -Out that does not exist yet takes the app down
    # mid-pass with a PathNotFoundException.
    if (-not (Test-Path $Out)) { New-Item -ItemType Directory -Path $Out | Out-Null }
    $env:LUMIT_SHOTS_OUT = $Out
}
if ($NoCrop) { $env:LUMIT_SHOTS_NOCROP = '1' }
Write-Host "> `$env:LUMIT_SHOTS = '1'; `$env:LUMIT_SHOTS_SHAPE = '$Shape'" -ForegroundColor Cyan

Push-Location "$repo\flutter_ui"
try {
    Run "flutter run -d windows -t $target"
} finally {
    Pop-Location
    Remove-Item Env:\LUMIT_SHOTS, Env:\LUMIT_SHOTS_SHAPE -ErrorAction SilentlyContinue
    Remove-Item Env:\LUMIT_SHOTS_OUT, Env:\LUMIT_SHOTS_NOCROP -ErrorAction SilentlyContinue
}

$where = if ($Out) { $Out } else { "$repo\web-docs\src\assets\shots" }
Write-Host "Pictures written into $where." -ForegroundColor Green
