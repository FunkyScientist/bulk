; Installer hooks for Bulk (Tauri NSIS bundle).

!macro NSIS_HOOK_PREINSTALL
  ; A running helper keeps bulk-helper.exe locked; stop it so it can be replaced.
  nsExec::Exec 'taskkill /F /IM bulk-helper.exe'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; Remove the scheduled tasks (Turbo scan and daily snapshots), then the helper.
  ExecWait '"$INSTDIR\bulk-helper.exe" --uninstall'
  nsExec::Exec 'taskkill /F /IM bulk-helper.exe'
  ; The "Analyze with Bulk" Explorer entries.
  DeleteRegKey HKCU "Software\Classes\Directory\shell\Bulk"
  DeleteRegKey HKCU "Software\Classes\Directory\Background\shell\Bulk"
  DeleteRegKey HKCU "Software\Classes\Drive\shell\Bulk"
!macroend
