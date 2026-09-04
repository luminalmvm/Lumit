#Requires -Version 5.1
<#
.SYNOPSIS
    Takes a translation file from a community translator, checks it, merges it, and
    remembers the English each line was translated from.

.DESCRIPTION
    This is the second half of the translation round trip. A translator fills in
    https://lumitlab.com/translate, the page hands them a .json, they send it back as a
    GitHub issue, and this reads it in. Nothing else writes an app_<locale>.arb.

    The sidecar, flutter_ui/lib/l10n/translation-state.json, is the whole point. An .arb
    file says what the German for a string is; it does not say what the English was when
    somebody wrote that German. Without that, a reworded English string keeps its old
    translation for ever and nobody can tell. The sidecar records the source English per
    key per locale, so "stale" is a fact rather than a guess - both here and on the web
    page, which computes its list from this file at build time.

    Subcommands:

      seed                    Adopt the translations already in the .arb files: record
                              today's English as their source. Idempotent - it only adds
                              entries the sidecar does not have, so it can be run again
                              after any bulk change to a translation file.
      status                  Per locale: translated, missing, stale, orphaned.
      ingest <file.json>      Validate a translator's file and merge it.
      prune                   Drop translations of keys English no longer has, and expire
                              the stale ones (a translation whose English moved on
                              is deleted, not served).

    A locale entry counts as translated when the key is present and either differs from the
    English or has a line in the sidecar. The Crowdin exports these files came from wrote
    untranslated strings as their source text, so 1,032 of the 1,036 keys in app_de.arb are
    English wearing a German filename; counting those as done would hide a thousand strings
    from every translator who ever opens the page. The sidecar is what separates those from
    an answer that happens to read the same in both languages - sRGB is sRGB in German -
    because a line there means somebody looked at the row and decided.

.PARAMETER L10nDir
    The folder holding app_*.arb. Defaults to flutter_ui/lib/l10n next to this script;
    -SelfTest points it at a temporary copy.

.PARAMETER SelfTest
    Build a small l10n folder in the temp directory, run a full round trip through it,
    and assert the result. Touches nothing in the repository.

.EXAMPLE
    .\scripts\translations.ps1 status

.EXAMPLE
    .\scripts\translations.ps1 ingest .\lumit-de.json

.NOTES
    An ingest is a commit like any other: read the file, run this, look at the diff, and
    say in the commit message which language gained how many strings, and from whom - the
    page signs a translator in with Discord and stamps their name into the file, and this
    prints it for exactly that reason.

    The sign-in on the page is a courtesy, not a boundary. This script is the boundary: it
    is the only thing that validates a file, and it runs where a human reads the diff.
#>
[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [ValidateSet('seed', 'status', 'ingest', 'prune')]
    [string]$Command,

    [Parameter(Position = 1)]
    [string]$Path,

    [string]$L10nDir,

    [switch]$SelfTest
)

$ErrorActionPreference = 'Stop'

$script:Locales = @('de', 'kk', 'uk', 'zh', 'zh_Hant')
$script:StateName = 'translation-state.json'
$script:Dir = $null

# ---------------------------------------------------------------- JSON, both ways

function Read-JsonMap([string]$file) {
    # ConvertFrom-Json keeps the document's order in the PSCustomObject it returns, which
    # is what lets an .arb be rewritten without shuffling 2,000 lines.
    $obj = ConvertFrom-Json ([IO.File]::ReadAllText($file, [Text.Encoding]::UTF8))
    $map = [ordered]@{}
    foreach ($p in $obj.PSObject.Properties) { $map[$p.Name] = $p.Value }
    return $map
}

function ConvertTo-JsonString([string]$s) {
    $e = $s -replace '\\', '\\' -replace '"', '\"' -replace "`r", '\r' -replace "`n", '\n' -replace "`t", '\t'
    return '"' + $e + '"'
}

