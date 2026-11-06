# Builds the Windows packages in target\windows\:
#   Chamfera-<version>-windows-x86_64.zip        portable: unzip anywhere and run chamfera.exe
#   Chamfera-<version>-windows-x86_64-setup.exe  per-user installer (only when NSIS is installed)
# Needs: cargo (MSVC toolchain), ImageMagick (`magick`, for the icon). Optional: NSIS
# (`makensis`), for the installer; pass -RequireInstaller to fail when it is missing.
# Run from any directory: pwsh packaging/windows/build-windows.ps1
param([switch]$RequireInstaller)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$Root = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$Out = Join-Path $Root "target\windows"
New-Item -ItemType Directory -Force -Path $Out | Out-Null

$Meta = cargo metadata --no-deps --format-version 1 --manifest-path (Join-Path $Root "Cargo.toml") | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw "cargo metadata failed" }
$Version = ($Meta.packages | Where-Object { $_.name -eq "chamfera" }).version
if (-not $Version) { throw "no version for the chamfera package" }
$Name = "Chamfera-$Version-windows-x86_64"

# The exe icon, generated from the SVG (never committed); build.rs embeds it.
$Icon = Join-Path $Out "chamfera.ico"
magick -background none (Join-Path $Root "assets\icons\chamfera.svg") `
  -define icon:auto-resize=256,128,64,48,32,24,16 $Icon
if ($LASTEXITCODE -ne 0) { throw "magick failed to make the icon" }
$env:CHAMFERA_WINDOWS_ICON = $Icon

cargo build --release --manifest-path (Join-Path $Root "Cargo.toml") -p chamfera -p chamfera-cli
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

# Portable zip: one top-level folder with both exes, the licences and the attributions.
$Stage = Join-Path $Out $Name
if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
New-Item -ItemType Directory -Path $Stage | Out-Null
foreach ($f in @("target\release\chamfera.exe", "target\release\chamfera-cli.exe",
                 "LICENSE-MIT", "LICENSE-APACHE", "ATTRIBUTION.md")) {
  Copy-Item (Join-Path $Root $f) $Stage
}
$Zip = Join-Path $Out "$Name.zip"
if (Test-Path $Zip) { Remove-Item -Force $Zip }
Compress-Archive -Path $Stage -DestinationPath $Zip
Write-Output "built $Zip"

# Installer (NSIS, zlib/libpng licence): per-user, no administrator rights needed.
$Nsis = Get-Command makensis -ErrorAction SilentlyContinue
if (-not $Nsis) {
  foreach ($p in @("${env:ProgramFiles(x86)}\NSIS\makensis.exe", "$env:ProgramFiles\NSIS\makensis.exe")) {
    if (Test-Path $p) { $Nsis = Get-Command $p; break }
  }
}
if (-not $Nsis) {
  if ($RequireInstaller) { throw "makensis (NSIS) not found" }
  Write-Output "NSIS not found: skipping the installer"
  exit 0
}
$Setup = Join-Path $Out "$Name-setup.exe"
& $Nsis.Source /V2 "/DVERSION=$Version" "/DSRCDIR=$Stage" "/DICON=$Icon" "/DOUTFILE=$Setup" `
  (Join-Path $PSScriptRoot "chamfera.nsi")
if ($LASTEXITCODE -ne 0) { throw "makensis failed" }
Write-Output "built $Setup"
