<# Smoke-test installation, upgrade, and removal on a disposable Windows CI runner. #>
[CmdletBinding()]
param([string]$InstallerPath)

Set-StrictMode -Version 2.0
$ErrorActionPreference = "Stop"
if ($env:GITHUB_ACTIONS -ne "true" -or $env:OS -ne "Windows_NT") {
    throw "This test may run only on a disposable Windows GitHub Actions runner."
}
$repository = Split-Path -Parent $PSScriptRoot
if (-not $InstallerPath) {
    $installers = @(Get-ChildItem (Join-Path $repository "dist") -Filter "bmux-*-windows-x86_64-setup.exe")
    if ($installers.Count -ne 1) { throw "Expected one installer in dist; pass -InstallerPath explicitly." }
    $InstallerPath = $installers[0].FullName
}
$InstallerPath = (Resolve-Path -LiteralPath $InstallerPath).Path
if ((Split-Path -Leaf $InstallerPath) -notmatch '^bmux-(.+)-windows-x86_64-setup\.exe$') {
    throw "Unexpected installer filename: $InstallerPath"
}
$version = $Matches[1]
$registry = [Microsoft.Win32.RegistryKey]::OpenBaseKey("CurrentUser", "Registry64")
$appKey = "Software\Bmux"
$uninstallKey = "Software\Microsoft\Windows\CurrentVersion\Uninstall\Bmux"
$dataDirectory = Join-Path $env:LOCALAPPDATA "Bmux"
$defaultDirectory = Join-Path $env:LOCALAPPDATA "Programs\Bmux"
$startMenu = Join-Path ([Environment]::GetFolderPath("Programs")) "Bmux"
$desktopShortcut = Join-Path ([Environment]::GetFolderPath("DesktopDirectory")) "Bmux.lnk"
$temporaryDirectory = Join-Path ([IO.Path]::GetTempPath()) ("bmux installer smoke " + [guid]::NewGuid())
$installDirectory = Join-Path $temporaryDirectory "Bmux App"
$fixturesOwned = $false

function Assert-True([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}
function Read-Registration([string]$Key, [string]$Name) {
    $entry = $registry.OpenSubKey($Key)
    if ($null -eq $entry) { return $null }
    try { return $entry.GetValue($Name) } finally { $entry.Dispose() }
}
function Invoke-Installer([string]$Executable, [string]$Arguments) {
    # Windows -Wait includes descendants, including the uninstaller's temporary copy.
    $process = Start-Process -FilePath $Executable -ArgumentList $Arguments -PassThru -Wait
    Assert-True ($process.ExitCode -eq 0) "Installer exited with $($process.ExitCode)."
}
function Assert-Marker([string]$Path) {
    Assert-True ((Test-Path -LiteralPath $Path) -and (Get-Content -LiteralPath $Path -Raw) -eq "preserve me") "Marker was changed or removed: $Path"
}

try {
    # Refuse existing installations or app data, even if invoked on a self-hosted runner.
    foreach ($key in @($appKey, $uninstallKey)) {
        $entry = $registry.OpenSubKey($key)
        if ($null -ne $entry) { $entry.Dispose(); throw "Refusing preexisting registry key: $key" }
    }
    foreach ($path in @($dataDirectory, $defaultDirectory, $startMenu, $desktopShortcut, $temporaryDirectory)) {
        Assert-True (-not (Test-Path -LiteralPath $path)) "Refusing preexisting path: $path"
    }
    $fixturesOwned = $true
    New-Item -ItemType Directory -Path $temporaryDirectory, $dataDirectory | Out-Null
    $dataMarker = Join-Path $dataDirectory "installer-test-marker.txt"
    Set-Content -LiteralPath $dataMarker -Value "preserve me" -NoNewline
    # NSIS requires /D= last, with no quotes around its value even when it has spaces.
    Invoke-Installer $InstallerPath "/S /D=$installDirectory"
    $ownedFiles = @("bmux.exe", "Uninstall.exe", "README.md", "THIRD_PARTY_NOTICES.md", "RUN_WINDOWS.txt", "assets\bmux.ico", "assets\themes.LICENSE", "assets\themes.source.json", "docs\PORTING.md", "docs\VALIDATION.md", "Assets\Fonts\OFL.txt")
    foreach ($file in $ownedFiles) {
        Assert-True (Test-Path -LiteralPath (Join-Path $installDirectory $file)) "Missing installed file: $file"
    }
    Assert-True ((Read-Registration $uninstallKey "InstallLocation") -eq $installDirectory) "Wrong registered installation directory."
    Assert-True ((Read-Registration $uninstallKey "DisplayVersion") -eq $version) "Wrong registered version."
    foreach ($name in @("Bmux.lnk", "Uninstall Bmux.lnk")) {
        Assert-True (Test-Path -LiteralPath (Join-Path $startMenu $name)) "Missing Start Menu shortcut: $name"
    }
    Assert-Marker $dataMarker
    $unknownFile = Join-Path $installDirectory "user-owned-marker.txt"
    Set-Content -LiteralPath $unknownFile -Value "preserve me" -NoNewline

    Invoke-Installer $InstallerPath "/S"
    Assert-True ((Read-Registration $appKey "InstallLocation") -eq $installDirectory) "Upgrade changed the installation directory."
    Assert-True (-not (Test-Path -LiteralPath $defaultDirectory)) "Upgrade created an unexpected default installation."
    Assert-Marker $dataMarker
    Assert-Marker $unknownFile

    Invoke-Installer (Join-Path $installDirectory "Uninstall.exe") "/S"
    foreach ($file in $ownedFiles) {
        Assert-True (-not (Test-Path -LiteralPath (Join-Path $installDirectory $file))) "Uninstaller retained an owned file: $file"
    }
    foreach ($key in @($appKey, $uninstallKey)) {
        $entry = $registry.OpenSubKey($key)
        if ($null -ne $entry) { $entry.Dispose(); throw "Uninstaller retained registry key: $key" }
    }
    Assert-True (-not (Test-Path -LiteralPath $startMenu)) "Uninstaller retained Start Menu shortcuts."
    Assert-True (-not (Test-Path -LiteralPath $desktopShortcut)) "Uninstaller retained the desktop shortcut."
    Assert-Marker $dataMarker
    Assert-Marker $unknownFile

    # Also exercise the normal first-install location without a /D= override.
    Invoke-Installer $InstallerPath "/S"
    Assert-True ((Read-Registration $appKey "InstallLocation") -eq $defaultDirectory) "Wrong default installation directory."
    Assert-True (Test-Path -LiteralPath (Join-Path $defaultDirectory "bmux.exe")) "Missing default installation."
    Invoke-Installer (Join-Path $defaultDirectory "Uninstall.exe") "/S"
    Assert-True (-not (Test-Path -LiteralPath $defaultDirectory)) "Default installation was not removed."
    Assert-Marker $dataMarker
    Write-Host "Installer smoke test passed: install, upgrade, uninstall, registration, shortcuts, and preserved user files."
} finally {
    if ($fixturesOwned) {
        foreach ($key in @($appKey, $uninstallKey)) { $registry.DeleteSubKeyTree($key, $false) }
        foreach ($path in @($temporaryDirectory, $dataDirectory, $defaultDirectory, $startMenu, $desktopShortcut)) {
            if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Recurse -Force }
        }
    }
    $registry.Dispose()
}