# Name/value pairs in document order, from either shape JSON arrives in - the ordered
# dictionaries this script builds, and the PSCustomObjects ConvertFrom-Json returns.
# Objects rather than two-element arrays because a one-property object's pair array
# flattens on the way out of the pipeline, and then [0] is the first letter of the key.
function Get-Pairs($value) {
    if ($value -is [System.Collections.IDictionary]) {
        return @($value.Keys | ForEach-Object { [pscustomobject]@{ Name = $_; Value = $value[$_] } })
    }
    return @($value.PSObject.Properties | ForEach-Object { [pscustomobject]@{ Name = $_.Name; Value = $_.Value } })
}

function Add-JsonValue([Text.StringBuilder]$sb, $value, [int]$depth) {
    if ($null -eq $value) { [void]$sb.Append('null'); return }
    if ($value -is [string]) { [void]$sb.Append((ConvertTo-JsonString $value)); return }
    if ($value -is [bool]) { [void]$sb.Append($(if ($value) { 'true' } else { 'false' })); return }
    if ($value -is [int] -or $value -is [long] -or $value -is [double]) {
        [void]$sb.Append([string]::Format([Globalization.CultureInfo]::InvariantCulture, '{0}', $value)); return
    }
    # An object. Anything else - an array, a date - is not something an .arb or the
    # sidecar contains, and guessing at it would write a file nobody can read back.
    if (-not ($value -is [System.Collections.IDictionary] -or $value -is [psobject])) {
        throw "cannot write a $($value.GetType().Name) to JSON"
    }
    $pairs = @(Get-Pairs $value)
    if ($pairs.Count -eq 0) { [void]$sb.Append('{}'); return }
    $pad = '  ' * ($depth + 1)
    [void]$sb.Append("{`n")
    for ($i = 0; $i -lt $pairs.Count; $i++) {
        [void]$sb.Append($pad).Append((ConvertTo-JsonString $pairs[$i].Name)).Append(': ')
        Add-JsonValue $sb $pairs[$i].Value ($depth + 1)
        if ($i -lt $pairs.Count - 1) { [void]$sb.Append(',') }
        [void]$sb.Append("`n")
    }
    [void]$sb.Append('  ' * $depth).Append('}')
}

function Write-JsonMap([string]$file, $map) {
    $sb = New-Object Text.StringBuilder
    Add-JsonValue $sb $map 0
    # No BOM, LF, no trailing newline: byte-for-byte the shape these files already have,
    # so an ingest shows as the lines it changed and nothing else.
    [IO.File]::WriteAllText($file, $sb.ToString(), (New-Object Text.UTF8Encoding($false)))
}

# ---------------------------------------------------------------- the files

function Get-ArbPath([string]$locale) { return (Join-Path $script:Dir "app_$locale.arb") }
function Get-StatePath { return (Join-Path $script:Dir $script:StateName) }

function Get-MessageKeys($map) { return @($map.Keys | Where-Object { -not $_.StartsWith('@') }) }

function Read-State {
    $p = Get-StatePath
    if (-not (Test-Path $p)) { return [ordered]@{} }
    $raw = Read-JsonMap $p
    $out = [ordered]@{}
    foreach ($loc in $raw.Keys) {
        $inner = [ordered]@{}
        foreach ($pair in (Get-Pairs $raw[$loc])) { $inner[$pair.Name] = $pair.Value }
        $out[$loc] = $inner
    }
    return $out
}

function Write-State($state) {
    $sorted = [ordered]@{}
    foreach ($loc in $script:Locales) {
        if (-not $state.Contains($loc)) { continue }
        $inner = [ordered]@{}
        foreach ($k in ($state[$loc].Keys | Sort-Object -CaseSensitive)) { $inner[$k] = $state[$loc][$k] }
        $sorted[$loc] = $inner
    }
    Write-JsonMap (Get-StatePath) $sorted
}

