<#
.SYNOPSIS
    Build the browser version of Lumit into flutter_ui\build\web.

.DESCRIPTION
    Four steps, each printed before it runs:

      1. cargo build      - the engine and bridge, for wasm32-unknown-unknown
      2. wasm-bindgen     - the page's side of the bridge, into flutter_ui\web\pkg
      3. wasm-opt         - shrinks the engine's wasm, when binaryen is on PATH
      4. flutter build web

    Needs the wasm32-unknown-unknown target (rustup target add
    wasm32-unknown-unknown) and wasm-bindgen-cli at the version in Cargo.lock
    (cargo install wasm-bindgen-cli --version <it> --locked). wasm-opt comes
    from binaryen, at the version the `web` job in ci.yml installs.

.PARAMETER Quick
    Build the engine without release optimisations. Quicker to build, slower
    to run.

.EXAMPLE
    .\scripts\build-web.ps1
#>
[CmdletBinding()]
param(
    [switch]$Quick
)

$ErrorActionPreference = 'Stop'

function Run([string]$line) {
    Write-Host "> $line" -ForegroundColor Cyan
    Invoke-Expression $line
    if ($LASTEXITCODE -ne 0) { throw "exited $LASTEXITCODE : $line" }
}

$repo = Split-Path $PSScriptRoot -Parent
$target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $repo 'target' }
# The web profile in Cargo.toml: release with full LTO and one codegen unit.
$build = if ($Quick) { 'debug' } else { 'web' }
$cargoProfile = if ($Quick) { 'dev' } else { 'web' }

Push-Location $repo
try {
    Run "cargo build -p lumit_bridge --lib --target wasm32-unknown-unknown --profile $cargoProfile"
    $wasm = Join-Path $target "wasm32-unknown-unknown/$build/lumit_bridge.wasm"
    Run "wasm-bindgen --target no-modules --no-typescript --out-dir flutter_ui/web/pkg `"$wasm`""
    # -O1: -O3 takes a quarter of an hour for 5% more and no smaller download.
    # The features are the ones Rust's wasm32 target turns on by default.
    if (-not $Quick -and (Get-Command wasm-opt -ErrorAction SilentlyContinue)) {
        $bg = 'flutter_ui/web/pkg/lumit_bridge_bg.wasm'
        Run "wasm-opt -O1 --enable-bulk-memory --enable-nontrapping-float-to-int --enable-reference-types --enable-multivalue $bg -o $bg"
    }
} finally {
    Pop-Location
}

Push-Location (Join-Path $repo 'flutter_ui')
try {
    # Nothing comes from a CDN: the renderer's files are served beside the app.
    # Flutter's default -O2. -O4 garbles text in this app (the Export button,
    # the time ruler), so don't ask for it.
    Run 'flutter build web --wasm --no-web-resources-cdn'
} finally {
    Pop-Location
}
