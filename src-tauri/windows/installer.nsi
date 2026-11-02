Unicode true
!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"
!include "FileFunc.nsh"

; Application metadata
!define PRODUCT_NAME "ThinkPro Vantage"
!define PRODUCT_VERSION "0.2.0"
!define PRODUCT_PUBLISHER "Viet Anh Nguyen"
!define PRODUCT_WEB_SITE "https://github.com/TysonTranThai/ThinkUtils-Maintained-Upgraded"
!define PRODUCT_DIR_REGKEY "Software\Microsoft\Windows\CurrentVersion\App Paths\thinkpro-vantage.exe"
!define PRODUCT_UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_NAME}"
!define PRODUCT_UNINST_ROOT_KEY "HKLM"

Name "${PRODUCT_NAME} ${PRODUCT_VERSION}"
OutFile "..\target\x86_64-pc-windows-gnu\release\thinkpro-vantage_0.2.0_x64-setup.exe"
InstallDir "$PROGRAMFILES64\ThinkPro Vantage"
InstallDirRegKey HKLM "${PRODUCT_DIR_REGKEY}" ""
RequestExecutionLevel admin

; Modern UI settings
!define MUI_ABORTWARNING
!define MUI_ICON "..\icons\icon.ico"
!define MUI_UNICON "..\icons\icon.ico"

; Welcome page
!insertmacro MUI_PAGE_WELCOME
; Directory page
!insertmacro MUI_PAGE_DIRECTORY
; Instfiles page
!insertmacro MUI_PAGE_INSTFILES
; Finish page
!define MUI_FINISHPAGE_RUN "$INSTDIR\thinkpro-vantage.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Launch ThinkPro Vantage"
!insertmacro MUI_PAGE_FINISH

; Uninstaller pages
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

; Languages
!insertmacro MUI_LANGUAGE "English"

; Architecture check: ensure 64-bit Windows
Function .onInit
  ${IfNot} ${IsNativeAMD64}
  ${AndIfNot} ${IsNativeARM64}
  ${AndIfNot} ${IsWow64}
    MessageBox MB_OK|MB_ICONSTOP "This version of ${PRODUCT_NAME} requires a 64-bit version of Windows."
    Abort
  ${EndIf}
FunctionEnd

; WebView2 detection helper function
Function CheckWebView2
  ClearErrors
  ; Check 64-bit / 32-bit registry paths for Evergreen WebView2 Runtime
  ReadRegStr $0 HKLM "SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-9E47-4467-A238-164F02E26E46}" "pv"
  ${If} $0 != ""
    Return
  ${EndIf}

  ReadRegStr $0 HKLM "SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-9E47-4467-A238-164F02E26E46}" "pv"
  ${If} $0 != ""
    Return
  ${EndIf}

  ReadRegStr $0 HKCU "SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-9E47-4467-A238-164F02E26E46}" "pv"
  ${If} $0 != ""
    Return
  ${EndIf}

  ; If WebView2 is not detected:
  MessageBox MB_YESNO|MB_ICONQUESTION "${PRODUCT_NAME} requires the Microsoft Edge WebView2 Runtime, which does not appear to be installed.$\n$\nWould you like to download and install it now from Microsoft?" IDYES download_wv2 IDNO done_wv2

download_wv2:
  DetailPrint "Downloading Microsoft Edge WebView2 Evergreen Bootstrapper..."
  InitPluginsDir
  NSISdl::download "https://go.microsoft.com/fwlink/p/?LinkId=2124703" "$PLUGINSDIR\MicrosoftEdgeWebview2Setup.exe"
  Pop $R0
  ${If} $R0 == "success"
    DetailPrint "Installing Microsoft Edge WebView2 Runtime..."
    ExecWait '"$PLUGINSDIR\MicrosoftEdgeWebview2Setup.exe" /silent /install' $R1
  ${Else}
    DetailPrint "Download failed. Please install Microsoft Edge WebView2 Runtime manually from: https://developer.microsoft.com/en-us/microsoft-edge/webview2/"
    MessageBox MB_OK|MB_ICONEXCLAMATION "Could not download the WebView2 Runtime automatically.$\n$\nPlease download and install it manually from Microsoft's website."
  ${EndIf}

done_wv2:
FunctionEnd

