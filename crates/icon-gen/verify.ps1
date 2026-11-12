# Verify all icon files were generated
$ErrorActionPreference = "Stop"

$colorways = @(
    "blue-purple",
    "baby-pink-purple",
    "graphite-gray",
    "lime-green",
    "lime-lemon",
    "orange-yellow"
)

$sizes = @(16, 24, 32, 48, 64, 128, 256)
$iconsDir = "D:\Desktop\PixelMusicDesktop\src\materialfy\assets\icons"

Write-Host "Verifying icon generation..." -ForegroundColor Cyan
$missing = @()

foreach ($colorway in $colorways) {
    # Check ICO
    $icoPath = "$iconsDir\$colorway.ico"
    if (-not (Test-Path $icoPath)) {
        $missing += $icoPath
    }

    # Check PNGs
    foreach ($size in $sizes) {
        $pngPath = "$iconsDir\$colorway-$size.png"
        if (-not (Test-Path $pngPath)) {
            $missing += $pngPath
        }
    }
}

if ($missing.Count -eq 0) {
    Write-Host "✓ All 48 icon files generated successfully!" -ForegroundColor Green
    Write-Host ""
    Write-Host "Generated:" -ForegroundColor Yellow
    Write-Host "  - 6 ICO files (one per colorway)" -ForegroundColor White
    Write-Host "  - 42 PNG files (7 sizes × 6 colorways)" -ForegroundColor White
    Write-Host "  - Total: 48 files" -ForegroundColor White
} else {
    Write-Host "✗ Missing $($missing.Count) files:" -ForegroundColor Red
    $missing | ForEach-Object { Write-Host "  $_" -ForegroundColor Red }
    exit 1
}
