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
if ((Test-Path $pkg) -and -not (Test-Path $prevPkg)) {
    Copy-Item $pkg $prevPkg
    Write-Host "[release] archived previous package -> $prevPkg"
}

# 1) Bump the three version sources (keep them in lockstep).
function Set-Version($path, $pattern, $replacement) {
    (Get-Content $path -Raw) -replace $pattern, $replacement | Set-Content -Encoding utf8 $path
}
Set-Version $cfgPath  '(?m)^(\s*version\s*=\s*")[^"]+(")'           ('${1}' + $Version + '${2}')
Set-Version $cargoTml '(?m)^(version\s*=\s*")[^"]+(")'             ('${1}' + $Version + '${2}')
Set-Version $tauriCfg '("version"\s*:\s*")[^"]+(")'               ('${1}' + $Version + '${2}')
Write-Host "[release] bumped installer.toml / Cargo.toml / tauri.conf.json"

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
if (Test-Path $prevPkg) {
    $patch = Join-Path $archive ("{0}-to-{1}.patch" -f $prevVer, $Version)
    & $bpkg delta --old $prevPkg --new $pkg --out $patch
    if ($LASTEXITCODE -eq 0 -and (Test-Path $patch)) {
        $patchUrl = "https://github.com/FreeProject089/BetterModsManager/releases/download/v$Version/" + (Split-Path $patch -Leaf)
        $deltas += [ordered]@{ from = $prevVer; url = $patchUrl }
        $assets += $patch
        Write-Host "[release] delta $prevVer -> $Version created"
    }
}

# 5) Write the final multi-source-aware update.json (version + url + notes + deltas).
$pkgUrl = "https://github.com/FreeProject089/BetterModsManager/releases/latest/download/bmm.bpkg"
$man = [ordered]@{
    version = $Version
    url     = $pkgUrl
    notes   = if ($Notes) { $Notes } else { "Better Mods Manager $Version" }
}
if ($deltas.Count -gt 0) { $man.deltas = $deltas }
($man | ConvertTo-Json -Depth 5) | Set-Content -Encoding ASCII $manifest
Write-Host "[release] wrote $manifest"
if ($ExtraSources.Count -gt 0) {
    Write-Host "[release] NOTE: also publish this update.json to your extra sources: $($ExtraSources -join ', ')"
}

# 6) Archive the NEW package for the next release's delta.
Copy-Item $pkg (Join-Path $archive ("bmm-{0}.bpkg" -f $Version)) -Force

# 7) Optionally publish the GitHub release.
Write-Host ""
Write-Host "[release] assets:"; $assets | ForEach-Object { Write-Host "    $_" }
if ($Publish) {
    $body = if ($Notes) { $Notes } else { "BMM $Version" }
    & gh release create "v$Version" $assets --title "BMM $Version" --notes $body
    if ($LASTEXITCODE -ne 0) { throw "gh release create failed" }
    Write-Host "[release] published v$Version"
} else {
    Write-Host ""
    Write-Host "[release] dry run (no -Publish). To publish:"
    Write-Host ("    gh release create v{0} {1} --title `"BMM {0}`" --notes `"...`"" -f $Version, ($assets -join ' '))
}
