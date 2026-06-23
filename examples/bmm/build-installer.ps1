# Build a real BetterInstaller setup.exe for Better Mods Manager.
#
# Prereqs:
#   1. Build BMM release:   cd ..\..\..\src-tauri ; cargo build --release
#   2. Build the engine:    cd ..\.. ; cargo build --release -p bpkg-cli -p installer
# Then run this script from anywhere:  powershell -File build-installer.ps1
#
# Output: BMM-Setup.exe (a single self-extracting installer). Double-click to test
# the full flow: prerequisites → Welcome → Setup (handoff) → install → shortcuts +
# bmm:// protocol + uninstaller. Then BMM launches with no first-run modals.

$ErrorActionPreference = "Stop"
$here = $PSScriptRoot
$repo = (Resolve-Path "$here\..\..").Path     # BetterInstaller root
$bmm  = (Resolve-Path "$repo\..").Path        # BMM repo root

$exe     = "$bmm\src-tauri\target\release\better-mods-manager.exe"
$sidecar = "$bmm\src-tauri\binaries\bmm-mcp-server-x86_64-pc-windows-msvc.exe"
$bpkg    = "$repo\target\release\bpkg.exe"
$shell   = "$repo\target\release\betterinstaller.exe"
$config  = "$here\installer.toml"
$payload = "$here\payload"
$bpkgOut = "$here\bmm.bpkg"
$out     = "$here\BMM-Setup.exe"

if (-not (Test-Path $exe))   { throw "Build BMM release first → missing $exe  (cd src-tauri; cargo build --release)" }
if (-not (Test-Path $bpkg))  { throw "Build the engine first → missing $bpkg  (cargo build --release -p bpkg-cli -p installer)" }
if (-not (Test-Path $shell)) { throw "Build the engine first → missing $shell" }

Write-Host "Assembling payload…" -ForegroundColor Cyan
Remove-Item $payload -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $payload | Out-Null

Copy-Item $exe "$payload\better-mods-manager.exe"
if (Test-Path $sidecar) {
    Copy-Item $sidecar "$payload\bmm-mcp-server.exe"   # the optional 'mcp-server' component
} else {
    Write-Host "  (MCP sidecar not found — building without it)" -ForegroundColor Yellow
}

# Runtime resources BMM resolves next to the exe. Adjust if your layout differs.
foreach ($r in @("app.cfg","LICENSE.md","TOS.md","TOS_FR.md","PRIVACY.md","PRIVACY_FR.md")) {
    if (Test-Path "$bmm\$r") { Copy-Item "$bmm\$r" "$payload\$r" }
}
if (Test-Path "$bmm\Update") { Copy-Item "$bmm\Update" "$payload\Update" -Recurse }

Write-Host "Packaging…" -ForegroundColor Cyan
& $bpkg pack --root $payload --config $config --out $bpkgOut
if ($LASTEXITCODE -ne 0) { throw "bpkg pack failed" }

# --- Optional: sign the package (recommended for release) -------------------
# & $bpkg keygen --out "$here\keys"
# & $bpkg sign $bpkgOut --key "$here\keys\private.key"
#   …then paste keys\public.key's hex into installer.toml [security].public_key
# ----------------------------------------------------------------------------

Write-Host "Stamping single-exe installer…" -ForegroundColor Cyan
& $bpkg build --installer $shell --config $config --package $bpkgOut --out $out
if ($LASTEXITCODE -ne 0) { throw "bpkg build failed" }

Write-Host "`nDone → $out" -ForegroundColor Green
Write-Host "Double-click it to test the full install/uninstall flow." -ForegroundColor Green
