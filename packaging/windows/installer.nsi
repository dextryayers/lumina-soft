Unicode true
ManifestDPIAware true
ManifestSupportedOS all

!include "MUI2.nsh"
!include "FileFunc.nsh"

!ifndef VERSION
    !define VERSION "0.0.0"
!endif
!ifndef NUMERIC_VERSION
    !define NUMERIC_VERSION "0.0.0.0"
!endif
!ifndef STAGE_DIR
    !error "STAGE_DIR must point at the staged release payload"
!endif
!ifndef OUT_FILE
    !define OUT_FILE "Lumina-setup.exe"
!endif

!define APP_NAME "Lumina"
!define APP_PUBLISHER "Lumina Contributors"
!define APP_URL "https://github.com/lumina-sh/lumina"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Lumina"

Name "${APP_NAME} ${VERSION}"
Caption "${APP_NAME} ${VERSION}"
BrandingText "${APP_NAME}"
OutFile "${OUT_FILE}"
InstallDir "$LOCALAPPDATA\Lumina"
InstallDirRegKey HKCU "Software\Lumina" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma
SetShellVarContext current

VIProductVersion "${NUMERIC_VERSION}"
VIAddVersionKey /LANG=1033 "ProductName" "${APP_NAME}"
VIAddVersionKey /LANG=1033 "ProductVersion" "${VERSION}"
VIAddVersionKey /LANG=1033 "FileDescription" "${APP_NAME} setup"
VIAddVersionKey /LANG=1033 "FileVersion" "${VERSION}"
VIAddVersionKey /LANG=1033 "CompanyName" "${APP_PUBLISHER}"
VIAddVersionKey /LANG=1033 "LegalCopyright" "MIT License"

!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\lumina.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Start Lumina"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

Section "Lumina" SecMain
    SetOutPath "$INSTDIR"
    File "${STAGE_DIR}\lumina.exe"
    File "${STAGE_DIR}\lumina-send.exe"
    File "${STAGE_DIR}\LICENSE"
    File "${STAGE_DIR}\README.md"
    File "${STAGE_DIR}\CHANGELOG.md"

    WriteUninstaller "$INSTDIR\uninstall.exe"

    CreateDirectory "$SMPROGRAMS\Lumina"
    CreateShortCut "$SMPROGRAMS\Lumina\Lumina.lnk" "$INSTDIR\lumina.exe" "" "$INSTDIR\lumina.exe" 0
    CreateShortCut "$SMPROGRAMS\Lumina\Uninstall Lumina.lnk" "$INSTDIR\uninstall.exe"

    WriteRegStr HKCU "Software\Lumina" "InstallDir" "$INSTDIR"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "${APP_NAME}"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "${APP_PUBLISHER}"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "URLInfoAbout" "${APP_URL}"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
    WriteRegStr HKCU "${UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
    WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
    WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1

    ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
    IntFmt $0 "0x%08X" $0
    WriteRegDWORD HKCU "${UNINSTALL_KEY}" "EstimatedSize" "$0"
SectionEnd

Section "Uninstall"
    Delete "$SMPROGRAMS\Lumina\Lumina.lnk"
    Delete "$SMPROGRAMS\Lumina\Uninstall Lumina.lnk"
    RMDir "$SMPROGRAMS\Lumina"

    Delete "$INSTDIR\lumina.exe"
    Delete "$INSTDIR\lumina-send.exe"
    Delete "$INSTDIR\LICENSE"
    Delete "$INSTDIR\README.md"
    Delete "$INSTDIR\CHANGELOG.md"
    Delete "$INSTDIR\uninstall.exe"
    RMDir "$INSTDIR"

    DeleteRegKey HKCU "${UNINSTALL_KEY}"
    DeleteRegKey HKCU "Software\Lumina"
SectionEnd
