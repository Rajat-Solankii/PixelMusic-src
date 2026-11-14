#Requires -Version 7.0
<#
.SYNOPSIS
    Builds PixelMusic (Rust + Qt 6.12) for Windows: the exe, a portable folder and an Inno Setup installer.

.DESCRIPTION
    1. Finds Qt 6.12 MSVC (qmake6.exe, windeployqt.exe) and sets QMAKE/PATH for cxx-qt.
    2. cargo build --release -p pixelmusic (or -CargoProfile debug).
    3. Assembles target\dist\pixelmusic\: pixelmusic.exe, qml\ (src\pixelmusic\qml), assets\ (fonts,
       icons), Qt through windeployqt --qmldir, the MSVC runtime.
    4. Builds the Pixel Music installer (installer\windows\Setup.cs, compiled with the csc.exe of the .NET
       Framework that ships with Windows; the app folder rides inside as payload.zip)
       -> target\dist\PixelMusic-<version>-Setup.exe, and its uninstaller.
    5. Also builds the previous Inno Setup installer -> target\dist\PixelMusic-<version>-Setup-inno.exe
       (kept while the new installer is being proven; skipped when Inno Setup isn't installed).

    Audio is played by librespot inside the exe (cpal/WASAPI opens the device); there are no external player DLLs.

.PARAMETER QtDir
    Qt prefix (the directory containing bin\qmake6.exe). Default: $env:QTDIR, the directory of $env:QMAKE,
    C:\Qt\6.12.0\msvc2022_64, ~\Qt\6.12.0\msvc2022_64, ~\Qt\6.12.0.

.PARAMETER Version
    Installer version. Default: [workspace.package] version from Cargo.toml.

.PARAMETER OutDir
    Output directory. Default: target\dist.

.PARAMETER CargoProfile
    Cargo profile: release (default) or debug.

.PARAMETER SkipBuild
    Do not run cargo (use an existing target\<profile>\pixelmusic.exe).

.PARAMETER SkipInstaller
    Skip both installer steps.

.PARAMETER SkipInno
    Skip only the Inno Setup installer.

.PARAMETER Portable
    Also pack the folder into PixelMusic-<version>-windows-x64-portable.zip.
#>
param(
    [string]$QtDir,
    [string]$Version,
    [string]$OutDir,
    [ValidateSet("release", "debug")]
    [string]$CargoProfile = "release",
    [switch]$SkipBuild,
    [switch]$SkipInstaller,
    [switch]$SkipInno,
    [switch]$Portable
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
# UTF-8 output in any console code page (cp866/cp1251)
try { [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false) } catch { }

$AppName = "Pixel Music"
$ExeName = "pixelmusic.exe"
$FilePrefix = "PixelMusic"

$ProjectRoot = $PSScriptRoot
if (-not $OutDir) { $OutDir = Join-Path $ProjectRoot "target\dist" }
$OutDir = [IO.Path]::GetFullPath($OutDir)
$Stage = Join-Path $OutDir "pixelmusic"

function Write-Step { param([string]$msg) Write-Host "`n==> $msg" -ForegroundColor Cyan }
function Write-Ok   { param([string]$msg) Write-Host "    OK: $msg" -ForegroundColor Green }
function Write-Warn { param([string]$msg) Write-Host "    WARN: $msg" -ForegroundColor Yellow }
function Abort      { param([string]$msg) Write-Host "`nERROR: $msg" -ForegroundColor Red; exit 1 }

# ─── Версия ───────────────────────────────────────────────────────────────────
if (-not $Version) {
    $cargoToml = Get-Content (Join-Path $ProjectRoot "Cargo.toml") -Raw
    if ($cargoToml -match '(?ms)^\[workspace\.package\].*?^version\s*=\s*"([^"]+)"') {
        $Version = $Matches[1]
    } else {
        $Version = "0.0.0"
    }
}
$Version = $Version.TrimStart("v")
Write-Host "$AppName $Version → $OutDir" -ForegroundColor White

# ─── 1. Qt ────────────────────────────────────────────────────────────────────
Write-Step "Looking for Qt 6.12 (MSVC)"
$candidates = @()
if ($QtDir) { $candidates += $QtDir }
if ($env:QTDIR) { $candidates += $env:QTDIR }
if ($env:QMAKE) { $candidates += (Split-Path (Split-Path $env:QMAKE)) }
$candidates += @(
    "C:\Qt\6.12.0\msvc2022_64",
    (Join-Path $HOME "Qt\6.12.0\msvc2022_64"),
    (Join-Path $HOME "Qt\6.12.0")
)
$QtDir = $null
foreach ($c in $candidates) {
    if ($c -and (Test-Path (Join-Path $c "bin\qmake6.exe"))) { $QtDir = (Resolve-Path $c).Path; break }
}
if (-not $QtDir) {
    Abort ("Qt 6.12 MSVC not found. Pass -QtDir or install it:`n" +
        "    python .github\scripts\install_qt.py --out C:\Qt\6.12.0\msvc2022_64")
}
$Qmake = Join-Path $QtDir "bin\qmake6.exe"
$WinDeployQt = Join-Path $QtDir "bin\windeployqt.exe"
if (-not (Test-Path $WinDeployQt)) { $WinDeployQt = Join-Path $QtDir "bin\windeployqt6.exe" }
if (-not (Test-Path $WinDeployQt)) { Abort "windeployqt not found in $QtDir\bin" }
$env:QMAKE = $Qmake
$env:QTDIR = $QtDir
$env:PATH = "$QtDir\bin;$env:PATH"
Write-Ok "$QtDir ($(& $Qmake -query QT_VERSION))"

# ─── 2. cargo build ───────────────────────────────────────────────────────────
$Exe = Join-Path $ProjectRoot "target\$CargoProfile\$ExeName"
if (-not $SkipBuild) {
    Write-Step "cargo build ($CargoProfile) -p pixelmusic"
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        Abort "cargo not found. Install Rust: https://rustup.rs (toolchain stable-x86_64-pc-windows-msvc)"
    }
    Push-Location $ProjectRoot
    try {
        # --locked: Cargo.lock держит vergen 9.0.6 (9.1 ломает build-скрипт librespot-core)
        if ($CargoProfile -eq "release") {
            cargo build --locked --release -p pixelmusic --bin pixelmusic
        } else {
            cargo build --locked -p pixelmusic --bin pixelmusic
        }
        if ($LASTEXITCODE -ne 0) { Abort "cargo build failed" }
    } finally {
        Pop-Location
    }
}
if (-not (Test-Path $Exe)) { Abort "$Exe not found" }
Write-Ok $Exe

# ─── 3. Папка дистрибутива ────────────────────────────────────────────────────
Write-Step "Assembling $Stage"
if (Test-Path $Stage) { Remove-Item $Stage -Recurse -Force }
New-Item -ItemType Directory -Force $Stage | Out-Null
Copy-Item $Exe $Stage

# QML и ресурсы — рядом с exe: оболочка ищет <exe>\qml, assets — соседний каталог
$srcQml = Join-Path $ProjectRoot "src\pixelmusic\qml"
$srcAssets = Join-Path $ProjectRoot "src\pixelmusic\assets"
Copy-Item $srcQml (Join-Path $Stage "qml") -Recurse
Get-ChildItem (Join-Path $Stage "qml") -Recurse -Directory -Filter "__pycache__" |
    Remove-Item -Recurse -Force
$assets = Join-Path $Stage "assets"
New-Item -ItemType Directory -Force $assets | Out-Null
Copy-Item (Join-Path $srcAssets "fonts") (Join-Path $assets "fonts") -Recurse   # fonts-src (исходники шрифтов) не нужен
Copy-Item (Join-Path $srcAssets "logo") (Join-Path $assets "logo") -Recurse     # логотипы Pixel Music по цветам (боковая панель, иконка окна)
Get-ChildItem $srcAssets -File | Where-Object { $_.Extension -in ".svg", ".png" } |
    Copy-Item -Destination $assets

$deployArgs = @("--qmldir", $srcQml, "--no-opengl-sw", "--no-compiler-runtime", "--translations", "ru,pt")   # Qt's own strings for the Russian and Portuguese UI; English needs no catalog
if ($CargoProfile -eq "release") { $deployArgs = @("--release") + $deployArgs }
& $WinDeployQt @deployArgs (Join-Path $Stage $ExeName)
if ($LASTEXITCODE -ne 0) { Abort "windeployqt failed with exit code $LASTEXITCODE" }
Write-Ok "Qt deployed by windeployqt"

# Рантайм MSVC (vcruntime140*.dll, msvcp140*.dll) кладём рядом с exe — без vc_redist у пользователя
$vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
$crtDir = $null
if (Test-Path $vswhere) {
    $vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
        -property installationPath | Select-Object -First 1
    if ($vs) {
        $crtDir = Get-ChildItem (Join-Path $vs "VC\Redist\MSVC") -Directory -ErrorAction SilentlyContinue |
            Sort-Object { try { [version]$_.Name } catch { [version]"0.0" } } -Descending |
            ForEach-Object { Get-ChildItem (Join-Path $_.FullName "x64") -Directory -Filter "Microsoft.VC*.CRT" -ErrorAction SilentlyContinue } |
            Select-Object -First 1
    }
}
if ($crtDir) {
    Copy-Item (Join-Path $crtDir.FullName "*.dll") $Stage
    Write-Ok "MSVC runtime: $($crtDir.FullName)"
} else {
    Write-Warn "MSVC runtime not found: a clean system will need the Visual C++ Redistributable"
}

$sizeMB = [math]::Round(((Get-ChildItem $Stage -Recurse -File | Measure-Object Length -Sum).Sum) / 1MB, 0)
Write-Ok "folder ready: $Stage ($sizeMB MB)"

if ($Portable) {
    $zip = Join-Path $OutDir "$FilePrefix-$Version-windows-x64-portable.zip"
    if (Test-Path $zip) { Remove-Item $zip -Force }
    Compress-Archive -Path (Join-Path $Stage "*") -DestinationPath $zip -CompressionLevel Optimal
    Write-Ok "portable archive: $zip"
}

if ($SkipInstaller) {
    Write-Host "`nDone: $Stage\$ExeName (installers skipped)" -ForegroundColor Green
    exit 0
}

# ─── 4. Pixel Music installer (installer\windows\Setup.cs) ────────────────────
Write-Step "Building the Pixel Music installer"
$installerSrc = Join-Path $ProjectRoot "installer\windows"
$csc = Join-Path $env:WINDIR "Microsoft.NET\Framework64\v4.0.30319\csc.exe"
if (-not (Test-Path $csc)) { Abort "csc.exe of .NET Framework 4 not found: $csc" }
$work = Join-Path $OutDir "installer-work"
if (Test-Path $work) { Remove-Item $work -Recurse -Force }
New-Item -ItemType Directory -Force $work | Out-Null

$payload = Join-Path $work "payload.zip"
Add-Type -AssemblyName System.IO.Compression.FileSystem
[IO.Compression.ZipFile]::CreateFromDirectory($Stage, $payload, [IO.Compression.CompressionLevel]::Optimal, $false)
Write-Ok "payload.zip: $([math]::Round((Get-Item $payload).Length / 1MB, 0)) MB"

$versionTxt = Join-Path $work "version.txt"
[IO.File]::WriteAllText($versionTxt, $Version)
$logoPng = Join-Path $installerSrc "logo.png"
$icon = Join-Path $ProjectRoot "pixelmusic.ico"
$setupCs = Join-Path $installerSrc "Setup.cs"
$refs = "/reference:System.IO.Compression.dll,System.IO.Compression.FileSystem.dll"
# the app's typeface (Google Sans, fixed weights cut from the variable font) for the installer's text
$fontRes = @("Regular", "Medium", "Bold") | ForEach-Object { "/resource:" + (Join-Path $installerSrc "fonts\GoogleSans-$_.ttf") + ",GoogleSans-$_.ttf" }

$uninstallExe = Join-Path $work "Uninstall.exe"
$out = & $csc /nologo /target:winexe /optimize+ /define:UNINSTALLER "/win32icon:$icon" $refs `
    "/resource:$logoPng,logo.png" $fontRes "/resource:$versionTxt,version.txt" "/out:$uninstallExe" $setupCs 2>&1
if ($LASTEXITCODE -ne 0) { $out | Write-Host; Abort "Uninstall.exe didn't compile" }

$setupExe = Join-Path $OutDir "$FilePrefix-$Version-Setup.exe"
if (Test-Path $setupExe) { Remove-Item $setupExe -Force }
$out = & $csc /nologo /target:winexe /optimize+ "/win32icon:$icon" $refs `
    "/resource:$payload,payload.zip" "/resource:$uninstallExe,Uninstall.exe" `
    "/resource:$logoPng,logo.png" $fontRes "/resource:$versionTxt,version.txt" "/out:$setupExe" $setupCs 2>&1
if ($LASTEXITCODE -ne 0) { $out | Write-Host; Abort "Setup.exe didn't compile" }
Remove-Item $work -Recurse -Force
Write-Ok "installer: $setupExe ($([math]::Round((Get-Item $setupExe).Length / 1MB, 0)) MB)"

if ($SkipInno) {
    Write-Host "`nDone:" -ForegroundColor Green
    Write-Host "  Installer:  $setupExe" -ForegroundColor White
    Write-Host "  Portable:   $Stage\$ExeName" -ForegroundColor White
    exit 0
}