# Every {name} in a string. ICU plurals put the placeholder inside the branch
# ("other{{count} files}"), so the same expression finds those too.
function Get-Tokens([string]$s) {
    $set = New-Object 'System.Collections.Generic.HashSet[string]'
    foreach ($m in [regex]::Matches($s, '\{(\w+)\}')) { [void]$set.Add($m.Groups[1].Value) }
    return , $set   # the comma keeps it a set; a bare return hands back its contents
}

# ---------------------------------------------------------------- the four commands

function Get-Status {
    $en = Read-JsonMap (Get-ArbPath 'en')
    $state = Read-State
    $enKeys = Get-MessageKeys $en
    $rows = @()
    foreach ($loc in $script:Locales) {
        $arb = Read-JsonMap (Get-ArbPath $loc)
        $seen = @{}
        foreach ($k in $state[$loc].Keys) { $seen[$k] = $state[$loc][$k] }
        $translated = 0; $stale = 0; $orphan = 0
        foreach ($k in (Get-MessageKeys $arb)) {
            if (-not $en.Contains($k)) { $orphan++; continue }
            # The source text copied through is not a translation - unless the sidecar
            # records an answer for the key, because sRGB is sRGB in German too. This is
            # the same test the page makes, and the two must agree or a row a translator
            # has already answered comes back to them for ever.
            if ($arb[$k] -ceq $en[$k] -and -not $seen.ContainsKey($k)) { continue }
            if ($seen.ContainsKey($k) -and -not ($seen[$k] -ceq $en[$k])) { $stale++ } else { $translated++ }
        }
        $rows += [pscustomobject]@{
            Locale     = $loc
            Translated = $translated
            Stale      = $stale
            Missing    = $enKeys.Count - $translated - $stale
            Orphaned   = $orphan
            Of         = $enKeys.Count
        }
    }
    return $rows
}

function Show-Status {
    Get-Status | Format-Table Locale, Translated, Stale, Missing, Orphaned, Of -AutoSize | Out-String | Write-Host
}

function Invoke-Seed {
    $en = Read-JsonMap (Get-ArbPath 'en')
    $state = Read-State
    $added = 0
    foreach ($loc in $script:Locales) {
        if (-not $state.Contains($loc)) { $state[$loc] = [ordered]@{} }
        $arb = Read-JsonMap (Get-ArbPath $loc)
        foreach ($k in (Get-MessageKeys $arb)) {
            if (-not $en.Contains($k)) { continue }
            if ($arb[$k] -ceq $en[$k]) { continue }
            if ($state[$loc].Contains($k)) { continue }
            # The legacy assumption: whoever wrote this translated today's English. It is
            # the only assumption available, and it is right until an English string is
            # reworded - which is exactly what the sidecar then catches.
            $state[$loc][$k] = $en[$k]
            $added++
        }
    }
    Write-State $state
    Write-Host "seeded $added translations across $($script:Locales.Count) locales"
    return $added
}

