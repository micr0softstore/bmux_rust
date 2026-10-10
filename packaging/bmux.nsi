; Built by build-windows-installer.ps1. Keep a stable app key for upgrades.
Unicode true
RequestExecutionLevel user
ManifestDPIAware true
ManifestSupportedOS all
SetCompressor /SOLID lzma
SetDatablockOptimize on

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"
!include "WinVer.nsh"
!include "FileFunc.nsh"

!ifndef AppVersion
  !error "AppVersion is required"
!endif
!ifndef AppNumericVersion
  !error "AppNumericVersion is required"
!endif
!ifndef SourceDir
  !error "SourceDir is required"
!endif
!ifndef AppBinary
  !error "AppBinary is required"
!endif
!ifndef InstallerPath
  !error "InstallerPath is required"
!endif

!define AppKey "Software\Bmux"
!define UninstallKey "Software\Microsoft\Windows\CurrentVersion\Uninstall\Bmux"

Name "[bmux]"
OutFile "${InstallerPath}"
; Leave empty to distinguish /D= overrides from the previous/default location.
InstallDir ""
ShowInstDetails show
ShowUninstDetails show
BrandingText "[bmux]"
Icon "${SourceDir}/assets/bmux.ico"
UninstallIcon "${SourceDir}/assets/bmux.ico"
VIProductVersion "${AppNumericVersion}"
VIAddVersionKey /LANG=1033 "ProductName" "[bmux]"
VIAddVersionKey /LANG=1033 "FileDescription" "Bmux installer"
VIAddVersionKey /LANG=1033 "ProductVersion" "${AppVersion}"
VIAddVersionKey /LANG=1033 "FileVersion" "${AppVersion}"
VIAddVersionKey /LANG=1033 "LegalCopyright" "See THIRD_PARTY_NOTICES.md"

!define MUI_ICON "${SourceDir}/assets/bmux.ico"
!define MUI_UNICON "${SourceDir}/assets/bmux.ico"
!define MUI_ABORTWARNING
!define MUI_WELCOMEPAGE_TEXT "Install [bmux] for your Windows account.$\r$\n$\r$\nClose bmux before installing an update. Existing workspaces, settings, and transcripts will be kept.$\r$\n$\r$\nRequires 64-bit Windows 10 version 1809 or newer."
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_COMPONENTS
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!define MUI_UNCONFIRMPAGE_TEXT_TOP "Remove [bmux] from your Windows account? Close bmux before continuing.$\r$\n$\r$\nYour settings, workspaces, and transcripts will be kept."
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_UNPAGE_FINISH
!insertmacro MUI_LANGUAGE "English"

Function .onInit
  ${IfNot} ${IsNativeAMD64}
    MessageBox MB_ICONSTOP|MB_OK "This installer requires an x86_64 (Intel or AMD) Windows computer." /SD IDOK
    SetErrorLevel 1633
    Quit
  ${EndIf}
  ${IfNot} ${AtLeastBuild} 17763
    MessageBox MB_ICONSTOP|MB_OK "Bmux requires Windows 10 version 1809 or newer." /SD IDOK
    SetErrorLevel 1633
    Quit
  ${EndIf}
  SetShellVarContext current
  SetRegView 64
  ReadRegStr $0 HKCU "${AppKey}" "InstallLocation"
  ${If} $0 != ""
    ; Upgrades keep the registered location, including when /D= is supplied.
    StrCpy $INSTDIR $0
  ${ElseIf} $INSTDIR == ""
    StrCpy $INSTDIR "$LOCALAPPDATA\Programs\Bmux"
  ${EndIf}
FunctionEnd

