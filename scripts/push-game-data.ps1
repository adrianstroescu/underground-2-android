# Copies the original game files from your PC install to the phone over adb.
#   powershell -ExecutionPolicy Bypass -File scripts\push-game-data.ps1 -GameDir "D:\Games\NFSU2" [-Car 350Z] [-Full]
# Default copies only what the current build loads (~150 MB); -Full copies CARS, GLOBAL,
# TRACKS, FRONTEND, LANGUAGES, SOUND (~1.3 GB) for later milestones.
param(
    [string]$GameDir = (Split-Path (Split-Path $PSScriptRoot -Parent) -Parent),
    [string]$Car = "350Z",
    [switch]$Full
)
$ErrorActionPreference = "Stop"
$package = "com.adrianstroescu.ug2"
$dest = "/sdcard/Android/data/$package/files/NFSU2"

if (-not (Test-Path "$GameDir\TRACKS\L4RA.BUN")) { throw "Not a UG2 install: $GameDir" }
& adb get-state | Out-Null
if ($LASTEXITCODE -ne 0) { throw "No device: connect the phone with USB debugging enabled" }

# Launching once makes Android create the app's data folder with the right owner.
& adb shell am start -n "$package/android.app.NativeActivity" | Out-Null
Start-Sleep -Seconds 2
& adb shell mkdir -p "$dest/CARS/$Car" "$dest/GLOBAL" "$dest/TRACKS"

function Push($rel) {
    $src = Join-Path $GameDir $rel
    if (-not (Test-Path $src)) { Write-Warning "missing $rel"; return }
    $target = "$dest/" + ($rel -replace '\\', '/')
    Write-Host "-> $rel"
    if ((Get-Item $src).PSIsContainer) {
        & adb push $src (Split-Path $target -Parent).Replace('\', '/') | Select-Object -Last 1
    } else {
        & adb push $src $target | Select-Object -Last 1
    }
    if ($LASTEXITCODE -ne 0) { throw "adb push failed for $rel" }
}

if ($Full) {
    foreach ($d in "CARS", "GLOBAL", "TRACKS", "FRONTEND", "LANGUAGES", "SOUND") { Push $d }
} else {
    Push "CARS\$Car\GEOMETRY.BIN"
    Push "CARS\$Car\TEXTURES.BIN"
    Push "CARS\TEXTURES.BIN"
    if (Test-Path "$GameDir\GLOBAL\GLOBALB.BUN") { Push "GLOBAL\GLOBALB.BUN" } else { Push "GLOBAL\GlobalB.lzc" }
    Push "GLOBAL\DYNTEX.BIN"
    Push "TRACKS\L4RA.BUN"
    Push "TRACKS\STREAML4RA.BUN"
    Push "TRACKS\LOC4DYNTEX.BIN"
}
# Folders created by adb belong to the shell user (mode 2770); open them up so the
# app's own user can read them.
& adb shell chmod -R a+rwX $dest
& adb shell am force-stop $package
Write-Host "Done. Start 'NFS Underground 2' on the phone."
