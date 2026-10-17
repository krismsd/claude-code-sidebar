# Capture the full virtual desktop (all monitors) to a PNG.
# From WSL:  powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$(wslpath -w scripts/screenshot.ps1)" -Out 'C:\dev\cc-sidebar\.shots\latest.png'
# Optional:  -Monitor 0   (capture only that monitor index)   -MaxWidth 2560 (downscale for readability)
param(
  [Parameter(Mandatory = $true)][string]$Out,
  [int]$Monitor = -1,
  [int]$MaxWidth = 2560
)
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type -Namespace Native -Name Dpi -MemberDefinition '[DllImport("user32.dll")] public static extern bool SetProcessDPIAware();'
[Native.Dpi]::SetProcessDPIAware() | Out-Null

if ($Monitor -ge 0) { $b = [System.Windows.Forms.Screen]::AllScreens[$Monitor].Bounds }
else { $b = [System.Windows.Forms.SystemInformation]::VirtualScreen }

$bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($b.Left, $b.Top, 0, 0, $bmp.Size)
$g.Dispose()

if ($bmp.Width -gt $MaxWidth) {
  $h = [int]($bmp.Height * $MaxWidth / $bmp.Width)
  $small = New-Object System.Drawing.Bitmap $bmp, $MaxWidth, $h
  $bmp.Dispose(); $bmp = $small
}
New-Item -ItemType Directory -Force -Path (Split-Path $Out) | Out-Null
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
Write-Output $Out
