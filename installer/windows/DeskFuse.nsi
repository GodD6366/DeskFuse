Unicode true
!define MUI_ICON "${__FILEDIR__}/../../apps/deskfuse-windows/Assets/DeskFuse.ico"
!define MUI_UNICON "${__FILEDIR__}/../../apps/deskfuse-windows/Assets/DeskFuse.ico"
!include "MUI2.nsh"
!include "x64.nsh"
!include "LogicLib.nsh"
!ifndef PAYLOAD_DIR
!error "Run scripts/package-windows.ps1; PAYLOAD_DIR is required."
!endif
Name "DeskFuse"
OutFile "${OUTPUT_FILE}"
InstallDir "$PROGRAMFILES64\DeskFuse"
RequestExecutionLevel admin
SetCompressor /SOLID lzma
ShowInstDetails show
ShowUninstDetails show
VIProductVersion "${VERSION}.${BUILD_NUMBER}"
VIAddVersionKey "ProductName" "DeskFuse"
VIAddVersionKey "FileDescription" "DeskFuse offline desktop installer"
VIAddVersionKey "FileVersion" "${VERSION}.${BUILD_NUMBER}"
!insertmacro MUI_PAGE_WELCOME
; No directory page: a LocalSystem service must not run from a user-selected path.
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_ICONSTOP "DeskFuse requires 64-bit Windows."
    Abort
  ${EndIf}
  SetRegView 64
  StrCpy $INSTDIR "$PROGRAMFILES64\DeskFuse"
FunctionEnd
!macro ControlService Action
  InitPluginsDir
  SetOutPath "$PLUGINSDIR"
  File /oname=service-control.ps1 "${__FILEDIR__}\service-control.ps1"
  ; Never inherit the plugin directory: its native System.dll shadows .NET's
  ; System.dll during Windows PowerShell Add-Type compilation.
  SetOutPath "$SYSDIR"
  ${DisableX64FSRedirection}
  nsExec::ExecToLog '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\service-control.ps1" -Action ${Action}'
  Pop $0
  ${EnableX64FSRedirection}
  ${If} $0 != 0
    MessageBox MB_ICONSTOP "DeskFuse service ${Action} failed. See installation details. Existing device identity and user files were not removed."
    Abort
  ${EndIf}
!macroend
Section "DeskFuse desktop and service"
  ; Ignore even command-line /D overrides before writing or registering a service.
  StrCpy $INSTDIR "$PROGRAMFILES64\DeskFuse"
  !insertmacro ControlService Prepare
  SetOutPath "$INSTDIR"
  ClearErrors
  File /r "${PAYLOAD_DIR}\*"
  IfErrors 0 +3
    MessageBox MB_ICONSTOP "Could not copy the complete payload. Close DeskFuse and retry installation."
    Abort
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  !insertmacro ControlService Register
  SetOutPath "$INSTDIR"
  SetShellVarContext all
  CreateDirectory "$SMPROGRAMS\DeskFuse"
  CreateShortcut "$SMPROGRAMS\DeskFuse\DeskFuse.lnk" "$INSTDIR\DeskFuse.Windows.exe"
  CreateShortcut "$DESKTOP\DeskFuse.lnk" "$INSTDIR\DeskFuse.Windows.exe"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\DeskFuse" "DisplayName" "DeskFuse"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\DeskFuse" "DisplayVersion" "${VERSION} (${BUILD_NUMBER})"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\DeskFuse" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\DeskFuse" "UninstallString" '$\"$INSTDIR\Uninstall.exe$\"'
  WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\DeskFuse" "NoModify" 1
  WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\DeskFuse" "NoRepair" 1
  ; Never launch the GUI from this elevated process or register an elevated GUI task.
SectionEnd
Section "Uninstall"
  SetRegView 64
  StrCpy $INSTDIR "$PROGRAMFILES64\DeskFuse"
  !insertmacro ControlService Remove
  ; Generated exact payload file list; no recursive deletion and no data directories.
  !include "${DELETE_INCLUDE}"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  SetShellVarContext all
  Delete "$SMPROGRAMS\DeskFuse\DeskFuse.lnk"
  Delete "$DESKTOP\DeskFuse.lnk"
  RMDir "$SMPROGRAMS\DeskFuse"
  DeleteRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\DeskFuse"
SectionEnd
