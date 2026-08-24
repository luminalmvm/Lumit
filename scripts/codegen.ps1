<#
.SYNOPSIS
    Regenerate the Flutter/Rust bridge after editing crates/lumit-bridge/src/api/**.

.DESCRIPTION
    Four steps, in this order, and the order is the whole point:

      1. flutter pub get                        - the generator wants the
                                                  packages resolved
      2. flutter_rust_bridge_codegen generate   - writes flutter_ui/lib/src/rust/**
                                                  and the Rust glue
      3. cargo build -p lumit_bridge            - rebuilds the library the
                                                  Dart-side frb tests load
      4. git status                             - so you can see what changed

    Step 3 is the one that is easy to forget. The frb tests check the built
    library against a content hash; skip the rebuild and every one of them fails
    with "found 0 widgets", which reads like a broken widget and is not.

    Never edit anything under flutter_ui/lib/src/rust/ by hand. It is generated,
    CI regenerates it and compares, and a hand edit is undone by the next run of
    this script. See docs/17-BRIDGE-CONTRACT.md and
    docs/learn/09-DOING-IT-YOURSELF.md.

.PARAMETER SkipBuild
    Run the generator but not the rebuild. Only useful when you are about to
    build some other way.

.EXAMPLE
    .\scripts\codegen.ps1
#>
[CmdletBinding()]
param(
    [switch]$SkipBuild
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

# The generator must run from flutter_ui/, because that is where
# flutter_rust_bridge.yaml lives and every path in it is relative to that file.
Push-Location "$repo\flutter_ui"
try {
    Run 'flutter pub get'
    Run 'flutter_rust_bridge_codegen generate'
} finally {
    Pop-Location
}

Push-Location $repo
try {
    if (-not $SkipBuild) { Run 'cargo build -p lumit_bridge' }
    Write-Host 'What changed:' -ForegroundColor Green
    Run 'git status --short'
} finally {
    Pop-Location
}
