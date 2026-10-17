# Drag with the left mouse button from (X1,Y1) to (X2,Y2) in physical pixels (for UI tests).
param([int]$X1, [int]$Y1, [int]$X2, [int]$Y2)
Add-Type -Namespace Native -Name Drag -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
[DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
[DllImport("user32.dll")] public static extern void mouse_event(uint f, int dx, int dy, uint d, UIntPtr e);
'@
[Native.Drag]::SetProcessDPIAware() | Out-Null
[Native.Drag]::SetCursorPos($X1, $Y1) | Out-Null; Start-Sleep -Milliseconds 150
[Native.Drag]::mouse_event(0x2, 0, 0, 0, [UIntPtr]::Zero); Start-Sleep -Milliseconds 150
for ($i = 1; $i -le 20; $i++) {
  [Native.Drag]::SetCursorPos($X1 + ($X2 - $X1) * $i / 20, $Y1 + ($Y2 - $Y1) * $i / 20) | Out-Null
  Start-Sleep -Milliseconds 30
}
Start-Sleep -Milliseconds 200
[Native.Drag]::mouse_event(0x4, 0, 0, 0, [UIntPtr]::Zero)
