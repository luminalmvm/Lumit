<#
.SYNOPSIS
Signs a draft Lumit release and publishes it.

.DESCRIPTION
CI leaves every release as a draft with an unsigned manifest. This downloads the
draft, checks every file against the manifest, signs the manifest with a key
kept on this Windows account, and publishes the release.

  .\scripts\sign-release.ps1 v0.12.0           sign and publish that draft
  .\scripts\sign-release.ps1 nightly -Yes      the newest nightly draft, unasked
  .\scripts\sign-release.ps1 -NewKey           make a key and keep it here
  .\scripts\sign-release.ps1 -NewKey -Backup   make a key to keep offline
  .\scripts\sign-release.ps1 -ImportKey k.pem  keep a PEM key here

Needs gh, signed in, and dart. Keys live in .lumit-signing in the user folder,
sealed to this Windows account.
#>
param(
    [Parameter(Position = 0)][string]$Release,
    [switch]$Yes,
    [switch]$NewKey,
    [switch]$Backup,
    [string]$ImportKey,
    [string]$Repo = 'luminalmvm/Lumit'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 3
$ProgressPreference = 'SilentlyContinue'
Add-Type -AssemblyName System.Security

$KeyFolder = Join-Path $env:USERPROFILE '.lumit-signing'
$Entropy = [Text.Encoding]::UTF8.GetBytes('lumit-release-key')
# The fixed PKCS#8 header of an Ed25519 private key; the 32-byte seed follows.
$Pkcs8Prefix = [byte[]](0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06,
    0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20)
$Flutter = Join-Path $PSScriptRoot '..\flutter_ui'
$ManifestName = 'lumit-release-manifest.json'
$SignatureName = 'lumit-release-manifest.json.sig'

function Fail([string]$message) {
    Write-Host $message -ForegroundColor Red
    exit 1
}

function Need([string]$tool) {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) { Fail "$tool is not on PATH." }
}

# Not named gh, which would call itself.
function Invoke-Gh {
    $out = & gh @args
    if ($LASTEXITCODE -ne 0) { Fail "gh $($args[0]) $($args[1]) failed." }
    $out
}

function GhJson([string]$path) {
    # Through a variable, so Windows PowerShell unrolls a JSON array.
    $json = (Invoke-Gh api $path) -join "`n" | ConvertFrom-Json
    $json
}

# Runs tool/release_sign.dart with the seed on stdin, and answers its last line.
function Invoke-Signer([byte[]]$seed, [string[]]$arguments) {
    Need dart
    Push-Location $Flutter
    try {
        if (-not (Test-Path '.dart_tool\package_config.json')) {
            Need flutter
            flutter pub get | Out-Host
            if ($LASTEXITCODE -ne 0) { Fail 'flutter pub get failed.' }
        }
        $out = [Convert]::ToBase64String($seed) | & dart run tool/release_sign.dart @arguments
        if ($LASTEXITCODE -ne 0 -or -not $out) { Fail 'The signing tool failed.' }
    } finally {
        Pop-Location
    }
    @($out)[-1].Trim()
}

function Save-Key([byte[]]$seed) {
    $public = Invoke-Signer $seed @('public')
    $path = Join-Path $KeyFolder ($public.Substring(0, 8).Replace('/', '_').Replace('+', '-') + '.key')
    if (Test-Path $path) { Fail "$path already exists." }
    New-Item -ItemType Directory -Force $KeyFolder | Out-Null
    $sealed = [Security.Cryptography.ProtectedData]::Protect($seed, $Entropy, 'CurrentUser')
    Set-Content -Path $path -Encoding Ascii -Value @($public, [Convert]::ToBase64String($sealed))
    Write-Host "Stored the key in $path"
    Write-Host "Public key: $public"
}

function Open-Key([string]$path) {
    $sealed = [Convert]::FromBase64String((Get-Content $path)[1])
    return , [Security.Cryptography.ProtectedData]::Unprotect($sealed, $Entropy, 'CurrentUser')
}

