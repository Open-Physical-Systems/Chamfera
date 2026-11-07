; Chamfera per-user installer for Windows (NSIS). Built by build-windows.ps1, which passes
; VERSION, SRCDIR (the staged files), ICON and OUTFILE with /D.
; Installs into %LOCALAPPDATA%\Programs\Chamfera without administrator rights, adds a Start
; menu shortcut and an entry in "Installed apps". Silent: setup.exe /S [/D=C:\path].

Unicode true
!include "MUI2.nsh"

!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\Chamfera"

Name "Chamfera ${VERSION}"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\Chamfera"
InstallDirRegKey HKCU "Software\Chamfera" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma

!define MUI_ICON "${ICON}"
!define MUI_UNICON "${ICON}"
!define MUI_FINISHPAGE_RUN "$INSTDIR\chamfera.exe"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Section "Chamfera"
  SetOutPath "$INSTDIR"
  File "${SRCDIR}\chamfera.exe"
  File "${SRCDIR}\chamfera-cli.exe"
  File "${SRCDIR}\LICENSE-MIT"
  File "${SRCDIR}\LICENSE-APACHE"
  File "${SRCDIR}\ATTRIBUTION.md"
  File "${SRCDIR}\THIRD-PARTY-LICENSES.txt"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  CreateShortcut "$SMPROGRAMS\Chamfera.lnk" "$INSTDIR\chamfera.exe"
  WriteRegStr HKCU "Software\Chamfera" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "Chamfera"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "Chamfera contributors"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayIcon" "$INSTDIR\chamfera.exe"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1
SectionEnd

Section "Uninstall"
  Delete "$INSTDIR\chamfera.exe"
  Delete "$INSTDIR\chamfera-cli.exe"
  Delete "$INSTDIR\LICENSE-MIT"
  Delete "$INSTDIR\LICENSE-APACHE"
  Delete "$INSTDIR\ATTRIBUTION.md"
  Delete "$INSTDIR\THIRD-PARTY-LICENSES.txt"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\Chamfera.lnk"
  DeleteRegKey HKCU "${UNINST_KEY}"
  DeleteRegKey HKCU "Software\Chamfera"
SectionEnd
