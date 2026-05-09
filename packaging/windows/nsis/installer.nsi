; gDriver Windows Installer — NSIS Template
;
; Used by Tauri v2 bundler via tauri.conf.json → bundle.windows.nsis.template.
; Preprocessed by build.ps1 before Tauri build to inject binary paths.
;
; Extends the standard Tauri installer to include:
;   - gdriver-daemon.exe (background sync daemon)
;   - gdriver_shell.dll   (Explorer shell extension)
;   - Registry entries for autostart + shell extension registration
;   - Post-install Explorer restart prompt
;
; Tauri replaces the following placeholders at build time:
;   {{product_name}}      — "gDriver"
;   {{version}}           — "0.1.0"
;   {{main_binary_name}}  — "gdriver"
;   {{binaries}}          — auto-generated File / Delete sections
;   {{resources}}         — auto-generated resource File sections
;
; Build script (build.ps1) replaces:
;   __DAEMON_BINARY__     — absolute path to gdriver-daemon.exe
;   __SHELL_DLL__         — absolute path to gdriver_shell.dll

; ── Includes ─────────────────────────────────────────────────────────────
!include "MUI2.nsh"
!include "FileFunc.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"

; ── Preprocessed paths (replaced by build.ps1) ───────────────────────────
!define DAEMON_BINARY  "__DAEMON_BINARY__"
!define SHELL_DLL      "__SHELL_DLL__"

; ── Includes ─────────────────────────────────────────────────────────────
!include "MUI2.nsh"
!include "FileFunc.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"

; ── General ──────────────────────────────────────────────────────────────
Name "{{product_name}}"
OutFile "{{product_name}}_{{version}}_x64-setup.exe"
BrandingText "{{product_name}}"

InstallDir "$PROGRAMFILES64\{{product_name}}"
InstallDirRegKey HKLM "Software\{{product_name}}" "InstallDir"

RequestExecutionLevel admin    ; Required for HKLM registry + shell extension

; ── Variables ────────────────────────────────────────────────────────────
Var StartMenuFolder
Var DaemonPID

; ── Modern UI ────────────────────────────────────────────────────────────
!define MUI_ABORTWARNING
!define MUI_ICON "{{icon_path}}"
!define MUI_UNICON "{{icon_path}}"

; Pages
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "{{license_path}}"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_STARTMENU Application $StartMenuFolder
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "SimpChinese"

; ── Installer Sections ───────────────────────────────────────────────────

Section "{{product_name}}" SecMain
    SectionIn RO

    SetOutPath "$INSTDIR"

    ; --- Main application (Tauri-generated) ---
    {{binaries}}

    ; --- Resources (icons, locales, WebView2) ---
    {{resources}}

    ; --- gdriver-daemon (sync engine) ---
    File "/oname=$INSTDIR\gdriver-daemon.exe" "${DAEMON_BINARY}"

    ; --- gdriver Shell Extension DLL ---
    File "/oname=$INSTDIR\gdriver_shell.dll" "${SHELL_DLL}"

    ; --- Shell extension registry ---
    WriteRegStr HKLM "Software\{{product_name}}" "InstallDir" "$INSTDIR"
    WriteRegStr HKLM "Software\{{product_name}}" "Version" "{{version}}"

    ; Register COM DLL
    Call RegisterShellExtension

    ; --- Uninstaller ---
    WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\{{product_name}}" \
                     "DisplayName" "{{product_name}}"
    WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\{{product_name}}" \
                     "DisplayVersion" "{{version}}"
    WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\{{product_name}}" \
                     "Publisher" "gDriver Contributors"
    WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\{{product_name}}" \
                     "DisplayIcon" "$INSTDIR\{{main_binary_name}}.exe"
    WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\{{product_name}}" \
                     "UninstallString" '"$INSTDIR\uninstall.exe"'
    WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\{{product_name}}" \
                     "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
    WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\{{product_name}}" \
                       "NoModify" 1
    WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\{{product_name}}" \
                       "NoRepair" 1
    WriteUninstaller "$INSTDIR\uninstall.exe"

    ; --- Autostart (launch daemon on user login) ---
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" \
                     "{{product_name}}" '"$INSTDIR\gdriver-daemon.exe"'

    ; --- Start menu shortcuts ---
    !insertmacro MUI_STARTMENU_WRITE_BEGIN Application
    CreateDirectory "$SMPROGRAMS\$StartMenuFolder"
    CreateShortcut "$SMPROGRAMS\$StartMenuFolder\{{product_name}}.lnk" \
                   "$INSTDIR\{{main_binary_name}}.exe"
    CreateShortcut "$SMPROGRAMS\$StartMenuFolder\Uninstall {{product_name}}.lnk" \
                   "$INSTDIR\uninstall.exe"
    !insertmacro MUI_STARTMENU_WRITE_END

    ; --- Estimate size ---
    ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
    IntFmt $0 "0x%08X" $0
    WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\{{product_name}}" \
                       "EstimatedSize" "$0"

    ; --- Kill and restart Explorer for shell extension to take effect ---
    Call RestartExplorerIfNeeded