function Get-PinnedKeys([string]$ref) {
    $text = (Invoke-Gh api "repos/$Repo/contents/flutter_ui/lib/state/release_signature.dart?ref=$ref" `
            -H 'Accept: application/vnd.github.raw') -join "`n"
    @([regex]::Matches($text, "'([A-Za-z0-9+/]{43}=)'") | ForEach-Object { $_.Groups[1].Value })
}

# Sorts v0.6.1-nightly.20261012.57 by version, then date, then run.
function Get-NightlyOrder([string]$tag) {
    if ($tag -notmatch '(\d+)\.(\d+)\.(\d+)-nightly\.(\d+)\.(\d+)') { return '' }
    '{0:D6}{1:D6}{2:D6}{3:D10}{4:D10}' -f @(1..5 | ForEach-Object { [long]$Matches[$_] })
}

if ($NewKey -or $ImportKey) {
    if ($NewKey) {
        $seed = New-Object byte[] 32
        $rng = [Security.Cryptography.RandomNumberGenerator]::Create()
        $rng.GetBytes($seed)
        $rng.Dispose()
    } else {
        $text = Get-Content -Raw $ImportKey
        if ($text -notmatch '-----BEGIN PRIVATE KEY-----([\s\S]+?)-----END PRIVATE KEY-----') {
            Fail 'That file holds no unencrypted private key.'
        }
        $der = [Convert]::FromBase64String(($Matches[1] -replace '\s', ''))
        if ($der.Length -ne 48 -or [BitConverter]::ToString($der, 0, 16) -ne [BitConverter]::ToString($Pkcs8Prefix)) {
            Fail 'That is not an Ed25519 private key.'
        }
        $seed = [byte[]]$der[16..47]
        [Array]::Clear($der, 0, $der.Length)
    }
    if ($NewKey -and $Backup) {
        $der = [byte[]]($Pkcs8Prefix + $seed)
        Write-Host 'Keep this offline, on paper or in a password manager. Never on this PC.'
        Write-Host '-----BEGIN PRIVATE KEY-----'
        Write-Host ([Convert]::ToBase64String($der))
        Write-Host '-----END PRIVATE KEY-----'
        Write-Host "Public key: $(Invoke-Signer $seed @('public'))"
        [Array]::Clear($der, 0, $der.Length)
    } else {
        Save-Key $seed
        if ($ImportKey) { Write-Host "You can now delete $ImportKey, or move it off this PC." }
    }
    [Array]::Clear($seed, 0, $seed.Length)
    exit 0
}

if (-not $Release) { Fail 'Name a release, such as v0.12.0 or nightly. -? says the rest.' }
Need gh
$null = & gh auth status
if ($LASTEXITCODE -ne 0) { Fail 'gh is not signed in. Run gh auth login.' }

Write-Host 'Finding the release'
$releases = GhJson "repos/$Repo/releases?per_page=100"
$nightlies = @($releases | Where-Object { $_.tag_name -like '*-nightly.*' } |
        Sort-Object { Get-NightlyOrder $_.tag_name })
$drafts = @($nightlies | Where-Object { $_.draft })
if ($Release -eq 'nightly') {
    if (-not $drafts) {
        Write-Host 'No nightly draft to sign.'
        exit 0
    }
    $tag = $drafts[-1].tag_name
} else {
    $tag = 'v' + $Release.TrimStart('v')
}
$rel = @($releases | Where-Object { $_.tag_name -ceq $tag })
if (-not $rel) { Fail "There is no release $tag." }
$rel = $rel[0]
if (-not $rel.draft) { Fail "$tag is already published." }
$version = $tag.Substring(1)

Write-Host "Downloading $tag"
# Whatever an earlier run left behind goes first.
Get-ChildItem -Directory (Join-Path $env:TEMP 'lumit-sign-*') | Remove-Item -Recurse -Force
$work = Join-Path $env:TEMP "lumit-sign-$version"
New-Item -ItemType Directory $work | Out-Null
Invoke-Gh release download $tag --repo $Repo --dir $work | Out-Host

Write-Host 'Checking every file against the manifest'
$manifestPath = Join-Path $work $ManifestName
if (-not (Test-Path $manifestPath)) { Fail "$tag has no $ManifestName." }
$manifest = [IO.File]::ReadAllText($manifestPath) | ConvertFrom-Json
if ($manifest.version -cne $version) { Fail "The manifest is for $($manifest.version), not $version." }
$names = @(Get-ChildItem -File $work | ForEach-Object Name |
        Where-Object { $_ -cne $ManifestName -and $_ -cne $SignatureName })
$named = @($manifest.assets | ForEach-Object { $_.name })
$missing = @($names | Where-Object { $named -cnotcontains $_ })
$extra = @($named | Where-Object { $names -cnotcontains $_ })
if ($missing) { Write-Host "Not in the manifest: $($missing -join ', ')" }
if ($extra) { Write-Host "In the manifest but not on the release: $($extra -join ', ')" }
if ($missing -or $extra -or $named.Count -ne $names.Count) { Fail 'The manifest does not match the release. Nothing was signed.' }
$bad = 0
foreach ($asset in $manifest.assets) {
    $file = Get-Item -LiteralPath (Join-Path $work $asset.name)
    $hash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLower()
    $api = @($rel.assets | Where-Object { $_.name -ceq $asset.name })
    $digest = if ($api) { $api[0].PSObject.Properties['digest'] } else { $null }
    $problem = $null
    if ($file.Length -ne $asset.size) { $problem = "$($file.Length) bytes, the manifest says $($asset.size)" }
    elseif ($hash -cne $asset.sha256) { $problem = 'its SHA-256 differs from the manifest' }
    elseif ($digest -and $digest.Value -and $digest.Value -cne "sha256:$hash") { $problem = 'its SHA-256 differs from GitHub''s' }
    if ($problem) { Write-Host "$($asset.name): $problem" -ForegroundColor Red; $bad++ }
    else { Write-Host "$($asset.name): OK" }
}
if ($bad) { Fail 'Nothing was signed.' }

Write-Host 'Choosing the key'
$previous = $null
if ($tag -like '*-nightly.*') {
    $published = @($nightlies | Where-Object { -not $_.draft })
    if ($published) { $previous = $published[-1].tag_name }
}
if (-not $previous) { $previous = (GhJson "repos/$Repo/releases/latest").tag_name }
$trusted = Get-PinnedKeys $previous
# The last key the previous release pins that is held here. A key on its way
# out is pinned last, so it keeps signing until a release drops it.
$keyFile = $null
if (Test-Path $KeyFolder) {
    $held = @(Get-ChildItem -File (Join-Path $KeyFolder '*.key'))
    foreach ($key in $trusted) {
        $match = @($held | Where-Object { (Get-Content $_.FullName -TotalCount 1) -ceq $key })
        if ($match) { $keyFile = $match[0] }
    }
}
if (-not $keyFile) { Fail "No key here is one $previous trusts. Import the key it pins with -ImportKey." }
$public = Get-Content $keyFile.FullName -TotalCount 1
Write-Host "Signing with $public, which $previous trusts"
$own = $rel.target_commitish
if (@(GhJson "repos/$Repo/git/matching-refs/tags/$tag" | Where-Object { $_.ref -ceq "refs/tags/$tag" })) { $own = $tag }
if ((Get-PinnedKeys $own) -cnotcontains $public) {
    Write-Host "Warning: $tag does not pin this key, so the next release must be signed with a key it does pin." -ForegroundColor Yellow
}

Write-Host 'Signing the manifest'
$seed = Open-Key $keyFile.FullName
try {
    $signature = Invoke-Signer $seed @('sign', $manifestPath)
} finally {
    [Array]::Clear($seed, 0, $seed.Length)
}
if ($signature -cnotmatch '^[A-Za-z0-9+/]{86}==$') { Fail 'The signing tool did not give a signature.' }
$sigPath = Join-Path $work $SignatureName
[IO.File]::WriteAllText($sigPath, $signature, [Text.Encoding]::ASCII)

if (-not $Yes) {
    if ((Read-Host "Publish $tag now? [y/N]") -ne 'y') {
        Remove-Item -Recurse -Force $work
        Write-Host "Not published. $tag is still a draft."
        exit 0
    }
}

Write-Host "Publishing $tag"
Invoke-Gh release upload $tag $sigPath --clobber --repo $Repo | Out-Host
Invoke-Gh release edit $tag --draft=false --repo $Repo | Out-Null
Write-Host "Published: https://github.com/$Repo/releases/tag/$tag"
Remove-Item -Recurse -Force $work

if ($Release -eq 'nightly') {
    foreach ($old in @($drafts | Where-Object { $_.tag_name -cne $tag })) {
        Invoke-Gh release delete $old.tag_name --yes --repo $Repo | Out-Host
        Write-Host "Deleted the older draft $($old.tag_name)"
    }
}
