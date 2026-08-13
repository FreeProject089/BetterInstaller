# =====================================================================
#  Build the Better Mods Manager installer with BetterInstaller.
#  Assembles the payload -> packs a .bpkg -> SIGNS it -> stamps the SFX.
#
#  Run from the BetterInstaller root:  ./examples/bmm/build-installer.ps1
#  ASCII-only on purpose (Windows PowerShell 5.1 chokes on non-ASCII).
# =====================================================================
param(
    [string]$BmmRoot = "..",                       # BetterModsManager repo root
    [string]$Config  = "examples/bmm/installer.toml",
    [string]$Out     = ""                          # default: <BmmRoot>/Release/BMM-Setup.exe
)
$ErrorActionPreference = "Stop"

$bpkg    = "./target/release/bpkg.exe"
$inst    = "./target/release/betterinstaller.exe"
$payload = "examples/bmm/payload"          # build temp (stays here)
$keyDir  = "examples/bmm/keys"             # signing key (stays here)
$priv    = "$keyDir/private.key"
# Distributable artifacts (setup + bpkg + update.json) go into <BmmRoot>/Release (gitignored).
$relDir  = Join-Path $BmmRoot "Release"
New-Item -ItemType Directory -Force $relDir | Out-Null
$pkg     = Join-Path $relDir "bmm.bpkg"
if (-not $Out) { $Out = Join-Path $relDir "BMM-Setup.exe" }

# 1) Build the engine (release) if needed.
# Always build: cargo is incremental, so this is nearly free when up to date, and
# testing for mere PRESENCE is a trap. Stale exes from an older commit sat here while
# installer.toml had moved on, and the build died with "unknown variant swatch" --
# a config error pointing at a perfectly valid config, because the binary reading it
# predated the feature.
Write-Host "[1/5] Building the engine (release)..."
cargo build --release -p bpkg-cli -p installer
if ($LASTEXITCODE -ne 0) { throw "engine build failed" }

# 2) Assemble the payload = the EXACT Tauri runtime layout, so the installed app
#    has EVERYTHING (exe + sidecar + the whole resource tree: Lang, tutorial assets,
#    builtin themes, Update docs, app.cfg, legal…). Just the exes is not enough.
Write-Host "[2/5] Assembling payload..."
$rel = Join-Path $BmmRoot "src-tauri/target/release"
if (-not (Test-Path (Join-Path $rel "better-mods-manager.exe"))) {
    throw "BMM is not built. Run 'npm run build' (or 'npx tauri build --no-bundle') first."
}
if (Test-Path $payload) { Remove-Item -Recurse -Force $payload }
New-Item -ItemType Directory -Force $payload | Out-Null

Copy-Item (Join-Path $rel "better-mods-manager.exe") $payload
Copy-Item (Join-Path $rel "bmm-mcp-server.exe")      $payload
# Tauri resource trees: "_up_" holds ../ resources (frontend/Lang, assets, Update,
# legal, app.cfg); "resources" holds icon etc. This mirrors how the app runs from
# target/release, so resolve_path() finds everything (incl. en/fr languages).
foreach ($d in @("_up_", "resources")) {
    $src = Join-Path $rel $d
    if (Test-Path $src) { Copy-Item -Recurse $src (Join-Path $payload $d) }
}
# Legal docs at the payload ROOT too — the installer's Terms step reads TOS.md /
# PRIVACY.md from the package root (separate from the in-app copies under _up_).
foreach ($doc in @("TOS.md","PRIVACY.md")) {
    $p = Join-Path $BmmRoot $doc
    if (Test-Path $p) { Copy-Item $p $payload }
}

# App logo for the installer sidebar ([branding].logo = "assets/logo.png").
$logoSrc = Join-Path $BmmRoot "src-tauri/icons/128x128.png"
if (Test-Path $logoSrc) {
    New-Item -ItemType Directory -Force (Join-Path $payload "assets") | Out-Null
    Copy-Item $logoSrc (Join-Path $payload "assets/logo.png")
}
# Optional pre-import EXTRAS (languages BEYOND en/fr, themes) -> <install>/presets/.
# en/fr ship inside the app above and are always present; this is opt-in additions.
$presets    = Join-Path $payload "presets"

