<#
.SYNOPSIS
Build an installable, per-user Windows x64 bmux package with NSIS 3.
.EXAMPLE
.\packaging\build-windows-installer.ps1
.EXAMPLE
.\packaging\build-windows-installer.ps1 -SkipBuild -BinaryPath C:\build\bmux.exe
.NOTES
Requires Rust, the Visual Studio C++ build tools/Windows SDK, and NSIS 3.
The default MSVC release build statically links the C runtime. A prebuilt
binary must already contain or be able to find all of its runtime dependencies.
#>
[CmdletBinding()]
param(
    [ValidateSet("x86_64-pc-windows-msvc", "x86_64-pc-windows-gnu")]
    [string]$Target = "x86_64-pc-windows-msvc",
    [switch]$SkipBuild,
    [string]$BinaryPath,
    [string]$OutputDirectory,
    [string]$NsisPath
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"
$repository = Split-Path -Parent $PSScriptRoot
$isWindowsHost = $env:OS -eq "Windows_NT"

function Resolve-InputPath([string]$Path) {
    $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Path)
}

function Find-NsisCompiler {
    if ($NsisPath) {
        $resolved = Resolve-InputPath $NsisPath
        if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) {
            throw "NSIS compiler not found: $resolved"
        }
        return $resolved
    }
    foreach ($name in @("makensis.exe", "makensis")) {
        $command = Get-Command $name -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($command) { return $command.Source }
    }
    foreach ($parent in @(${env:ProgramFiles(x86)}, $env:ProgramFiles, "$env:LOCALAPPDATA\Programs")) {
        if ($parent) {
            $candidate = Join-Path $parent "NSIS\makensis.exe"
            if (Test-Path -LiteralPath $candidate -PathType Leaf) { return $candidate }
        }
    }
    throw "NSIS 3 was not found. Install it from https://nsis.sourceforge.io/Download, add makensis.exe to PATH, or pass -NsisPath."
}

function Assert-X64Executable([string]$Path) {
    $stream = [System.IO.File]::OpenRead($Path)
    $reader = New-Object System.IO.BinaryReader($stream)
    try {
        if ($stream.Length -lt 64 -or $reader.ReadUInt16() -ne 0x5A4D) {
            throw "The binary is not a Windows PE executable: $Path"
        }
        $stream.Position = 0x3C
        $peOffset = $reader.ReadUInt32()
        if ($peOffset -lt 64 -or $peOffset + 26 -gt $stream.Length) {
            throw "The binary has an invalid Windows PE header: $Path"
        }
        $stream.Position = $peOffset
        if ($reader.ReadUInt32() -ne 0x00004550 -or $reader.ReadUInt16() -ne 0x8664) {
            throw "The installer requires an x86_64 Windows executable: $Path"
        }
        $stream.Position = $peOffset + 24
        if ($reader.ReadUInt16() -ne 0x020B) {
            throw "The installer requires a 64-bit Windows executable: $Path"
        }
    } finally {
        $reader.Dispose()
        $stream.Dispose()
    }
}

if ($BinaryPath -and -not $SkipBuild) {
    throw "Use -SkipBuild with -BinaryPath to package an existing executable."
}
if (-not $isWindowsHost -and -not $SkipBuild) {
    throw "Run this build script on Windows, or use -SkipBuild -BinaryPath with a prebuilt Windows executable."
}
if (-not (Get-Command cargo -CommandType Application -ErrorAction SilentlyContinue)) {
    throw "Cargo was not found. Install Rust from https://rustup.rs/ and restart your terminal."
}
$compiler = Find-NsisCompiler
if ($BinaryPath) { $BinaryPath = Resolve-InputPath $BinaryPath }
if ($OutputDirectory) {
    $OutputDirectory = Resolve-InputPath $OutputDirectory
} else {
    $OutputDirectory = Join-Path $repository "dist"
}

