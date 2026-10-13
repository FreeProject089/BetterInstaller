# =====================================================================
#  BMM release automation (BetterInstaller).
#  Bumps versions -> builds BMM -> packs/signs/stamps -> makes a delta from the
#  previous release -> writes a multi-source update.json -> (optionally) publishes
#  the GitHub release with all assets.
#
#  Usage (from the BetterInstaller root):
#    ./examples/bmm/release.ps1 -Version 1.1.0 -Notes "Dark mode + faster scans"
#    ./examples/bmm/release.ps1 -Version 1.1.0 -Notes "..." -Publish          # also `gh release create`
#    ./examples/bmm/release.ps1 -Version 1.1.0 -SkipBmmBuild                   # reuse the current BMM exe
#
#  ASCII-only (Windows PowerShell 5.1).
# =====================================================================
param(
    [Parameter(Mandatory = $true)][string]$Version,         # new version, e.g. 1.1.0
    [string]$Notes = "",                                    # release notes (manifest + GH body)
    [string]$BmmRoot = "..",                                # BetterModsManager repo root
    [string[]]$ExtraSources = @(),                          # extra manifest URLs baked into update.json
    [switch]$SkipBmmBuild,                                  # don't rebuild the BMM exe
    [switch]$Publish                                        # run `gh release create`
)
$ErrorActionPreference = "Stop"

$cfgPath  = "examples/bmm/installer.toml"
$tauriCfg = Join-Path $BmmRoot "src-tauri/tauri.conf.json"
$cargoTml = Join-Path $BmmRoot "src-tauri/Cargo.toml"
# Distributable artifacts go into <BmmRoot>/Release (gitignored), not examples/bmm.
$relDir   = Join-Path $BmmRoot "Release"
New-Item -ItemType Directory -Force $relDir | Out-Null
$pkg      = Join-Path $relDir "bmm.bpkg"
$setup    = Join-Path $relDir "BMM-Setup.exe"
$manifest = Join-Path $relDir "update.json"
$archive  = Join-Path $relDir "releases"
$bpkg     = "./target/release/bpkg.exe"

