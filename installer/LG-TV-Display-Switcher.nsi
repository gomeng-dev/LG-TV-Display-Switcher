Unicode true
RequestExecutionLevel user

!include "MUI2.nsh"
!include "LogicLib.nsh"

!define APP_NAME "LG-TV-Display-Switcher"
!define APP_EXE "LG-TV-Display-Switcher.exe"
!define CONFIG_FILE "LG-TV-Display-Switcher.cfg"
!define LEGACY_CONFIG_FILE "TVGuardTray.cfg"
!define LOG_FILE "LG-TV-Display-Switcher.log"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_NAME}"

!ifndef APP_BINARY
!define APP_BINARY "target\release\${APP_EXE}"
!endif

!ifndef DEFAULT_CONFIG
!define DEFAULT_CONFIG "installer\DefaultConfig.cfg"
!endif

!ifndef ENSURE_DISPLAY_CONFIG
!define ENSURE_DISPLAY_CONFIG "installer\EnsureDisplayConfig.ps1"
!endif

!ifndef ICON_FILE
!define ICON_FILE "assets\app-icon.ico"
!endif

!ifndef INSTALLER_OUTPUT
!define INSTALLER_OUTPUT "dist\${APP_NAME}-Setup.exe"
!endif

Name "${APP_NAME}"
OutFile "${INSTALLER_OUTPUT}"
Icon "${ICON_FILE}"
UninstallIcon "${ICON_FILE}"
InstallDir "$LOCALAPPDATA\${APP_NAME}"
InstallDirRegKey HKCU "${UNINSTALL_KEY}" "InstallLocation"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

Section "Install"
    SetShellVarContext current
    SetOutPath "$INSTDIR"

    DetailPrint "Stopping running copies..."
    nsExec::ExecToLog 'taskkill /IM "${APP_EXE}" /F'
    Pop $0
    nsExec::ExecToLog 'taskkill /IM "TVGuardTray.exe" /F'
    Pop $0

    DetailPrint "Installing ${APP_NAME}..."
    File /oname=$INSTDIR\${APP_EXE} "${APP_BINARY}"

    IfFileExists "$INSTDIR\${CONFIG_FILE}" config_done 0
    IfFileExists "$INSTDIR\${LEGACY_CONFIG_FILE}" 0 write_default_config
        CopyFiles /SILENT "$INSTDIR\${LEGACY_CONFIG_FILE}" "$INSTDIR\${CONFIG_FILE}"
        Goto config_done

write_default_config:
    File /oname=$INSTDIR\${CONFIG_FILE} "${DEFAULT_CONFIG}"

config_done:
    Call EnsureDisplayConfig

    DetailPrint "Creating shortcuts..."
    CreateShortcut "$SMPROGRAMS\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}" "" "$INSTDIR\${APP_EXE}"
    CreateShortcut "$SMSTARTUP\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}" "" "$INSTDIR\${APP_EXE}"

    WriteUninstaller "$INSTDIR\Uninstall.exe"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "${APP_NAME}"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "0.1.3"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "gomeng-dev"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\${APP_EXE}"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
    WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
    WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1

    DetailPrint "Starting ${APP_NAME}..."
    ExecShell "" "$INSTDIR\${APP_EXE}"
SectionEnd

Function EnsureDisplayConfig
    DetailPrint "Checking DisplayConfig dependency..."
    InitPluginsDir
    File /oname=$PLUGINSDIR\EnsureDisplayConfig.ps1 "${ENSURE_DISPLAY_CONFIG}"
    nsExec::ExecToLog 'powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$PLUGINSDIR\EnsureDisplayConfig.ps1"'
    Pop $0
    ${If} $0 != 0
        MessageBox MB_ICONEXCLAMATION|MB_OK "DisplayConfig could not be installed automatically. ${APP_NAME} can still run, but display switching may fail until DisplayConfig is installed from PowerShell Gallery."
    ${EndIf}
FunctionEnd

Section "Uninstall"
    SetShellVarContext current

    DetailPrint "Stopping ${APP_NAME}..."
    nsExec::ExecToLog 'taskkill /IM "${APP_EXE}" /F'
    Pop $0

    Delete "$SMSTARTUP\${APP_NAME}.lnk"
    Delete "$SMPROGRAMS\${APP_NAME}.lnk"
    Delete "$INSTDIR\${APP_EXE}"
    Delete "$INSTDIR\Uninstall.exe"

    DeleteRegKey HKCU "${UNINSTALL_KEY}"

    MessageBox MB_YESNO|MB_ICONQUESTION "Remove ${APP_NAME} settings and logs too?" IDNO keep_data
        Delete "$INSTDIR\${CONFIG_FILE}"
        Delete "$INSTDIR\${LEGACY_CONFIG_FILE}"
        Delete "$INSTDIR\${LOG_FILE}"
        Delete "$INSTDIR\TVGuardTray.log"

keep_data:
    RMDir "$INSTDIR"
SectionEnd
