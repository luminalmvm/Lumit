param([Parameter(Mandatory)][string]$FfmpegDir)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    . ./scripts/win-dev-env.ps1 -FfmpegDir $FfmpegDir
    $env:CARGO_BUILD_JOBS = '4'
    & cargo build -p lumit_bridge -p lumit-ofx-broker -p lumit-aplug-broker -p lumit-media-broker -p lumit-cli --release -j4
    if ($LASTEXITCODE) { throw 'Native build failed' }
    $target = (cargo metadata --no-deps --format-version 1 | ConvertFrom-Json).target_directory
    $env:CARGO_TARGET_DIR = $target
    $env:LUMIT_PREBUILT_BRIDGE = "$target/release/lumit_bridge.dll"
    & cmake -S ofx -B flutter_ui/build/resolve-ofx -G 'Visual Studio 17 2022' -A x64
    if ($LASTEXITCODE) { throw 'OFX configure failed' }
    & cmake --build flutter_ui/build/resolve-ofx --config Release -j4
    if ($LASTEXITCODE) { throw 'OFX build failed' }
    Push-Location flutter_ui
    try { & flutter build windows --release --no-pub }
    finally { Pop-Location }
    if ($LASTEXITCODE) { throw 'Editor build failed' }
    $editor = "$root/flutter_ui/build/windows/x64/runner/Release"
    $plugin = "$editor/Resolve/LumitPreview.ofx.bundle/Contents/Win64"
    New-Item -ItemType Directory -Force -Path $plugin | Out-Null
    Get-ChildItem "$FfmpegDir/bin" -Filter '*.dll' | Copy-Item -Destination "$editor/lib" -Force
    Copy-Item flutter_ui/build/resolve-ofx/Release/LumitPreview.ofx $plugin -Force
    Copy-Item ofx/link.lua, ofx/import.js, ofx/canvas.png "$editor/Resolve" -Force
    Write-Output "Built editor and OFX under $editor. Run ofx/install-menu.ps1 to register Resolve linking."
} finally { Pop-Location }