function Invoke-Ingest([string]$file) {
    if (-not $file) { throw 'ingest needs a file: .\scripts\translations.ps1 ingest .\lumit-de.json' }
    if (-not (Test-Path $file)) { throw "no such file: $file" }
    $sub = Read-JsonMap $file

    $locale = [string]$sub['locale']
    if ($script:Locales -notcontains $locale) {
        throw "unknown locale '$locale' - Lumit translates into $($script:Locales -join ', '). A new language starts with an app_<locale>.arb and a line in this script's list."
    }
    if (-not $sub.Contains('entries')) { throw 'the file has no "entries" object' }

    $en = Read-JsonMap (Get-ArbPath 'en')
    $entries = @(Get-Pairs $sub['entries'])
    $sources = @{}
    if ($sub.Contains('sourceHashes')) {
        foreach ($p in (Get-Pairs $sub['sourceHashes'])) { $sources[$p.Name] = [string]$p.Value }
    }

    # Everything is checked before anything is written: a file that is wrong in one line
    # leaves the .arb untouched, so there is never a half-applied translation to unpick.
    $errors = @()
    $accept = @()
    foreach ($pair in $entries) {
        $key = [string]$pair.Name
        $text = [string]$pair.Value
        if (-not $en.Contains($key)) { $errors += "$key : no such string in app_en.arb"; continue }
        if ([string]::IsNullOrWhiteSpace($text)) { $errors += "$key : empty translation"; continue }
        $want = Get-Tokens ([string]$en[$key])
        $got = Get-Tokens $text
        if (-not $want.SetEquals($got)) {
            $errors += "$key : placeholders must survive translation - English has {$(($want | Sort-Object) -join '}, {')}, this has $(if ($got.Count) { '{' + (($got | Sort-Object) -join '}, {') + '}' } else { 'none' })"
            continue
        }
        $accept += [pscustomobject]@{ Key = $key; Text = $text }
    }
    if ($errors.Count -gt 0) {
        throw "$($errors.Count) problem(s), nothing was written:`n  " + ($errors -join "`n  ")
    }

    $arb = Read-JsonMap (Get-ArbPath $locale)
    $state = Read-State
    if (-not $state.Contains($locale)) { $state[$locale] = [ordered]@{} }

    $new = 0; $changed = 0; $arrivedStale = 0
    foreach ($a in $accept) {
        $key = $a.Key; $text = $a.Text
        if ($arb.Contains($key)) {
            if (-not ($arb[$key] -ceq $text)) { $changed++ }
            $arb[$key] = $text
        }
        else {
            Add-ArbKey $arb $key $text $en
            $new++
        }
        # What this translation was made from. If the translator worked from an English
        # string that has since been reworded, that is recorded as-is: the entry arrives
        # already stale rather than pretending to be current.
        $src = if ($sources.ContainsKey($key)) { $sources[$key] } else { [string]$en[$key] }
        if (-not ($src -ceq [string]$en[$key])) { $arrivedStale++ }
        $state[$locale][$key] = $src
    }

    Write-JsonMap (Get-ArbPath $locale) $arb
    Write-State $state
    Write-Host "$locale : $($accept.Count) entries ($new new, $changed reworded)"
    # The page signs a translator in with Discord and stamps their name on the file. Say it
    # here, because the commit message is the only place the credit survives.
    $translator = if ($sub.Contains('translator')) { [string]$sub['translator'] } else { '' }
    if ($translator) { Write-Host "  from $translator - name them in the commit message" }
    if ($arrivedStale -gt 0) {
        Write-Host "  $arrivedStale were translated from English that has since changed - status lists them as stale" -ForegroundColor Yellow
    }
    return [pscustomobject]@{ Locale = $locale; Count = $accept.Count; New = $new; Changed = $changed; Stale = $arrivedStale; Translator = $translator }
}

# Put a key where a sorted file would have it, and carry English's note across so the
# file keeps the shape it has. The .arb files are not actually sorted - they are in the
# order the application grew - so this inserts rather than re-sorting the file, which
# would be a 2,000-line diff that says nothing about what a translator sent.
function Add-ArbKey($arb, [string]$key, [string]$text, $en) {
    $meta = if ($en.Contains("@$key")) { $en["@$key"] } else { $null }
    $out = [ordered]@{}
    $placed = $false
    foreach ($k in @($arb.Keys)) {
        if (-not $placed -and -not $k.StartsWith('@') -and [string]::CompareOrdinal($k, $key) -gt 0) {
            $out[$key] = $text
            if ($meta) { $out["@$key"] = $meta }
            $placed = $true
        }
        $out[$k] = $arb[$k]
    }
    if (-not $placed) {
        $out[$key] = $text
        if ($meta) { $out["@$key"] = $meta }
    }
    $arb.Clear()
    foreach ($k in $out.Keys) { $arb[$k] = $out[$k] }
}

