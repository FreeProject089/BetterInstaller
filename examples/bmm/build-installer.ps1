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

# 2) Assemble the payload from BMM's build artifacts + legal docs.
Write-Host "[2/5] Assembling payload..."
$rel = Join-Path $BmmRoot "src-tauri/target/release"
if (Test-Path $payload) { Remove-Item -Recurse -Force $payload }
New-Item -ItemType Directory -Force $payload | Out-Null

Copy-Item (Join-Path $rel "better-mods-manager.exe") $payload
Copy-Item (Join-Path $rel "bmm-mcp-server.exe")      $payload
foreach ($doc in @("TOS.md","PRIVACY.md")) {
    $p = Join-Path $BmmRoot $doc
    if (Test-Path $p) { Copy-Item $p $payload }
}
# Bundled content for the pre-import options. Lands at <install>/presets/ and the
# first-run handoff copies it into BMM (languages -> Lang dir, themes -> themes dir).
$presets = Join-Path $payload "presets"
New-Item -ItemType Directory -Force (Join-Path $presets "Lang")   | Out-Null
New-Item -ItemType Directory -Force (Join-Path $presets "themes") | Out-Null

# BMM's language packs, so "Import extra languages" has real files to seed.
$langSrc = Join-Path $BmmRoot "frontend/Lang"
if (Test-Path $langSrc) {
    Get-ChildItem (Join-Path $langSrc "*.json") |
        Where-Object { $_.Name -ne "template.json" } |
        ForEach-Object { Copy-Item $_.FullName (Join-Path $presets "Lang") }
}
# Maintainer extras: drop more languages / themes / a full bmm-preset.json here.
$userBundle = "examples/bmm/bundle/presets"
if (Test-Path $userBundle) { Copy-Item -Recurse (Join-Path $userBundle "*") $presets }
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

Write-Host ""
Write-Host "Done -> $Out"
