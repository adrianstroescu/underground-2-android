# Builds build/ug2.apk without Gradle: cargo -> javac/d8 -> aapt2 -> zipalign -> apksigner.
#   powershell -ExecutionPolicy Bypass -File scripts\build-apk.ps1 [-Install] [-Run] [-DebugBuild] [-Emulator]
# -Emulator also packs an x86_64 library so the same APK runs in the Android emulator.
param(
    [switch]$Install,
    [switch]$Run,
    [switch]$DebugBuild,
    [switch]$Emulator
)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
$minSdk = 28
$targetSdk = 35
$package = "com.adrianstroescu.ug2"

function Latest($dir, $filter = "*") {
    if (-not (Test-Path $dir)) { return $null }
    Get-ChildItem $dir -Directory -Filter $filter | Sort-Object { [version]($_.Name -replace '[^0-9.]', '') } -ErrorAction SilentlyContinue | Select-Object -Last 1
}

$sdk = $env:ANDROID_HOME
if (-not $sdk) { $sdk = $env:ANDROID_SDK_ROOT }
if (-not $sdk) { $sdk = "$env:LOCALAPPDATA\Android\Sdk" }
$ndk = $env:ANDROID_NDK_HOME
if (-not $ndk) { $ndk = (Latest "$sdk\ndk").FullName }
$buildTools = (Latest "$sdk\build-tools").FullName
$platform = (Get-ChildItem "$sdk\platforms" -Directory -Filter "android-*" | Sort-Object { [int]($_.Name -replace '\D', '') } | Select-Object -Last 1).FullName
if (-not $ndk -or -not $buildTools -or -not $platform) { throw "Android SDK/NDK not found (sdk=$sdk)" }
Write-Host "SDK $sdk`nNDK $ndk`nbuild-tools $buildTools`nplatform $platform"
$llvm = "$ndk\toolchains\llvm\prebuilt\windows-x86_64\bin"

# Host build scripts on the windows-gnu toolchain (no Visual Studio) need `dlltool`;
# the NDK's llvm-dlltool understands the same arguments when named dlltool.exe.
if (-not (Get-Command dlltool.exe -ErrorAction SilentlyContinue)) {
    $tools = "$root\build\tools"
    New-Item -ItemType Directory -Force $tools | Out-Null
    Copy-Item "$llvm\llvm-dlltool.exe" "$tools\dlltool.exe" -Force
    $env:Path = "$tools;$env:Path"
}

$abis = @(@{ abi = "arm64-v8a"; target = "aarch64-linux-android" })
if ($Emulator) { $abis += @{ abi = "x86_64"; target = "x86_64-linux-android" } }

$profile = if ($DebugBuild) { "debug" } else { "release" }
$out = "$root\build"
$stage = "$out\stage"
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }

foreach ($a in $abis) {
    $t = $a.target
    $envName = $t.ToUpper().Replace("-", "_")
    $clang = "$llvm\$t$minSdk-clang.cmd"
    Set-Item "env:CARGO_TARGET_${envName}_LINKER" $clang
    Set-Item "env:CC_$($t.Replace('-', '_'))" $clang
    Set-Item "env:AR_$($t.Replace('-', '_'))" "$llvm\llvm-ar.exe"
    $cargoArgs = @("build", "-p", "ug2-android", "--target", $t)
    if (-not $DebugBuild) { $cargoArgs += "--release" }
    Push-Location $root
    try {
        & cargo @cargoArgs
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed for $t" }
    } finally { Pop-Location }
    New-Item -ItemType Directory -Force "$stage\lib\$($a.abi)" | Out-Null
    & "$llvm\llvm-strip.exe" --strip-unneeded -o "$stage\lib\$($a.abi)\libug2.so" "$root\target\$t\$profile\libug2.so"
    if ($LASTEXITCODE -ne 0) { throw "strip failed" }
}

# MainActivity.java: a NativeActivity subclass whose only job is immersive full screen.
$jdk = if ($env:JAVA_HOME) { "$env:JAVA_HOME\bin" } else { Split-Path (Get-Command javac.exe).Source }
$classes = "$stage\classes"
New-Item -ItemType Directory -Force $classes | Out-Null
$sources = @(Get-ChildItem "$root\android\java" -Recurse -Filter *.java | ForEach-Object FullName)
& "$jdk\javac.exe" --release 11 -nowarn -cp "$platform\android.jar" -d $classes @sources
if ($LASTEXITCODE -ne 0) { throw "javac failed" }
$classFiles = @(Get-ChildItem $classes -Recurse -Filter *.class | ForEach-Object FullName)
& "$buildTools\d8.bat" --release --min-api $minSdk --lib "$platform\android.jar" --output $stage @classFiles
if ($LASTEXITCODE -ne 0) { throw "d8 failed" }

$unsigned = "$out\ug2-unsigned.apk"
$aligned = "$out\ug2-aligned.apk"
$apk = "$out\ug2.apk"
Remove-Item -Force -ErrorAction SilentlyContinue $unsigned, $aligned, $apk
& "$buildTools\aapt2.exe" link -o $unsigned --manifest "$root\android\AndroidManifest.xml" -I "$platform\android.jar" `
    --min-sdk-version $minSdk --target-sdk-version $targetSdk
if ($LASTEXITCODE -ne 0) { throw "aapt2 link failed" }

Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem
$zip = [System.IO.Compression.ZipFile]::Open($unsigned, [System.IO.Compression.ZipArchiveMode]::Update)
try {
    [System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip, "$stage\classes.dex", "classes.dex", [System.IO.Compression.CompressionLevel]::Optimal) | Out-Null
    foreach ($a in $abis) {
        [System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip, "$stage\lib\$($a.abi)\libug2.so", "lib/$($a.abi)/libug2.so", [System.IO.Compression.CompressionLevel]::Optimal) | Out-Null
    }
} finally { $zip.Dispose() }

& "$buildTools\zipalign.exe" -f 4 $unsigned $aligned
if ($LASTEXITCODE -ne 0) { throw "zipalign failed" }

$keystore = "$env:USERPROFILE\.android\debug.keystore"
if (-not (Test-Path $keystore)) {
    New-Item -ItemType Directory -Force (Split-Path $keystore) | Out-Null
    $keytool = if ($env:JAVA_HOME) { "$env:JAVA_HOME\bin\keytool.exe" } else { "keytool" }
    & $keytool -genkeypair -v -keystore $keystore -storepass android -alias androiddebugkey -keypass android `
        -keyalg RSA -keysize 2048 -validity 10000 -dname "CN=Android Debug,O=Android,C=US"
}
& "$buildTools\apksigner.bat" sign --ks $keystore --ks-pass pass:android --key-pass pass:android --out $apk $aligned
if ($LASTEXITCODE -ne 0) { throw "apksigner failed" }
Remove-Item -Force -ErrorAction SilentlyContinue $unsigned, $aligned, "$apk.idsig"
$size = [math]::Round((Get-Item $apk).Length / 1MB, 1)
Write-Host "APK: $apk ($size MB)"

if ($Install -or $Run) {
    & adb install -r $apk
    if ($LASTEXITCODE -ne 0) { throw "adb install failed" }
}
if ($Run) {
    & adb shell am start -n "$package/.MainActivity"
    Write-Host "Logs: adb logcat -s ug2:* RustStdoutStderr:*"
}