function Invoke-Prune {
    $en = Read-JsonMap (Get-ArbPath 'en')
    $state = Read-State
    $dropped = 0; $expired = 0
    foreach ($loc in $script:Locales) {
        $arb = Read-JsonMap (Get-ArbPath $loc)
        if (-not $state.Contains($loc)) { $state[$loc] = [ordered]@{} }
        foreach ($k in (Get-MessageKeys $arb)) {
            $gone = -not $en.Contains($k)
            $isStale = (-not $gone) -and $state[$loc].Contains($k) -and -not ($state[$loc][$k] -ceq [string]$en[$k]) -and -not ($arb[$k] -ceq [string]$en[$k])
            if (-not ($gone -or $isStale)) { continue }
            $arb.Remove($k)
            if ($arb.Contains("@$k")) { $arb.Remove("@$k") }
            if ($state[$loc].Contains($k)) { $state[$loc].Remove($k) }
            if ($gone) { $dropped++ } else { $expired++; Write-Host "  $loc : $k expired (English was reworded)" }
        }
        # A sidecar line for a translation that is no longer there says nothing. A
        # "machine:<key>" line says who wrote <key>, so it lives and dies with it.
        foreach ($k in @($state[$loc].Keys)) {
            $of = if ($k.StartsWith('machine:')) { $k.Substring(8) } else { $k }
            if (-not $arb.Contains($of)) { $state[$loc].Remove($k) }
        }
        Write-JsonMap (Get-ArbPath $loc) $arb
    }
    Write-State $state
    Write-Host "pruned: $dropped keys English no longer has, $expired stale translations expired"
    return [pscustomobject]@{ Dropped = $dropped; Expired = $expired }
}

# ---------------------------------------------------------------- the self-test

function Assert([bool]$ok, [string]$what) {
    if (-not $ok) { throw "self-test failed: $what" }
    Write-Host "  ok  $what"
}

