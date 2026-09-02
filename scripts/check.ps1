<#
.SYNOPSIS
    Run the engine's own gates: formatting, clippy, and the tests.

.DESCRIPTION
    The same three things CI runs, in the same order, so a red CI is a surprise
    rather than a routine. Every command is printed before it runs.

    Formatting and clippy always cover the whole workspace - they are cheap, and
    a warning anywhere fails the merge. The tests are the slow part, so -Crate
    narrows them to one crate while you are working on it.

    This covers the engine only. The Flutter side is `flutter analyze` and
    `flutter test <one file>`; never the whole Flutter suite on this machine.
    See docs/learn/09-DOING-IT-YOURSELF.md for why.

.PARAMETER Crate
    Test only this crate, for example lumit-core. Omit it to test the whole
    workspace.

.PARAMETER SkipTests
    Formatting and clippy only. A fast pass before a commit.

.PARAMETER Fix
    Rewrite formatting instead of complaining about it (cargo fmt --all).

.EXAMPLE
    .\scripts\check.ps1 -Crate lumit-core

.EXAMPLE
    .\scripts\check.ps1
    The full engine pass: formatting, clippy, then every crate's tests. The GPU
    tests share one device and one set of compiled shaders per test process
    (crates/lumit-gpu/src/test_support.rs), so the whole pass is minutes, not
    hours. Use -Crate while working on one crate.
#>
[CmdletBinding()]
param(
    # A crate name, nothing else: the name is pasted into the command line that
    # actually runs, so it is checked rather than trusted.
    [ValidatePattern('^[a-z0-9_-]+$')]
    [string]$Crate,
    [switch]$SkipTests,
    [switch]$Fix
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

Push-Location $repo
try {
    if ($Fix) { Run 'cargo fmt --all' } else { Run 'cargo fmt --all --check' }
    Run 'cargo clippy --workspace --all-targets -- -D warnings'

    if (-not $SkipTests) {
        # One run, GPU crates included: their tests borrow one shared device
        # under a lock, so they serialise themselves - exactly what
        # .github/workflows/ci.yml runs too.
        if ($Crate) {
            Run "cargo test -p $Crate"
        } else {
            Run 'cargo test --workspace'
        }
    }
    Write-Host 'All green.' -ForegroundColor Green
} finally {
    Pop-Location
}
