; NSIS hooks for the Tauri-generated installer.
;
; Install:   registers the native-messaging host for Chrome, Edge, Brave,
;            Chromium, Vivaldi and Firefox (per-user HKCU keys + manifests).
; Uninstall: removes those registrations. User downloads, settings and the
;            database in %APPDATA%\com.pixidl.app are preserved
;            unless the user ticks "Delete the application data".

!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "Registering browser integration..."
  nsExec::ExecToLog '"$INSTDIR\pixidl-native-host.exe" --register'
  Pop $0
  DetailPrint "Browser integration registration exit code: $0"
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DetailPrint "Removing browser integration..."
  nsExec::ExecToLog '"$INSTDIR\pixidl-native-host.exe" --unregister'
  Pop $0
  ; Stop a running instance so its files can be removed.
  nsExec::Exec 'taskkill /IM pixidl.exe /T'
  Pop $0
!macroend
