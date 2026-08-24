<#
.SYNOPSIS
    Regenerate the manual's effect pages from the engine's own catalogue.

.DESCRIPTION
    Every effect's parameters - names, ranges, defaults, units - are written down
    once, in Rust, on the effect's declaration. Nobody types them into the
    manual. They travel:

      1. cargo test -p lumit-core regenerate_fx_reference -- --ignored
         asks the engine to write its whole catalogue to
         crates/lumit-core/fx-reference.json. (--ignored is Rust's way of saying
         "only when asked for by name" - the test writes a file, so it must not
         fire on every run.)
      2. npm run docs:effects, from web-docs/, turns that file into one page per
         effect, plus one table per category on the section index.

    The generator rewrites only the block between the GENERATED markers on each
    page. The prose above and below - what the effect is for, what each control
    does - is hand-written and survives untouched.

    See docs/learn/09-DOING-IT-YOURSELF.md for the whole routine.

.PARAMETER Check
    Change nothing; fail if the pages have fallen behind the engine. This asks
    "is the manual stale?" and answers with an exit code.

.PARAMETER Pictures
    Also run npm run docs:effect-shots, which renders each effect's
    before-and-after picture with the engine itself. Slow - minutes - and needs
    ffmpeg on PATH, which win-dev-env.ps1 arranges.

.PARAMETER Only
    With -Pictures, redo one effect's picture instead of all of them. Takes an
    effect's match name, for example accumulation_mb.

.EXAMPLE
    .\scripts\manual-pages.ps1
    The usual case after changing a slider's range or adding an effect.

.EXAMPLE
    .\scripts\manual-pages.ps1 -Check

.EXAMPLE
    .\scripts\manual-pages.ps1 -Pictures -Only accumulation_mb
#>
[CmdletBinding()]
param(
    [switch]$Check,
    [switch]$Pictures,
    [ValidatePattern('^[A-Za-z0-9_]+$')]
    [string]$Only
)

$ErrorActionPreference = 'Stop'

# Prints the command, runs it, stops on failure. The line is printed exactly as
# it is run, so what scrolls past is what you would type yourself.
#
# It runs the line rather than an argument array on purpose: `npm run x -- --y`
# does not survive being splatted, because PowerShell treats a bare `--` as its
# own end-of-parameters marker and eats it before npm ever sees it.
function Run([string]$line) {
    Write-Host "> $line" -ForegroundColor Cyan
    Invoke-Expression $line
    if ($LASTEXITCODE -ne 0) { throw "exited $LASTEXITCODE : $line" }
}

if ($Check -and $Pictures) { throw '-Pictures renders files; it cannot be combined with -Check.' }

$repo = Split-Path $PSScriptRoot -Parent
. "$PSScriptRoot\win-dev-env.ps1"

if (-not $Check) {
    Push-Location $repo
    try {
        Run 'cargo test -p lumit-core regenerate_fx_reference -- --ignored'
    } finally {
        Pop-Location
    }
}

Push-Location "$repo\web-docs"
try {
    if (-not (Test-Path 'node_modules')) { Run 'npm install' }

    if ($Check) { Run 'npm run docs:effects -- --check' } else { Run 'npm run docs:effects' }

    if ($Pictures) {
        if ($Only) {
            Write-Host "  (LUMIT_FX_EXAMPLES_ONLY=$Only - just this one effect)" -ForegroundColor DarkGray
            $env:LUMIT_FX_EXAMPLES_ONLY = $Only
        }
        try {
            Run 'npm run docs:effect-shots'
        } finally {
            Remove-Item Env:\LUMIT_FX_EXAMPLES_ONLY -ErrorAction SilentlyContinue
        }
    }
    Write-Host 'The manual matches the engine.' -ForegroundColor Green
} finally {
    Pop-Location
}
