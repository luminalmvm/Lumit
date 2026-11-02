<#
.SYNOPSIS
    Build the browser version of Lumit into flutter_ui\build\web.

.DESCRIPTION
    Three steps, each printed before it runs:

      1. cargo build      - the engine and bridge, for wasm32-unknown-unknown
      2. wasm-bindgen     - the page's side of the bridge, into flutter_ui\web\pkg
      3. flutter build web

    Needs the wasm32-unknown-unknown target (rustup target add
    wasm32-unknown-unknown) and wasm-bindgen-cli at the version in Cargo.lock
    (cargo install wasm-bindgen-cli --version <it> --locked).

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
$build = if ($Quick) { 'debug' } else { 'release' }
$release = if ($Quick) { '' } else { '--release' }

Push-Location $repo
try {
    Run "cargo build -p lumit_bridge --lib --target wasm32-unknown-unknown $release"
    $wasm = Join-Path $target "wasm32-unknown-unknown/$build/lumit_bridge.wasm"
    Run "wasm-bindgen --target no-modules --no-typescript --out-dir flutter_ui/web/pkg `"$wasm`""
} finally {
    Pop-Location
}

Push-Location (Join-Path $repo 'flutter_ui')
try {
    # Nothing comes from a CDN: the renderer's files are served beside the app.
    # ponytail: -O0 because dart2wasm's wasm-opt step fails to parse this app's
    # module at -O1 and above; lift it once that is fixed upstream.
    Run 'flutter build web --wasm --no-web-resources-cdn -O0'
} finally {
    Pop-Location
}