Section "Bmux (required)" AppSection
  SectionIn RO
  SetShellVarContext current
  SetRegView 64
  SetOutPath "$INSTDIR"
  SetOverwrite on
  ClearErrors
  File /oname=bmux.exe "${AppBinary}"
  ${If} ${Errors}
    MessageBox MB_ICONSTOP|MB_OK "Could not replace bmux.exe. Close bmux and run the installer again." /SD IDOK
    Abort
  ${EndIf}
  File "${SourceDir}/README.md"
  File "${SourceDir}/THIRD_PARTY_NOTICES.md"
  File /oname=RUN_WINDOWS.txt "${SourceDir}/packaging/WINDOWS.txt"
  SetOutPath "$INSTDIR\assets"
  File "${SourceDir}/assets/bmux.ico"
  File "${SourceDir}/assets/themes.LICENSE"
  File "${SourceDir}/assets/themes.source.json"
  SetOutPath "$INSTDIR\Assets\Fonts"
  File "${SourceDir}/Assets/Fonts/OFL.txt"
  SetOutPath "$INSTDIR\docs"
  File "${SourceDir}/docs/PORTING.md"
  File "${SourceDir}/docs/VALIDATION.md"
  SetOutPath "$INSTDIR"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  ${If} ${Errors}
    MessageBox MB_ICONSTOP|MB_OK "Some installation files could not be written. Check available disk space and rerun the installer." /SD IDOK
    Abort
  ${EndIf}
  CreateDirectory "$SMPROGRAMS\Bmux"
  CreateShortcut "$SMPROGRAMS\Bmux\Bmux.lnk" "$INSTDIR\bmux.exe" "" "$INSTDIR\bmux.exe" 0
  CreateShortcut "$SMPROGRAMS\Bmux\Uninstall Bmux.lnk" "$INSTDIR\Uninstall.exe"
  WriteRegStr HKCU "${AppKey}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UninstallKey}" "DisplayName" "[bmux]"
  WriteRegStr HKCU "${UninstallKey}" "DisplayVersion" "${AppVersion}"
  WriteRegStr HKCU "${UninstallKey}" "DisplayIcon" "$INSTDIR\bmux.exe,0"
  WriteRegStr HKCU "${UninstallKey}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UninstallKey}" "URLInfoAbout" "https://github.com/imbencetsx/bmux"
  WriteRegStr HKCU "${UninstallKey}" "UninstallString" '$\"$INSTDIR\Uninstall.exe$\"'
  WriteRegStr HKCU "${UninstallKey}" "QuietUninstallString" '$\"$INSTDIR\Uninstall.exe$\" /S'
  WriteRegDWORD HKCU "${UninstallKey}" "NoModify" 1
  WriteRegDWORD HKCU "${UninstallKey}" "NoRepair" 1
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  WriteRegDWORD HKCU "${UninstallKey}" "EstimatedSize" $0
SectionEnd

Section /o "Desktop shortcut" DesktopSection
  CreateShortcut "$DESKTOP\Bmux.lnk" "$INSTDIR\bmux.exe" "" "$INSTDIR\bmux.exe" 0
SectionEnd

!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
  !insertmacro MUI_DESCRIPTION_TEXT ${AppSection} "The bmux application, notices, documentation, and Start Menu shortcuts."
  !insertmacro MUI_DESCRIPTION_TEXT ${DesktopSection} "Also place a bmux shortcut on your desktop."
!insertmacro MUI_FUNCTION_DESCRIPTION_END

Function un.onInit
  SetShellVarContext current
  SetRegView 64
FunctionEnd

Section "Uninstall"
  SetShellVarContext current
  SetRegView 64
  ClearErrors
  Delete "$INSTDIR\bmux.exe"
  ${If} ${Errors}
    MessageBox MB_ICONSTOP|MB_OK "Could not remove bmux.exe. Close bmux and run the uninstaller again." /SD IDOK
    Abort
  ${EndIf}
  ; Delete only files owned by the installer. Never recursively remove $INSTDIR
  ; or touch %LOCALAPPDATA%\Bmux / a custom BMUX_DATA_DIR.
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\THIRD_PARTY_NOTICES.md"
  Delete "$INSTDIR\RUN_WINDOWS.txt"
  Delete "$INSTDIR\assets\bmux.ico"
  Delete "$INSTDIR\assets\themes.LICENSE"
  Delete "$INSTDIR\assets\themes.source.json"
  Delete "$INSTDIR\Assets\Fonts\OFL.txt"
  Delete "$INSTDIR\docs\PORTING.md"
  Delete "$INSTDIR\docs\VALIDATION.md"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR\Assets\Fonts"
  RMDir "$INSTDIR\assets"
  RMDir "$INSTDIR\docs"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\Bmux\Bmux.lnk"
  Delete "$SMPROGRAMS\Bmux\Uninstall Bmux.lnk"
  RMDir "$SMPROGRAMS\Bmux"
  Delete "$DESKTOP\Bmux.lnk"
  DeleteRegKey HKCU "${UninstallKey}"
  DeleteRegValue HKCU "${AppKey}" "InstallLocation"
  DeleteRegKey /ifempty HKCU "${AppKey}"
SectionEnd
