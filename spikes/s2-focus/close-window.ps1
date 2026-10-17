# Close a Windows Terminal window by handle (only use on windows our tests created).
param([Parameter(Mandatory = $true)][long]$Hwnd)
Add-Type -Namespace Native -Name Close -MemberDefinition '[DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);'
[Native.Close]::PostMessage([IntPtr]$Hwnd, 0x10, [IntPtr]::Zero, [IntPtr]::Zero)
# WT asks "Do you want to close all tabs?" for multi-tab windows: press "Close all" in THIS window only.
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$A = [System.Windows.Automation.AutomationElement]
for ($i = 0; $i -lt 10; $i++) {
  Start-Sleep -Milliseconds 300
  try { $w = $A::FromHandle([IntPtr]$Hwnd) } catch { 'window closed'; exit 0 }
  $btn = $w.FindFirst([System.Windows.Automation.TreeScope]::Descendants,
    (New-Object System.Windows.Automation.PropertyCondition($A::NameProperty, 'Close all')))
  if ($btn) { $btn.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke(); 'confirmed close all'; exit 0 }
}
'no confirm dialog found'
