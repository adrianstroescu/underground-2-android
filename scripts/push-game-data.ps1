# Copies the original game files from your PC install to the phone over adb.
#   powershell -ExecutionPolicy Bypass -File scripts\push-game-data.ps1 -GameDir "D:\Games\NFSU2" [-Full]
# Default copies what the current build uses (~2.5 GB: the movies, all cars, GLOBAL, Bayview and the
# drag, drift, Street X and URL tracks with their routes, sound and music); -Full also
# copies FRONTEND, LANGUAGES and the unused regions. Files already on the phone are
# skipped (--sync), so running it again after an update only sends what is new.
# Set ANDROID_SERIAL when more than one device or emulator is attached.
param(
    [string]$GameDir = (Split-Path (Split-Path $PSScriptRoot -Parent) -Parent),
    [switch]$Full,
    [switch]$NoMovies
)
$ErrorActionPreference = "Stop"
$package = "com.adrianstroescu.ug2"
$dest = "/sdcard/Android/data/$package/files/NFSU2"

if (-not (Test-Path "$GameDir\TRACKS\L4RA.BUN")) { throw "Not a UG2 install: $GameDir" }
& adb get-state | Out-Null
if ($LASTEXITCODE -ne 0) { throw "No device: connect the phone with USB debugging enabled" }

# Launching once makes Android create the app's data folder with the right owner.
& adb shell am start -n "$package/.MainActivity" | Out-Null
Start-Sleep -Seconds 2
& adb shell am force-stop $package
& adb shell mkdir -p "$dest/TRACKS"

function Push($rel) {
    $src = Join-Path $GameDir $rel
    if (-not (Test-Path $src)) { Write-Warning "missing $rel"; return }
    $target = "$dest/" + ($rel -replace '\\', '/')
    Write-Host "-> $rel"
    if ((Get-Item $src).PSIsContainer) { $target = (Split-Path $target -Parent).Replace('\', '/') }
    # Large pushes over USB occasionally abort mid-transfer ("write failed"); retry.
    foreach ($try in 1..3) {
        & adb push --sync $src $target | Select-Object -Last 1
        if ($LASTEXITCODE -eq 0) { return }
        Write-Warning "push failed, retry $try"
    }
    throw "adb push failed for $rel"
}

foreach ($d in "CARS", "GLOBAL", "SOUND", "pfdata", "FRONTEND", "LANGUAGES") { Push $d }
# The movies (~0.9 GB): EA/THX logos, the opening and the story scenes.
if (-not $NoMovies) { Push "MOVIES" }
if ($Full) {
    foreach ($d in "TRACKS") { Push $d }
} else {
    Push "TRACKS\LOC4DYNTEX.BIN"
    # Bayview, then the race-only regions: drags (B, D), drift (C), Street X (F), URL (G).
    foreach ($r in "L4RA", "L4RB", "L4RC", "L4RD", "L4RF", "L4RG") {
        foreach ($f in "$r.BUN", "STREAM$r.BUN", "ROUTES$r") { Push "TRACKS\$f" }
    }
    Get-ChildItem "$GameDir\TRACKS" -Filter "TRACKMAP4*.BIN" | ForEach-Object { Push "TRACKS\$($_.Name)" }
}
# Folders created by adb belong to the shell user (mode 2770); open them up so the
# app's own user can read them. NFSU2 itself belongs to the app and can't be chmod-ed.
& adb shell "chmod -R a+rwX $dest/*"
Write-Host "Done. Start 'NFS Underground 2' on the phone."
