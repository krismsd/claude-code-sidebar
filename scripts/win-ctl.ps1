# Window control for tests (use only on windows our tests created).
#   win-ctl.ps1 -Hwnd <h> -Do min|max|restore     |   win-ctl.ps1 -Process cc-sidebar -Do rect
param([long]$Hwnd, [string]$Process, [Parameter(Mandatory = $true)][string]$Do)
Add-Type -Namespace Native -Name Ctl -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
[DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int c);
[DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
public struct RECT { public int Left, Top, Right, Bottom; }
'@
[Native.Ctl]::SetProcessDPIAware() | Out-Null
if ($Process) { $Hwnd = (Get-Process $Process | Where-Object MainWindowHandle -ne 0 | Select-Object -First 1).MainWindowHandle }
$h = [IntPtr]$Hwnd
switch ($Do) {
  'min' { [Native.Ctl]::ShowWindow($h, 6) | Out-Null }
  'max' { [Native.Ctl]::ShowWindow($h, 3) | Out-Null }
  'restore' { [Native.Ctl]::ShowWindow($h, 9) | Out-Null }
  'rect' { }
}
$r = New-Object Native.Ctl+RECT; [Native.Ctl]::GetWindowRect($h, [ref]$r) | Out-Null
"hwnd=$Hwnd rect=($($r.Left),$($r.Top))-($($r.Right),$($r.Bottom))"
