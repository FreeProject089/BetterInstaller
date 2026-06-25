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
    [string]$Out     = "examples/bmm/BMM-Setup.exe"
)
$ErrorActionPreference = "Stop"

$bpkg    = "./target/release/bpkg.exe"
$inst    = "./target/release/betterinstaller.exe"
$payload = "examples/bmm/payload"
$pkg     = "examples/bmm/bmm.bpkg"
$keyDir  = "examples/bmm/keys"
$priv    = "$keyDir/private.key"

# 1) Build the engine (release) if needed.
if (-not (Test-Path $bpkg) -or -not (Test-Path $inst)) {
    Write-Host "[1/5] Building the engine (release)..."
    cargo build --release -p bpkg-cli -p installer
}

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

# 4) Sign it (keygen if no key yet). public_key in installer.toml must match.
Write-Host "[4/5] Signing..."
if (-not (Test-Path $priv)) {
    Write-Host "      No key found -> generating one in $keyDir"
    & $bpkg keygen --out $keyDir
    Write-Host "      IMPORTANT: copy $keyDir/public.key into [security].public_key"
}
& $bpkg sign --key $priv $pkg

# 5) Stamp the self-extracting installer.
Write-Host "[5/5] Building $Out ..."
& $bpkg build --installer $inst --config $Config --package $pkg --out $Out

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
    $manifest = [ordered]@{
        version = $ver
        url     = $pkgUrl
        notes   = "Better Mods Manager $ver"
    }
    $manifestPath = "examples/bmm/update.json"
    ($manifest | ConvertTo-Json) | Set-Content -Encoding ASCII $manifestPath
    Write-Host "      update.json -> $manifestPath (version $ver)"
} else {
    Write-Host "      (skipped update.json: version/manifest_url not found in config)"
}

Write-Host ""
Write-Host "Done -> $Out"
Write-Host "Release upload: $Out (setup) + examples/bmm/bmm.bpkg + examples/bmm/update.json"