SectionEnd


Section "Uninstall"
    ; --- Kill daemon if running ---
    nsExec::ExecToStack 'taskkill /f /im gdriver-daemon.exe'
    Pop $0

    ; --- Stop main app if running ---
    nsExec::ExecToStack 'taskkill /f /im {{main_binary_name}}.exe'
    Pop $0

    ; --- Unregister shell extension ---
    Call un.UnregisterShellExtension

    ; --- Remove files ---
    Delete "$INSTDIR\gdriver-daemon.exe"
    Delete "$INSTDIR\gdriver_shell.dll"
    Delete "$INSTDIR\uninstall.exe"

    {{binaries}}
    RMDir /r "$INSTDIR"

    ; --- Remove autostart ---
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "{{product_name}}"

    ; --- Remove registry entries ---
    DeleteRegKey HKLM "Software\{{product_name}}"
    DeleteRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\{{product_name}}"

    ; --- Remove start menu ---
    !insertmacro MUI_STARTMENU_GETFOLDER Application $StartMenuFolder
    Delete "$SMPROGRAMS\$StartMenuFolder\{{product_name}}.lnk"
    Delete "$SMPROGRAMS\$StartMenuFolder\Uninstall {{product_name}}.lnk"
    RMDir "$SMPROGRAMS\$StartMenuFolder"

    ; --- Restart Explorer to remove shell extension ---
    Call un.RestartExplorerIfNeeded
SectionEnd


; ── Shell Extension Registration ─────────────────────────────────────────

