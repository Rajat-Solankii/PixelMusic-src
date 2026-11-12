# Script to regenerate all icons
Write-Host "Regenerating all icons..." -ForegroundColor Cyan

# Run the icon generator
cargo run --release --package icon-gen

if ($LASTEXITCODE -eq 0) {
    Write-Host "`n✓ Icons regenerated successfully!" -ForegroundColor Green

    # Verify
    Write-Host "`nVerifying..." -ForegroundColor Yellow
    pwsh -File "$PSScriptRoot\verify.ps1"
} else {
    Write-Host "`n✗ Icon generation failed!" -ForegroundColor Red
    exit 1
}
