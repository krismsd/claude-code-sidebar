# Left-click at physical screen coordinates (for UI tests), then restore the cursor position.
# -MoveOnly: just move the cursor there (to test hover) and leave it. -Stay: click and leave it there.
param([Parameter(Mandatory = $true)][int]$X, [Parameter(Mandatory = $true)][int]$Y, [switch]$MoveOnly, [switch]$Stay)
Add-Type -Namespace Native -Name Mouse -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
[DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
[DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT p);
[DllImport("user32.dll")] public static extern void mouse_event(uint f, int dx, int dy, uint d, UIntPtr e);
public struct POINT { public int X, Y; }
'@
[Native.Mouse]::SetProcessDPIAware() | Out-Null
$p = New-Object Native.Mouse+POINT; [Native.Mouse]::GetCursorPos([ref]$p) | Out-Null
[Native.Mouse]::SetCursorPos($X, $Y) | Out-Null
if ($MoveOnly) { exit 0 }
Start-Sleep -Milliseconds 100
[Native.Mouse]::mouse_event(0x2, 0, 0, 0, [UIntPtr]::Zero); [Native.Mouse]::mouse_event(0x4, 0, 0, 0, [UIntPtr]::Zero)
Start-Sleep -Milliseconds 300
if (-not $Stay) { [Native.Mouse]::SetCursorPos($p.X, $p.Y) | Out-Null }
