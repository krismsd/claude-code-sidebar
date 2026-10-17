# Spike S2: list Windows Terminal tabs via UI Automation, or select the tab whose name contains
# -Match and bring its window to the foreground.
#   wt-tabs.ps1            -> list "hwnd | selected | tab name" for every WT window
#   wt-tabs.ps1 -Match '[cc:test1]'
#   wt-tabs.ps1 -Match '[cc:test1]' -Keys '{DOWN}~'   (SendKeys syntax; only sent if that window is foreground)
param([string]$Match, [string]$Keys)
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
Add-Type -Namespace Native -Name Win -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
[DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
[DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
[DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
[DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
'@
$A = [System.Windows.Automation.AutomationElement]
$Scope = [System.Windows.Automation.TreeScope]
$C = [System.Windows.Automation.PropertyCondition]
$sw = [Diagnostics.Stopwatch]::StartNew()
$wins = $A::RootElement.FindAll($Scope::Children, (New-Object $C($A::ClassNameProperty, 'CASCADIA_HOSTING_WINDOW_CLASS')))
$tabCond = New-Object $C($A::ControlTypeProperty, [System.Windows.Automation.ControlType]::TabItem)
foreach ($w in $wins) {
  $hwnd = [IntPtr]$w.Current.NativeWindowHandle
  foreach ($tab in $w.FindAll($Scope::Descendants, $tabCond)) {
    $name = $tab.Current.Name
    $sel = $tab.GetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern).Current.IsSelected
    if (-not $Match) { "{0} | {1} | {2}" -f $hwnd, $sel, $name; continue }
    if ($name.Contains($Match)) {
      $tab.GetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern).Select()
      if ([Native.Win]::IsIconic($hwnd)) { [Native.Win]::ShowWindow($hwnd, 9) | Out-Null }
      # Alt press/release lets a background process take the foreground.
      [Native.Win]::keybd_event(0x12, 0, 0, [UIntPtr]::Zero); [Native.Win]::keybd_event(0x12, 0, 2, [UIntPtr]::Zero)
      $ok = [Native.Win]::SetForegroundWindow($hwnd)
      "selected '$name' in $hwnd; SetForegroundWindow=$ok; foreground now $([Native.Win]::GetForegroundWindow()); $($sw.ElapsedMilliseconds) ms"
      if ($Keys) {
        Start-Sleep -Milliseconds 300
        if ([Native.Win]::GetForegroundWindow() -ne $hwnd) { 'NOT foreground - keys not sent'; exit 2 }
        Add-Type -AssemblyName System.Windows.Forms
        [System.Windows.Forms.SendKeys]::SendWait($Keys); "sent keys"
      }
      exit 0
    }
  }
}
if ($Match) { "no tab matching '$Match' ($($sw.ElapsedMilliseconds) ms)"; exit 1 }
"($($sw.ElapsedMilliseconds) ms)"