Function RegisterShellExtension
    ; Context Menu Handler (all file types)
    WriteRegStr HKCR "*\shellex\ContextMenuHandlers\gDriver" "" \
                    "{D4C2B2A1-1111-2222-3333-444455556600}"
    WriteRegStr HKCR "Directory\shellex\ContextMenuHandlers\gDriver" "" \
                    "{D4C2B2A1-1111-2222-3333-444455556600}"
    WriteRegStr HKCR "Directory\Background\shellex\ContextMenuHandlers\gDriver" "" \
                    "{D4C2B2A1-1111-2222-3333-444455556600}"

    ; Icon Overlay: Cloud
    WriteRegStr HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\ShellIconOverlayIdentifiers\ gDriverCloud" "" \
                    "{D4C2B2A1-1111-2222-3333-444455556601}"
    ; Icon Overlay: Syncing
    WriteRegStr HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\ShellIconOverlayIdentifiers\ gDriverSyncing" "" \
                    "{D4C2B2A1-1111-2222-3333-444455556602}"
    ; Icon Overlay: Synced
    WriteRegStr HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\ShellIconOverlayIdentifiers\ gDriverSynced" "" \
                    "{D4C2B2A1-1111-2222-3333-444455556603}"
    ; Icon Overlay: Error
    WriteRegStr HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\ShellIconOverlayIdentifiers\ gDriverError" "" \
                    "{D4C2B2A1-1111-2222-3333-444455556604}"
    ; Icon Overlay: Uploading
    WriteRegStr HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\ShellIconOverlayIdentifiers\ gDriverUploading" "" \
                    "{D4C2B2A1-1111-2222-3333-444455556605}"

    ; COM CLSID: Context Menu
    WriteRegStr HKCR "CLSID\{D4C2B2A1-1111-2222-3333-444455556600}" "" \
                    "gDriver Context Menu Handler"
    WriteRegStr HKCR "CLSID\{D4C2B2A1-1111-2222-3333-444455556600}\InProcServer32" "" \
                    "$INSTDIR\gdriver_shell.dll"
    WriteRegStr HKCR "CLSID\{D4C2B2A1-1111-2222-3333-444455556600}\InProcServer32" \
                    "ThreadingModel" "Apartment"

    ; COM CLSID: Cloud Overlay
    WriteRegStr HKCR "CLSID\{D4C2B2A1-1111-2222-3333-444455556601}" "" \
                    "gDriver Cloud Overlay"
    WriteRegStr HKCR "CLSID\{D4C2B2A1-1111-2222-3333-444455556601}\InProcServer32" "" \
                    "$INSTDIR\gdriver_shell.dll"
    WriteRegStr HKCR "CLSID\{D4C2B2A1-1111-2222-3333-444455556601}\InProcServer32" \
                    "ThreadingModel" "Apartment"

    ; COM CLSID: Syncing Overlay
    WriteRegStr HKCR "CLSID\{D4C2B2A1-1111-2222-3333-444455556602}" "" \
                    "gDriver Syncing Overlay"
    WriteRegStr HKCR "CLSID\{D4C2B2A1-1111-2222-3333-444455556602}\InProcServer32" "" \
                    "$INSTDIR\gdriver_shell.dll"
    WriteRegStr HKCR "CLSID\{D4C2B2A1-1111-2222-3333-444455556602}\InProcServer32" \
                    "ThreadingModel" "Apartment"

    ; COM CLSID: Synced Overlay
    WriteRegStr HKCR "CLSID\{D4C2B2A1-1111-2222-3333-444455556603}" "" \
                    "gDriver Synced Overlay"
    WriteRegStr HKCR "CLSID\{D4C2B2A1-1111-2222-3333-444455556603}\InProcServer32" "" \
                    "$INSTDIR\gdriver_shell.dll"
    WriteRegStr HKCR "CLSID\{D4C2B2A1-1111-2222-3333-444455556603}\InProcServer32" \
                    "ThreadingModel" "Apartment"

    ; COM CLSID: Error Overlay
    WriteRegStr HKCR "CLSID\{D4C2B2A1-1111-2222-3333-444455556604}" "" \
                    "gDriver Error Overlay"
    WriteRegStr HKCR "CLSID\{D4C2B2A1-1111-2222-3333-444455556604}\InProcServer32" "" \
                    "$INSTDIR\gdriver_shell.dll"
    WriteRegStr HKCR "CLSID\{D4C2B2A1-1111-2222-3333-444455556604}\InProcServer32" \
                    "ThreadingModel" "Apartment"

    ; COM CLSID: Uploading Overlay
    WriteRegStr HKCR "CLSID\{D4C2B2A1-1111-2222-3333-444455556605}" "" \
                    "gDriver Uploading Overlay"
    WriteRegStr HKCR "CLSID\{D4C2B2A1-1111-2222-3333-444455556605}\InProcServer32" "" \
                    "$INSTDIR\gdriver_shell.dll"
    WriteRegStr HKCR "CLSID\{D4C2B2A1-1111-2222-3333-444455556605}\InProcServer32" \
                    "ThreadingModel" "Apartment"

    ; Approve the shell extension for Explorer
    WriteRegStr HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Shell Extensions\Approved" \
                    "{D4C2B2A1-1111-2222-3333-444455556600}" "gDriver Context Menu Handler"
    WriteRegStr HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Shell Extensions\Approved" \
                    "{D4C2B2A1-1111-2222-3333-444455556601}" "gDriver Cloud Overlay"
    WriteRegStr HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Shell Extensions\Approved" \
                    "{D4C2B2A1-1111-2222-3333-444455556602}" "gDriver Syncing Overlay"
    WriteRegStr HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Shell Extensions\Approved" \
                    "{D4C2B2A1-1111-2222-3333-444455556603}" "gDriver Synced Overlay"
    WriteRegStr HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Shell Extensions\Approved" \
                    "{D4C2B2A1-1111-2222-3333-444455556604}" "gDriver Error Overlay"
    WriteRegStr HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Shell Extensions\Approved" \
                    "{D4C2B2A1-1111-2222-3333-444455556605}" "gDriver Uploading Overlay"

    ; Register DLL
    ExecWait '"$SYSDIR\regsvr32.exe" /s "$INSTDIR\gdriver_shell.dll"'
