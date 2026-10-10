[CmdletBinding()]
param(
    [string]$Target = "x86_64-pc-windows-msvc",
    [switch]$SkipBuild
)
$ErrorActionPreference = "Stop"
Push-Location (Split-Path -Parent $PSScriptRoot)
try {
    if (-not $SkipBuild) {
        cargo build --release --locked --target $Target
        if ($LASTEXITCODE -ne 0) { throw "Cargo build failed ($LASTEXITCODE)." }
    }
    $metadata = cargo metadata --no-deps --format-version 1 --locked | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { throw "Cargo metadata failed ($LASTEXITCODE)." }
    $version = ($metadata.packages | Where-Object { $_.name -eq "bmux" }).version
    $name = "bmux-$version-$Target"
    $stage = Join-Path "dist" $name
    $binary = Join-Path "target/$Target/release" "bmux.exe"
    if (-not (Test-Path $binary)) { throw "Missing $binary." }
    New-Item -ItemType Directory -Force -Path "$stage/assets", "$stage/Assets/Fonts", "$stage/docs" | Out-Null
    Copy-Item $binary "$stage/bmux.exe" -Force
    Copy-Item "assets/bmux.ico", "assets/themes.LICENSE", "assets/themes.source.json" "$stage/assets/" -Force
    Copy-Item "Assets/Fonts/OFL.txt" "$stage/Assets/Fonts/" -Force
    Copy-Item "README.md", "THIRD_PARTY_NOTICES.md" "$stage/" -Force
    Copy-Item "docs/PORTING.md", "docs/VALIDATION.md" "$stage/docs/" -Force
    Copy-Item "packaging/WINDOWS.txt" "$stage/RUN_WINDOWS.txt" -Force
    Compress-Archive -Path "$stage/*" -DestinationPath "dist/$name.zip" -Force
    Write-Host "Created dist/$name.zip"
} finally {
    Pop-Location
}