function Read-TomlVersion($path) {
    (Select-String -Path $path -Pattern '(?m)^\s*version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
}

# 0) Discover the version we're upgrading FROM (for the delta), then archive its package.
$prevVer = Read-TomlVersion $cfgPath
Write-Host "[release] $prevVer -> $Version"
New-Item -ItemType Directory -Force $archive | Out-Null
$prevPkg = Join-Path $archive ("bmm-{0}.bpkg" -f $prevVer)
# Re-running a release (installer.toml already at $Version) has no previous version to diff from.
if ($prevVer -eq $Version) { Write-Host "[release] same version as installer.toml: no archive, no delta" }
if (($prevVer -ne $Version) -and (Test-Path $pkg) -and -not (Test-Path $prevPkg)) {
    Copy-Item $pkg $prevPkg
    Write-Host "[release] archived previous package -> $prevPkg"
}

# 1) Bump the three version sources (keep them in lockstep).
# Read AND written as UTF-8 without a byte-order mark. Windows PowerShell 5.1 reads a BOM-less
# file with `Get-Content` as Windows-1252, which turned every accent of tauri.conf.json and
# Cargo.toml into mojibake; `Set-Content -Encoding utf8` adds a BOM that breaks their parsers.
# A file already at this version is not rewritten at all.
function Set-Version($path, $pattern, $replacement) {
    $full = (Resolve-Path $path).Path
    $utf8 = New-Object System.Text.UTF8Encoding($false)
    $old = [System.IO.File]::ReadAllText($full, $utf8)
    $text = $old -replace $pattern, $replacement
    if ($text -ne $old) { [System.IO.File]::WriteAllText($full, $text, $utf8) }
}
Set-Version $cfgPath  '(?m)^(\s*version\s*=\s*")[^"]+(")'           ('${1}' + $Version + '${2}')
Set-Version $cargoTml '(?m)^(version\s*=\s*")[^"]+(")'             ('${1}' + $Version + '${2}')
Set-Version $tauriCfg '("version"\s*:\s*")[^"]+(")'               ('${1}' + $Version + '${2}')
Write-Host "[release] bumped installer.toml / Cargo.toml / tauri.conf.json"
# package.json / package-lock.json are not rewritten here (npm owns them): bump them first with
# `npm version <x.y.z> --no-git-tag-version`. The gate below names every source that disagrees.
Push-Location $BmmRoot
try {
    & node scripts/check-release-versions.mjs
    if ($LASTEXITCODE -ne 0) { throw "the BMM sources do not agree on $Version (see above)" }
} finally { Pop-Location }

# 2) Build BMM (frontend + exe) unless skipped.
if (-not $SkipBmmBuild) {
    Write-Host "[release] building BMM..."
    Push-Location $BmmRoot
    try { & npm run build; if ($LASTEXITCODE -ne 0) { throw "BMM build failed" } } finally { Pop-Location }
}

# 3) Pack + sign + stamp (+ emit a base update.json).
& ./examples/bmm/build-installer.ps1 -BmmRoot $BmmRoot -Config $cfgPath -Out $setup
if ($LASTEXITCODE -ne 0) { throw "build-installer failed" }

# 4) Delta from the previous release (if we have its package).
$assets = @($setup, $pkg, $manifest)
$deltas = @()
if (($prevVer -ne $Version) -and (Test-Path $prevPkg)) {
    $patch = Join-Path $archive ("{0}-to-{1}.patch" -f $prevVer, $Version)
    & $bpkg delta --old $prevPkg --new $pkg --out $patch
    if ($LASTEXITCODE -eq 0 -and (Test-Path $patch)) {
        $patchUrl = "https://github.com/FreeProject089/BetterModsManager/releases/download/v$Version/" + (Split-Path $patch -Leaf)
        $deltas += [ordered]@{ from = $prevVer; url = $patchUrl }
        $assets += $patch
        Write-Host "[release] delta $prevVer -> $Version created"
    }
}

# 5) Re-write update.json with the delta, SIGNED (bpkg update-manifest; same key as the
#    package, expires in 7 days -- renew weekly with `bpkg resign-manifest`).
#    Package mirrors come from [update].package_urls, as in build-installer.ps1.
$pkgUrl = "https://github.com/FreeProject089/BetterModsManager/releases/latest/download/bmm.bpkg"
$noteText = if ($Notes) { $Notes } else { "Better Mods Manager $Version" }
$umArgs = @("update-manifest", "--package", $pkg, "--config", $cfgPath, "--key", "examples/bmm/keys/private.key",
            "--url", $pkgUrl, "--notes", $noteText, "--installer", $setup, "--out", $manifest)
$cfgText = [System.IO.File]::ReadAllText((Resolve-Path $cfgPath).Path, (New-Object System.Text.UTF8Encoding($false)))
$pkgMatch = [regex]::Match($cfgText, '(?ms)^\s*package_urls\s*=\s*\[(.*?)\]')
if ($pkgMatch.Success) {
    foreach ($q in [regex]::Matches($pkgMatch.Groups[1].Value, '"([^"]+)"')) {
        $mu = $q.Groups[1].Value.Trim()
        if ($mu -and $mu -ne $pkgUrl) { $umArgs += @("--mirror", $mu) }
    }
}
foreach ($d in $deltas) { $umArgs += @("--delta", ("{0}={1}" -f $d.from, $d.url)) }
& $bpkg @umArgs
if ($LASTEXITCODE -ne 0) { throw "update-manifest failed" }
Write-Host "[release] wrote $manifest (signed)"
if ($ExtraSources.Count -gt 0) {
    Write-Host "[release] NOTE: also publish this update.json to your extra sources: $($ExtraSources -join ', ')"
}

# 6) Archive the NEW package for the next release's delta.
Copy-Item $pkg (Join-Path $archive ("bmm-{0}.bpkg" -f $Version)) -Force

# 7) BMM's OWN signed incremental manifest (update-manifest.json). A BMM copy that
#    BetterInstaller did not install (and every 1.0.x full-installer fallback) reads THIS file,
#    not update.json, and runs an installer only when the signed manifest lists it with its
#    SHA-256. Publishing without it is what made 1.0.x answer "Update refused: this release
#    publishes no signed update manifest". Generated from the setup just stamped, signed with
#    the same key under BMM's own context, and verified against the key BMM pins.
Push-Location $BmmRoot
try {
    $relAssets = Join-Path "dist" ("release-assets-v{0}" -f $Version)
    # A setup left there by an earlier run would be a second installer on the release.
    Get-ChildItem $relAssets -ErrorAction SilentlyContinue | Where-Object { $_.Name -match '\.(exe|msi)$' } | Remove-Item -Force
    & node scripts/gen-update-manifest.mjs --version $Version --installer Release/BMM-Setup.exe --require-installer
    if ($LASTEXITCODE -ne 0) { throw "gen-update-manifest failed" }
    $bmmManifest = Join-Path $relAssets "update-manifest.json"
    $keyFile = Join-Path (Get-Location) "BetterInstaller/examples/bmm/keys/private.key"
    if (-not (Test-Path $keyFile)) { $keyFile = Join-Path $PSScriptRoot "keys/private.key" }
    & node scripts/sign-update-manifest.mjs sign $bmmManifest --key $keyFile
    if ($LASTEXITCODE -ne 0) { throw "signing update-manifest.json failed" }
    & node scripts/sign-update-manifest.mjs verify $bmmManifest
    if ($LASTEXITCODE -ne 0) { throw "update-manifest.json does not verify against the key BMM pins" }
    # Every artefact states $Version, both manifests were signed NOW, ONE installer.
    $bpkgAbs = (Resolve-Path (Join-Path $PSScriptRoot "../../target/release/bpkg.exe")).Path
    & node scripts/check-release-versions.mjs --artifacts --bpkg $bpkgAbs
    if ($LASTEXITCODE -ne 0) { throw "release artefacts disagree (see above)" }
    $doc = Get-Content $bmmManifest -Raw | ConvertFrom-Json
    $assets += (Resolve-Path $bmmManifest).Path
    foreach ($f in @($doc.files)) {
        $name = ($f.download_url -split '/')[-1]
        $path = Join-Path $relAssets $name
        if (-not (Test-Path $path)) { throw "$name is named by update-manifest.json but absent from $relAssets" }
        $assets += (Resolve-Path $path).Path
    }
    $changelog = Join-Path (Get-Location) ("Update/changelog_v{0}_EN.md" -f $Version)
} finally { Pop-Location }

# ONE installer among the assets: BMM 0.9.11 has no version guard and runs the FIRST .msi/.exe
# of the release, so a second one is a second program it may launch.
$installersOut = @($assets | Where-Object { $_ -match '\.(exe|msi)$' })
if ($installersOut.Count -ne 1) { throw "exactly one installer asset expected, got: $($installersOut -join ', ')" }
# 0.9.11 falls back to the first ".exe" OR ".zip" asset, and its match is case-sensitive: a .zip
# on the release could be what it runs, and only a lowercase ".exe" is seen at all.
$zips = @($assets | Where-Object { $_ -match '\.zip$' })
if ($zips.Count -gt 0) { throw "no .zip asset on a BMM release (BMM 0.9.11 may run it): $($zips -join ', ')" }
if (-not ($installersOut[0] -cmatch 'BMM-Setup\.exe$')) { throw "the installer asset must be named BMM-Setup.exe (lowercase .exe), got: $($installersOut[0])" }

# 8) Optionally publish the GitHub release: every asset in ONE call (so no client ever sees the
#    release without its signed manifests), as Latest and NOT a pre-release: the stable channel
#    reads /releases/latest (v1.0.1 went out as a pre-release and was offered to nobody).
#    Both manifests expire 7 days after signing: publish right after this script, and let
#    BMM's resign-manifests.yml renew them twice a week.
Write-Host ""
Write-Host "[release] assets:"; $assets | ForEach-Object { Write-Host "    $_" }
$notesArgs = if (Test-Path $changelog) { @("--notes-file", $changelog) } else { @("--notes", $(if ($Notes) { $Notes } else { "BMM $Version" })) }
if ($Publish) {
    & gh release create "v$Version" $assets --title "BMM $Version" --latest @notesArgs
    if ($LASTEXITCODE -ne 0) { throw "gh release create failed" }
    Write-Host "[release] published v$Version (Latest)"
} else {
    Write-Host ""
    Write-Host "[release] dry run (no -Publish). To publish (within the manifests' 7-day validity):"
    Write-Host ("    gh release create v{0} {1} --title `"BMM {0}`" --latest {2}" -f $Version, ($assets -join ' '), ($notesArgs -join ' '))
}