function Invoke-SelfTest {
    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("lumit-tr-" + [guid]::NewGuid().ToString('n').Substring(0, 8))
    New-Item -ItemType Directory -Path $tmp | Out-Null
    try {
        $script:Dir = $tmp
        $en = [ordered]@{
            '@@locale'  = 'en'
            'apply'     = 'Apply'
            '@apply'    = [pscustomobject]@{ description = 'The confirm button.' }
            'files'     = '{count} files'
            '@files'    = [pscustomobject]@{ description = 'A count.'; placeholders = [pscustomobject]@{ count = [pscustomobject]@{ type = 'int' } } }
            'zoom'      = 'Zoom'
            '@zoom'     = [pscustomobject]@{ description = 'The Viewer control.' }
        }
        Write-JsonMap (Join-Path $tmp 'app_en.arb') $en
        foreach ($loc in $script:Locales) {
            $arb = [ordered]@{
                '@@locale' = $loc
                'apply'    = 'Apply'                 # the source copied through, not a translation
                'gone'     = 'Weg'                   # a key English no longer has
                'zoom'     = "Zoom-$loc"             # a real translation
            }
            Write-JsonMap (Join-Path $tmp "app_$loc.arb") $arb
        }

        Assert ((Invoke-Seed) -eq $script:Locales.Count) 'seed adopts only the real translations, not the English copies'
        $s = (Get-Status | Where-Object { $_.Locale -eq 'de' })
        Assert ($s.Translated -eq 1 -and $s.Missing -eq 2 -and $s.Orphaned -eq 1) 'status counts translated, missing and orphaned'

        # An answer that reads the same in both languages is an answer, once the sidecar
        # says somebody wrote it. Without the record it is the source text copied through,
        # which is what the line above counts as missing.
        $same = Join-Path $tmp 'same.json'
        Write-JsonMap $same ([ordered]@{ locale = 'de'; entries = [ordered]@{ apply = 'Apply' } })
        Invoke-Ingest $same | Out-Null
        $s = (Get-Status | Where-Object { $_.Locale -eq 'de' })
        Assert ($s.Translated -eq 2 -and $s.Missing -eq 1) 'an identical answer counts once the sidecar records it'

        # A file with a placeholder dropped is refused whole.
        $bad = Join-Path $tmp 'bad.json'
        Write-JsonMap $bad ([ordered]@{ locale = 'de'; entries = [ordered]@{ files = 'Dateien'; apply = 'Anwenden' } })
        $refused = $false
        try { Invoke-Ingest $bad | Out-Null } catch { $refused = $true }
        Assert $refused 'a translation that drops {count} is refused'
        Assert ((Read-JsonMap (Join-Path $tmp 'app_de.arb'))['apply'] -ceq 'Apply') 'the good line of a refused file was not written either'

        # The round trip: two entries in, both in the .arb and both in the sidecar.
        $good = Join-Path $tmp 'good.json'
        Write-JsonMap $good ([ordered]@{
                locale       = 'de'
                translator   = 'someone'
                entries      = [ordered]@{ files = '{count} Dateien'; apply = 'Anwenden' }
                sourceHashes = [ordered]@{ files = '{count} files'; apply = 'Apply' }
            })
        $r = Invoke-Ingest $good
        Assert ($r.Count -eq 2 -and $r.New -eq 1) 'ingest reports what it merged'
        Assert ($r.Translator -eq 'someone') 'the name the page stamped on the file comes back for the commit message'
        $de = Read-JsonMap (Join-Path $tmp 'app_de.arb')
        Assert ($de['files'] -ceq '{count} Dateien' -and $de['apply'] -ceq 'Anwenden') 'the translations reached the .arb'
        Assert ($de.Contains('@files')) "English's note came with the new key"
        Assert ((@($de.Keys) -join ',') -eq '@@locale,apply,files,@files,gone,zoom') 'a new key lands in sorted position'
        $st = Read-State
        Assert ($st['de']['files'] -ceq '{count} files') 'the sidecar remembers the English it was translated from'
        $s = (Get-Status | Where-Object { $_.Locale -eq 'de' })
        Assert ($s.Translated -eq 3 -and $s.Missing -eq 0 -and $s.Stale -eq 0) 'status sees all three translated'

        # Reword the English: the translation of it goes stale, and prune expires it.
        $en['zoom'] = 'Zoom level'
        Write-JsonMap (Join-Path $tmp 'app_en.arb') $en
        $s = (Get-Status | Where-Object { $_.Locale -eq 'de' })
        Assert ($s.Stale -eq 1 -and $s.Translated -eq 2) 'a reworded English string makes its translation stale'
        $p = Invoke-Prune
        Assert ($p.Expired -eq 5 -and $p.Dropped -eq 5) 'prune expires the stale ones and drops the orphans'
        $de = Read-JsonMap (Join-Path $tmp 'app_de.arb')
        Assert (-not $de.Contains('zoom') -and -not $de.Contains('gone')) 'both left the .arb'
        Assert (-not (Read-State)['de'].Contains('zoom')) 'and the sidecar line went with it'
        $s = (Get-Status | Where-Object { $_.Locale -eq 'de' })
        Assert ($s.Stale -eq 0 -and $s.Missing -eq 1) 'the expired string is simply missing again'

        Write-Host 'self-test passed' -ForegroundColor Green
    }
    finally {
        Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
    }
}

# ---------------------------------------------------------------- dispatch

if ($SelfTest) {
    Invoke-SelfTest
    return
}

$script:Dir = if ($L10nDir) { (Resolve-Path $L10nDir).Path } else { (Resolve-Path (Join-Path $PSScriptRoot '..\flutter_ui\lib\l10n')).Path }

switch ($Command) {
    'seed' { Invoke-Seed | Out-Null }
    'status' { Show-Status }
    'ingest' { Invoke-Ingest $Path | Out-Null }
    'prune' { Invoke-Prune | Out-Null }
    default { Get-Help $PSCommandPath -Detailed }
}