Section "MainSection" SEC01
  SetOutPath "$INSTDIR"
  SetOverwrite on

  ; Check and install WebView2 if needed
  Call CheckWebView2

  ; Install core application and dependencies
  File "..\target\x86_64-pc-windows-gnu\release\thinkpro-vantage.exe"
  File "..\target\x86_64-pc-windows-gnu\release\WebView2Loader.dll"
  File "..\icons\icon.ico"

  ; Create Start Menu Shortcuts
  CreateDirectory "$SMPROGRAMS\${PRODUCT_NAME}"
  CreateShortcut "$SMPROGRAMS\${PRODUCT_NAME}\${PRODUCT_NAME}.lnk" "$INSTDIR\thinkpro-vantage.exe" "" "$INSTDIR\icon.ico" 0
  CreateShortcut "$SMPROGRAMS\${PRODUCT_NAME}\Uninstall ${PRODUCT_NAME}.lnk" "$INSTDIR\uninstall.exe" "" "$INSTDIR\icon.ico" 0

  ; Create Desktop Shortcut
  CreateShortcut "$DESKTOP\${PRODUCT_NAME}.lnk" "$INSTDIR\thinkpro-vantage.exe" "" "$INSTDIR\icon.ico" 0

  ; Registry entries
  WriteRegStr HKLM "${PRODUCT_DIR_REGKEY}" "" "$INSTDIR\thinkpro-vantage.exe"
  WriteRegStr ${PRODUCT_UNINST_ROOT_KEY} "${PRODUCT_UNINST_KEY}" "DisplayName" "${PRODUCT_NAME}"
  WriteRegStr ${PRODUCT_UNINST_ROOT_KEY} "${PRODUCT_UNINST_KEY}" "UninstallString" "$INSTDIR\uninstall.exe"
  WriteRegStr ${PRODUCT_UNINST_ROOT_KEY} "${PRODUCT_UNINST_KEY}" "DisplayIcon" "$INSTDIR\icon.ico"
  WriteRegStr ${PRODUCT_UNINST_ROOT_KEY} "${PRODUCT_UNINST_KEY}" "DisplayVersion" "${PRODUCT_VERSION}"
  WriteRegStr ${PRODUCT_UNINST_ROOT_KEY} "${PRODUCT_UNINST_KEY}" "URLInfoAbout" "${PRODUCT_WEB_SITE}"
  WriteRegStr ${PRODUCT_UNINST_ROOT_KEY} "${PRODUCT_UNINST_KEY}" "Publisher" "${PRODUCT_PUBLISHER}"
  WriteRegStr ${PRODUCT_UNINST_ROOT_KEY} "${PRODUCT_UNINST_KEY}" "InstallLocation" "$INSTDIR"

  ; Calculate estimated size
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD ${PRODUCT_UNINST_ROOT_KEY} "${PRODUCT_UNINST_KEY}" "EstimatedSize" "$0"

  ; Create uninstaller
  WriteUninstaller "$INSTDIR\uninstall.exe"
SectionEnd

Section "Uninstall"
  ; Kill running application if active
  DetailPrint "Closing running instances of ${PRODUCT_NAME}..."
  nsExec::Exec 'taskkill /F /IM thinkpro-vantage.exe'

  ; Remove shortcuts
  Delete "$DESKTOP\${PRODUCT_NAME}.lnk"
  Delete "$SMPROGRAMS\${PRODUCT_NAME}\${PRODUCT_NAME}.lnk"
  Delete "$SMPROGRAMS\${PRODUCT_NAME}\Uninstall ${PRODUCT_NAME}.lnk"
  RMDir "$SMPROGRAMS\${PRODUCT_NAME}"

  ; Remove files
  Delete "$INSTDIR\thinkpro-vantage.exe"
  Delete "$INSTDIR\WebView2Loader.dll"
  Delete "$INSTDIR\icon.ico"
  Delete "$INSTDIR\uninstall.exe"

  ; Remove install directory if empty
  RMDir "$INSTDIR"

  ; Remove registry keys
  DeleteRegKey ${PRODUCT_UNINST_ROOT_KEY} "${PRODUCT_UNINST_KEY}"
  DeleteRegKey HKLM "${PRODUCT_DIR_REGKEY}"

  SetAutoClose true
SectionEnd
