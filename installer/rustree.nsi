; rustree Windows setup - per-user install, Start-menu entry, uninstaller wired
; into the Windows "Apps" settings. scripts/release.ps1 compiles it:
;
;   makensis /DVERSION=<x.y.z> /DEXE=<path to rustree.exe> /DOUTFILE=<setup.exe> installer\rustree.nsi
;
; Per-user because nothing here needs machine-wide rights: the files live under
; %LOCALAPPDATA%\Programs, the registry entries under HKCU, and the app itself
; elevates at start (rustree.manifest). scripts/update.ps1 refreshes the same
; directory in place, so setup and development copy are one install.

Unicode true
SetCompressor /SOLID lzma
ManifestDPIAware true
RequestExecutionLevel user

!ifndef VERSION
  !error "VERSION is not defined - build the setup via scripts/release.ps1"
!endif
!ifndef EXE
  !error "EXE is not defined - build the setup via scripts/release.ps1"
!endif
!ifndef OUTFILE
  !error "OUTFILE is not defined - build the setup via scripts/release.ps1"
!endif

!define PRODUCT "rustree"
!define PUBLISHER "Kevin Meister"
!define HOMEPAGE "https://github.com/kmm-codes/rustree"
!define EXE_NAME "rustree.exe"
!define WINDOW_TITLE "rustree - Disk Space Analyzer"   ; MainWindow.title in ui/main.slint
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT}"

Name "${PRODUCT}"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\${PRODUCT}"
InstallDirRegKey HKCU "Software\${PRODUCT}" "InstallDir"
BrandingText "${PRODUCT} ${VERSION}"

VIProductVersion "${VERSION}.0"
VIFileVersion "${VERSION}.0"
VIAddVersionKey "ProductName" "${PRODUCT}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "FileDescription" "${PRODUCT} ${VERSION} Setup"
VIAddVersionKey "CompanyName" "${PUBLISHER}"
VIAddVersionKey "LegalCopyright" "${PUBLISHER}"

!include "MUI2.nsh"
!include "FileFunc.nsh"

!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN
!define MUI_FINISHPAGE_RUN_FUNCTION LaunchApp

!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

; The first language is the fallback; the system UI language wins if listed.
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "German"

LangString RUNNING ${LANG_ENGLISH} "${PRODUCT} is still running. Close it, then click Retry."
LangString RUNNING ${LANG_GERMAN} "${PRODUCT} ist noch aktiv. Bitte beenden und dann auf Wiederholen klicken."

; The app runs elevated, so a plain Exec (CreateProcess) would fail with
; "elevation required"; ExecShell goes through ShellExecute and shows UAC.
Function LaunchApp
  ExecShell "open" "$INSTDIR\${EXE_NAME}"
FunctionEnd

; A running instance keeps its executable locked. This setup runs without
; elevation and cannot close the elevated app itself, so it asks the user.
!macro CHECK_RUNNING prefix
Function ${prefix}CheckRunning
  again:
  FindWindow $0 "" "${WINDOW_TITLE}"
  StrCmp $0 0 done
  MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "$(RUNNING)" IDRETRY again
  Abort
  done:
FunctionEnd
!macroend
!insertmacro CHECK_RUNNING ""
!insertmacro CHECK_RUNNING "un."

Function .onInit
  Call CheckRunning
FunctionEnd

Function un.onInit
  Call un.CheckRunning
FunctionEnd

Section "Install"
  SetOutPath "$INSTDIR"
  File "/oname=${EXE_NAME}" "${EXE}"
  WriteUninstaller "$INSTDIR\uninstall.exe"

  CreateShortcut "$SMPROGRAMS\${PRODUCT}.lnk" "$INSTDIR\${EXE_NAME}"

  WriteRegStr HKCU "Software\${PRODUCT}" "InstallDir" "$INSTDIR"

  ; The entry Windows shows under Settings > Apps > Installed apps.
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "${PRODUCT}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\${EXE_NAME}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "${PUBLISHER}"
  WriteRegStr HKCU "${UNINSTALL_KEY}" "URLInfoAbout" "${HOMEPAGE}"
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
  Delete "$INSTDIR\${EXE_NAME}"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"

  Delete "$SMPROGRAMS\${PRODUCT}.lnk"

  DeleteRegKey HKCU "${UNINSTALL_KEY}"
  DeleteRegKey HKCU "Software\${PRODUCT}"
SectionEnd