# Bundle BMM's built-in themes so the "Import themes" setup option seeds them on first
# run: payload presets/themes/ -> <install>/presets/themes/ -> the handoff consumer
# copies them into the user themes dir when 'import_starter_themes' stays checked.
$themesSrc = Join-Path $BmmRoot "frontend/assets/builtin-themes"
if (Test-Path $themesSrc) {
    $themesDst = Join-Path $presets "themes"
    New-Item -ItemType Directory -Force $themesDst | Out-Null
    Copy-Item (Join-Path $themesSrc "*.json") $themesDst
    Write-Host ("      bundled {0} themes for import" -f (Get-ChildItem $themesDst -Filter *.json).Count)
}

$userBundle = "examples/bmm/bundle/presets"
if (Test-Path $userBundle) {
    New-Item -ItemType Directory -Force $presets | Out-Null
    Copy-Item -Recurse (Join-Path $userBundle "*") $presets
}
if (Test-Path "examples/bmm/bundle/bmm-preset.json") {
    Copy-Item "examples/bmm/bundle/bmm-preset.json" $payload
}

# 3) Pack the .bpkg.
Write-Host "[3/5] Packing $pkg ..."
& $bpkg pack --root $payload --config $Config --out $pkg
if ($LASTEXITCODE -ne 0) { throw "pack failed" }

# 4) Sign it (keygen if no key yet). public_key in installer.toml must match.
Write-Host "[4/5] Signing..."
if (-not (Test-Path $priv)) {
    Write-Host "      No key found -> generating one in $keyDir"
    & $bpkg keygen --out $keyDir
    Write-Host "      IMPORTANT: copy $keyDir/public.key into [security].public_key"
}
& $bpkg sign --key $priv $pkg
if ($LASTEXITCODE -ne 0) { throw "sign failed" }

# 5) Stamp the self-extracting installer.
Write-Host "[5/5] Building $Out ..."
& $bpkg build --installer $inst --config $Config --package $pkg --out $Out
if ($LASTEXITCODE -ne 0) { throw "build failed" }

# 6) Emit update.json — the auto-update manifest. Upload BOTH this file and the
#    signed bmm.bpkg to the GitHub release so installed copies can auto-update.
#    (manifest_url in installer.toml points at .../releases/latest/download/update.json)
$cfgText = Get-Content $Config -Raw
$verMatch = [regex]::Match($cfgText, '(?m)^\s*version\s*=\s*"([^"]+)"')
$urlMatch = [regex]::Match($cfgText, '(?m)^\s*manifest_url\s*=\s*"([^"]+)"')
if ($verMatch.Success -and $urlMatch.Success) {
    $ver    = $verMatch.Groups[1].Value
    # The .bpkg lives next to update.json in the same release.
    $pkgUrl = ($urlMatch.Groups[1].Value -replace 'update\.json$', 'bmm.bpkg')
    # Package mirrors, listed explicitly in [update].package_urls.
    #
    # This used to DERIVE them by rewriting "update.json" to "bmm.bpkg" in each manifest_urls
    # entry. That only holds for a GitHub release, where the two files sit side by side under the
    # same name. It is wrong for the shape BCWEB actually serves — /api/assets/<key>, where the
    # package lives under a different key entirely — so the guess would have produced a URL that
    # 404s. Declaring them beats inferring them.
    #
    # bpkg-core tries `url` first, then each of these. Safe by construction: the Ed25519
    # signature is verified before anything is applied, so a mirror can serve a bad file and
    # never get it installed.
    $mirrors = @()
    $pkgMatch = [regex]::Match($cfgText, '(?ms)^\s*package_urls\s*=\s*\[(.*?)\]')
    if ($pkgMatch.Success) {
        foreach ($q in [regex]::Matches($pkgMatch.Groups[1].Value, '"([^"]+)"')) {
            $m = $q.Groups[1].Value.Trim()
            if ($m -and $m -ne $pkgUrl) { $mirrors += $m }
        }
    }
    $manifest = [ordered]@{
        version = $ver
        url     = $pkgUrl
        notes   = "Better Mods Manager $ver"
    }
    if ($mirrors.Count -gt 0) {
        $manifest.urls = @($mirrors)
        Write-Host ("      {0} package mirror(s) in update.json" -f $mirrors.Count)
    }
    $manifestPath = Join-Path $relDir "update.json"
    ($manifest | ConvertTo-Json) | Set-Content -Encoding ASCII $manifestPath
    Write-Host "      update.json -> $manifestPath (version $ver)"
} else {
    Write-Host "      (skipped update.json: version/manifest_url not found in config)"
}

Write-Host ""
Write-Host "Done -> $Out"
Write-Host ("Release upload: {0} (setup) + {1} + {2}" -f $Out, $pkg, (Join-Path $relDir "update.json"))