# ─── 5. Inno Setup (the previous installer, kept until the new one is proven) ──
Write-Step "Looking for Inno Setup"
$iscc = $null
$isccCmd = Get-Command ISCC -ErrorAction SilentlyContinue
foreach ($candidate in @(
    $(if ($isccCmd) { $isccCmd.Source }),
    "C:\Program Files (x86)\Inno Setup 6\ISCC.exe",
    "C:\Program Files\Inno Setup 6\ISCC.exe",
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
)) {
    if ($candidate -and (Test-Path $candidate)) { $iscc = $candidate; break }
}
if (-not $iscc) {
    Write-Warn "Inno Setup 6 not found (winget install JRSoftware.InnoSetup), skipping the Inno installer"
    Write-Host "`nDone:" -ForegroundColor Green
    Write-Host "  Installer:  $setupExe" -ForegroundColor White
    Write-Host "  Portable:   $Stage\$ExeName" -ForegroundColor White
    exit 0
}
Write-Ok $iscc

Write-Step "Building the Inno Setup installer"
# VersionInfoVersion — только числа: 0.2.0-rc1 → 0.2.0
$numVersion = ($Version -split '[-+]')[0]
# "-Setup-inno.exe": the in-app updater picks the asset ending in "-setup.exe", which is the new installer
$innoBase = "$FilePrefix-$Version-Setup-inno"
& $iscc "/Qp" "/DAppVersion=$Version" "/DAppNumVersion=$numVersion" "/DDistDir=$Stage" "/O$OutDir" "/F$innoBase" (Join-Path $ProjectRoot "pixelmusic.iss")
if ($LASTEXITCODE -ne 0) { Abort "Inno Setup failed with exit code $LASTEXITCODE" }

$innoExe = Join-Path $OutDir "$innoBase.exe"
if (-not (Test-Path $innoExe)) { Abort "$innoExe not found after ISCC" }
Write-Host "`nDone:" -ForegroundColor Green
Write-Host "  Installer:  $setupExe" -ForegroundColor White
Write-Host "  Inno Setup: $innoExe" -ForegroundColor White
Write-Host "  Portable:   $Stage\$ExeName" -ForegroundColor White
