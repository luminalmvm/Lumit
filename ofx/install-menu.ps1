param([string]$Editor = "$PSScriptRoot/../flutter_ui/build/windows/x64/runner/Release")
$ErrorActionPreference = 'Stop'
$Editor = (Resolve-Path -LiteralPath $Editor).Path
if (!(Test-Path -LiteralPath "$Editor/Resolve/LumitPreview.ofx.bundle/Contents/Win64/LumitPreview.ofx")) {
    throw 'Build the Lumit Resolve plugin first.'
}
$menu = Join-Path $env:APPDATA 'Blackmagic Design/DaVinci Resolve/Support/Fusion/Scripts/Utility'
New-Item -ItemType Directory -Force -Path "$Editor/Resolve", $menu | Out-Null
Copy-Item "$PSScriptRoot/link.lua", "$PSScriptRoot/import.js", "$PSScriptRoot/canvas.png" "$Editor/Resolve" -Force
$quoted = $Editor.Replace('\','\\').Replace('"','\"')
@"
local root = "$quoted"
assert(loadfile(root .. '/Resolve/link.lua'))(root)
"@ | Set-Content -LiteralPath (Join-Path $menu 'Create Lumit Composition.lua') -Encoding utf8
$pluginPath = Join-Path $Editor 'Resolve'
$paths = @([Environment]::GetEnvironmentVariable('OFX_PLUGIN_PATH', 'User') -split ';' |
    Where-Object { $_ -and $_.TrimEnd('\', '/') -ine $pluginPath })
$paths += $pluginPath
[Environment]::SetEnvironmentVariable('OFX_PLUGIN_PATH', ($paths -join ';'), 'User')
$env:OFX_PLUGIN_PATH = $paths -join ';'
Write-Output 'Installed Create Lumit Composition in Workspace > Scripts.'
Write-Output 'Registered the Resolve plugin folder. Restart Resolve before testing.'