Push-Location $repository
try {
    $metadataJson = & cargo metadata --no-deps --format-version 1 --locked
    if ($LASTEXITCODE -ne 0) { throw "Cargo metadata failed ($LASTEXITCODE)." }
    $metadata = $metadataJson | ConvertFrom-Json
    $package = @($metadata.packages | Where-Object { $_.name -eq "bmux" })
    if ($package.Count -ne 1) { throw "Could not identify the bmux package in Cargo metadata." }
    $version = $package[0].version
    if ($version -notmatch '^(\d+)\.(\d+)\.(\d+)(?:[-+][0-9A-Za-z.+-]+)?$') {
        throw "Unsupported package version: $version"
    }
    $parts = @([int]$Matches[1], [int]$Matches[2], [int]$Matches[3])
    if (@($parts | Where-Object { $_ -gt 65535 }).Count -gt 0) {
        throw "Windows version components must be between 0 and 65535: $version"
    }
    $numericVersion = ($parts -join ".") + ".0"

    if (-not $SkipBuild) {
        # Keep the normal MSVC installer self-contained on systems without the
        # Visual C++ redistributable. Restore the caller's environment afterwards.
        $previousRustFlags = $env:RUSTFLAGS
        $previousEncodedFlags = $env:CARGO_ENCODED_RUSTFLAGS
        $hadEncodedFlags = Test-Path Env:CARGO_ENCODED_RUSTFLAGS
        try {
            if ($Target -eq "x86_64-pc-windows-msvc") {
                if ($hadEncodedFlags) {
                    $staticFlags = "-C" + [char]31 + "target-feature=+crt-static"
                    $env:CARGO_ENCODED_RUSTFLAGS = if ($previousEncodedFlags) {
                        $previousEncodedFlags + [char]31 + $staticFlags
                    } else {
                        $staticFlags
                    }
                } else {
                    $env:RUSTFLAGS = ("$previousRustFlags -C target-feature=+crt-static").Trim()
                }
            }
            & cargo build --release --locked --target $Target
            if ($LASTEXITCODE -ne 0) { throw "Cargo release build failed ($LASTEXITCODE)." }
        } finally {
            $env:RUSTFLAGS = $previousRustFlags
            $env:CARGO_ENCODED_RUSTFLAGS = $previousEncodedFlags
        }
    }

    if (-not $BinaryPath) {
        $BinaryPath = Join-Path $metadata.target_directory "$Target/release/bmux.exe"
    }
    if (-not (Test-Path -LiteralPath $BinaryPath -PathType Leaf)) {
        throw "Missing release executable: $BinaryPath. Build it first or supply -BinaryPath with -SkipBuild."
    }
    Assert-X64Executable $BinaryPath
    New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
    $installer = Join-Path $OutputDirectory "bmux-$version-windows-x86_64-setup.exe"
    # NSIS uses slash options on Windows and dash options on Unix hosts.
    $prefix = if ($isWindowsHost) { "/" } else { "-" }
    # Compile into a temporary directory so a failed build preserves the last
    # complete installer. Ignore machine-wide/local NSIS configuration files.
    $stagingDirectory = Join-Path $OutputDirectory (".bmux-installer-" + [guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Path $stagingDirectory | Out-Null
    $stagedInstaller = Join-Path $stagingDirectory ([System.IO.Path]::GetFileName($installer))
    $arguments = @(
        "${prefix}NOCONFIG", "${prefix}V3", "${prefix}WX",
        "${prefix}DAppVersion=$version",
        "${prefix}DAppNumericVersion=$numericVersion",
        "${prefix}DSourceDir=$repository",
        "${prefix}DAppBinary=$BinaryPath",
        "${prefix}DInstallerPath=$stagedInstaller",
        (Join-Path $PSScriptRoot "bmux.nsi")
    )
    try {
        & $compiler @arguments
        if ($LASTEXITCODE -ne 0) { throw "NSIS installer build failed ($LASTEXITCODE)." }
        if (-not (Test-Path -LiteralPath $stagedInstaller -PathType Leaf)) {
            throw "NSIS did not create the expected installer: $stagedInstaller"
        }
        Move-Item -LiteralPath $stagedInstaller -Destination $installer -Force
    } finally {
        Remove-Item -LiteralPath $stagingDirectory -Recurse -Force
    }
    Write-Host "Created $installer"
} finally {
    Pop-Location
}
