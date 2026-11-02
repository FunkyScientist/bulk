; Installer hooks for Bulk (Tauri NSIS bundle).

!macro NSIS_HOOK_PREINSTALL
  ; A running helper keeps bulk-helper.exe locked; stop it so it can be replaced.
  nsExec::Exec 'taskkill /F /IM bulk-helper.exe'
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; The `bulk` command: only the bin folder goes on PATH, so typing `bulk` runs the
  ; CLI rather than Bulk.exe. .NET tells open programs that PATH changed.
  nsExec::Exec `powershell -NoProfile -ExecutionPolicy Bypass -Command "$$d = '$INSTDIR\bin'; $$p = [Environment]::GetEnvironmentVariable('Path', 'Machine'); if (($$p -split ';') -notcontains $$d) { [Environment]::SetEnvironmentVariable('Path', $$p.TrimEnd(';') + ';' + $$d, 'Machine') }"`
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; Remove the scheduled tasks (Turbo scan and daily snapshots), then the helper.
  ExecWait '"$INSTDIR\bulk-helper.exe" --uninstall'
  nsExec::Exec 'taskkill /F /IM bulk-helper.exe'
  ; The "Analyze with Bulk" Explorer entries.
  DeleteRegKey HKCU "Software\Classes\Directory\shell\Bulk"
  DeleteRegKey HKCU "Software\Classes\Directory\Background\shell\Bulk"
  DeleteRegKey HKCU "Software\Classes\Drive\shell\Bulk"
  ; The `bulk` command's folder on PATH.
  nsExec::Exec `powershell -NoProfile -ExecutionPolicy Bypass -Command "$$d = '$INSTDIR\bin'; $$p = [Environment]::GetEnvironmentVariable('Path', 'Machine'); [Environment]::SetEnvironmentVariable('Path', (($$p -split ';') | Where-Object { $$_ -and $$_ -ne $$d }) -join ';', 'Machine')"`
!macroend