FunctionEnd


Function un.UnregisterShellExtension
    ; Unregister DLL
    ExecWait '"$SYSDIR\regsvr32.exe" /s /u "$INSTDIR\gdriver_shell.dll"' $0

    ; Context Menu Handler
    DeleteRegKey HKCR "*\shellex\ContextMenuHandlers\gDriver"
    DeleteRegKey HKCR "Directory\shellex\ContextMenuHandlers\gDriver"
    DeleteRegKey HKCR "Directory\Background\shellex\ContextMenuHandlers\gDriver"

    ; Icon Overlays
    DeleteRegKey HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\ShellIconOverlayIdentifiers\ gDriverCloud"
    DeleteRegKey HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\ShellIconOverlayIdentifiers\ gDriverSyncing"
    DeleteRegKey HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\ShellIconOverlayIdentifiers\ gDriverSynced"
    DeleteRegKey HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\ShellIconOverlayIdentifiers\ gDriverError"
    DeleteRegKey HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\ShellIconOverlayIdentifiers\ gDriverUploading"

    ; COM CLSIDs
    DeleteRegKey HKCR "CLSID\{D4C2B2A1-1111-2222-3333-444455556600}"
    DeleteRegKey HKCR "CLSID\{D4C2B2A1-1111-2222-3333-444455556601}"
    DeleteRegKey HKCR "CLSID\{D4C2B2A1-1111-2222-3333-444455556602}"
    DeleteRegKey HKCR "CLSID\{D4C2B2A1-1111-2222-3333-444455556603}"
    DeleteRegKey HKCR "CLSID\{D4C2B2A1-1111-2222-3333-444455556604}"
    DeleteRegKey HKCR "CLSID\{D4C2B2A1-1111-2222-3333-444455556605}"

    ; Approved shell extensions
    DeleteRegValue HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Shell Extensions\Approved" \
                        "{D4C2B2A1-1111-2222-3333-444455556600}"
    DeleteRegValue HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Shell Extensions\Approved" \
                        "{D4C2B2A1-1111-2222-3333-444455556601}"
    DeleteRegValue HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Shell Extensions\Approved" \
                        "{D4C2B2A1-1111-2222-3333-444455556602}"
    DeleteRegValue HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Shell Extensions\Approved" \
                        "{D4C2B2A1-1111-2222-3333-444455556603}"
    DeleteRegValue HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Shell Extensions\Approved" \
                        "{D4C2B2A1-1111-2222-3333-444455556604}"
    DeleteRegValue HKLM "SOFTWARE\Microsoft\Windows\CurrentVersion\Shell Extensions\Approved" \
                        "{D4C2B2A1-1111-2222-3333-444455556605}"
FunctionEnd


; ── Explorer Restart ─────────────────────────────────────────────────────

Function RestartExplorerIfNeeded
    ; Prompt user to restart Explorer (so shell extension loads immediately)
    MessageBox MB_YESNO|MB_ICONQUESTION \
        "The gDriver shell extension has been installed.$\n$\nRestart Windows Explorer now to enable file status icons and context menus?$\n$\n(Your files and running programs are not affected.)" \
        IDNO skip_restart

    ; Kill Explorer — it auto-restarts
    nsExec::ExecToStack 'taskkill /f /im explorer.exe'
    Pop $0
    Sleep 1500
    Exec '"$WINDIR\explorer.exe"'

  skip_restart:
FunctionEnd


Function un.RestartExplorerIfNeeded
    MessageBox MB_YESNO|MB_ICONQUESTION \
        "The gDriver shell extension has been removed.$\n$\nRestart Windows Explorer now to remove file status icons and context menus?" \
        IDNO un_skip_restart

    nsExec::ExecToStack 'taskkill /f /im explorer.exe'
    Pop $0
    Sleep 1500
    Exec '"$WINDIR\explorer.exe"'

  un_skip_restart:
FunctionEnd
