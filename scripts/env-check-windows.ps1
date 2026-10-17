# cc-sidebar environment check (Windows side). Read-only: prints info, changes nothing.
# Run in PowerShell:  powershell -ExecutionPolicy Bypass -File .\env-check-windows.ps1
$ErrorActionPreference = 'SilentlyContinue'
function H($t) { "`n=== $t ===" }
function Try-Cmd($label, [scriptblock]$sb) { $r = & $sb 2>&1 | Out-String; if ([string]::IsNullOrWhiteSpace($r)) { "${label}: NOT FOUND" } else { "${label}: $($r.Trim())" } }

H 'Windows'
(Get-CimInstance Win32_OperatingSystem | Select-Object Caption, Version, BuildNumber | Format-List | Out-String).Trim()

H 'Windows Terminal'
Get-AppxPackage -Name 'Microsoft.WindowsTerminal*' | ForEach-Object { "$($_.Name) $($_.Version)" }
Try-Cmd 'wt.exe' { (Get-Command wt.exe).Source }
$wtSettings = Get-ChildItem "$env:LOCALAPPDATA\Packages\Microsoft.WindowsTerminal*\LocalState\settings.json" | Select-Object -First 1
if ($wtSettings) {
  $j = Get-Content $wtSettings.FullName -Raw | ConvertFrom-Json
  "windowingBehavior: $($j.windowingBehavior)"
  "profiles:"; $j.profiles.list | ForEach-Object { "  - $($_.name) | source=$($_.source) | commandline=$($_.commandline) | suppressApplicationTitle=$($_.suppressApplicationTitle)" }
}

H 'Monitors'
Add-Type -AssemblyName System.Windows.Forms
[System.Windows.Forms.Screen]::AllScreens | ForEach-Object { "$($_.DeviceName) primary=$($_.Primary) bounds=$($_.Bounds) workArea=$($_.WorkingArea)" }
"DPI scaling (LogPixels): $((Get-ItemProperty 'HKCU:\Control Panel\Desktop\WindowMetrics').AppliedDPI)"

H 'Build toolchain (Tauri prerequisites)'
Try-Cmd 'rustc' { rustc --version }
Try-Cmd 'cargo' { cargo --version }
Try-Cmd 'rustup default' { rustup default }
Try-Cmd 'node' { node --version }
Try-Cmd 'npm' { npm --version }
Try-Cmd 'pnpm' { pnpm --version }
Try-Cmd 'git' { git --version }
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
if (Test-Path $vswhere) { "MSVC C++ build tools: " + ((& $vswhere -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property displayName) -join ', ') } else { 'MSVC C++ build tools: vswhere NOT FOUND' }
$wv = Get-ItemProperty 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
"WebView2 runtime: $(if ($wv) { $wv.pv } else { 'NOT FOUND (may still be present per-user)' })"

H 'WSL'
Try-Cmd 'wsl --version' { wsl.exe --version }
"distros:"; (wsl.exe -l -v) -replace "`0", ''
$wslcfg = "$env:USERPROFILE\.wslconfig"
if (Test-Path $wslcfg) { ".wslconfig:"; Get-Content $wslcfg } else { '.wslconfig: none (default NAT networking)' }

H 'Done'
'Review the output above before sharing (it contains no secrets, but check profile commandlines).'
