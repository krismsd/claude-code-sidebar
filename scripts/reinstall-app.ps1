# Upgrade the installed cc-sidebar from the latest NSIS bundle and restart it. Waits for the old
# process to exit (otherwise the silent installer reports success without replacing the exe)
# and fails if the installed exe isn't the freshly built one.
$built = Get-Item "C:\dev\cc-sidebar\src-tauri\target\release\cc-sidebar.exe"
$setup = Get-ChildItem "C:\dev\cc-sidebar\src-tauri\target\release\bundle\nsis\*-setup.exe" | Sort-Object LastWriteTime | Select-Object -Last 1
# Ask it to close (it gives the reserved space back and restores desktop icons first);
# force-kill only if it doesn't go within 15 s, since a killed AppBar scatters desktop icons.
if (Get-Process cc-sidebar -ErrorAction SilentlyContinue) {
  taskkill.exe /IM cc-sidebar.exe | Out-Null
  $deadline = (Get-Date).AddSeconds(15)
  while ((Get-Process cc-sidebar -ErrorAction SilentlyContinue) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 300 }
  Stop-Process -Name cc-sidebar -ErrorAction SilentlyContinue
}
while (Get-Process cc-sidebar -ErrorAction SilentlyContinue) { Start-Sleep -Milliseconds 300 }
Start-Sleep 2
$p = Start-Process $setup.FullName -ArgumentList "/S" -PassThru -Wait
$installed = Get-Item "$env:LOCALAPPDATA\cc-sidebar\cc-sidebar.exe"
if ($p.ExitCode -ne 0 -or $installed.Length -ne $built.Length -or $installed.LastWriteTime -lt $built.LastWriteTime.AddMinutes(-1)) {
  "FAILED: installer exit $($p.ExitCode); installed $($installed.LastWriteTime) vs built $($built.LastWriteTime)"; exit 1
}
Start-Process $installed.FullName
"reinstalled ($($installed.LastWriteTime))"
