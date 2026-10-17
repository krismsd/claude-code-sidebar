# Get or change the primary display's scaling, the way the Settings app does (undocumented
# DisplayConfig packet types -3 = get, -4 = set; values are steps relative to the recommended scale).
#   dpi.ps1            -> print current/recommended scale
#   dpi.ps1 -Rel <n>   -> set to recommended + n steps
param([Nullable[int]]$Rel)
Add-Type -TypeDefinition @'
using System; using System.Runtime.InteropServices;
public static class Dpi {
  [StructLayout(LayoutKind.Sequential)] public struct Hdr { public int type; public int size; public uint luidLow; public int luidHigh; public uint id; }
  [StructLayout(LayoutKind.Sequential)] public struct Get { public Hdr h; public int min; public int cur; public int max; }
  [StructLayout(LayoutKind.Sequential)] public struct Set { public Hdr h; public int rel; }
  [DllImport("user32.dll")] static extern int GetDisplayConfigBufferSizes(uint f, out uint np, out uint nm);
  [DllImport("user32.dll")] static extern int QueryDisplayConfig(uint f, ref uint np, byte[] p, ref uint nm, byte[] m, IntPtr t);
  [DllImport("user32.dll")] static extern int DisplayConfigGetDeviceInfo(ref Get g);
  [DllImport("user32.dll")] static extern int DisplayConfigSetDeviceInfo(ref Set s);
  static Hdr Source(int type, int size) {
    uint np, nm; GetDisplayConfigBufferSizes(2, out np, out nm);
    byte[] p = new byte[np * 72], m = new byte[nm * 64];
    int r = QueryDisplayConfig(2, ref np, p, ref nm, m, IntPtr.Zero); if (r != 0) throw new Exception("QueryDisplayConfig " + r);
    // first active path: sourceInfo { LUID adapterId; UINT32 id; ... }
    return new Hdr { type = type, size = size, luidLow = BitConverter.ToUInt32(p, 0), luidHigh = BitConverter.ToInt32(p, 4), id = BitConverter.ToUInt32(p, 8) };
  }
  public static int[] Read() { var g = new Get { h = Source(-3, Marshal.SizeOf(typeof(Get))) }; int r = DisplayConfigGetDeviceInfo(ref g); if (r != 0) throw new Exception("get " + r); return new[] { g.min, g.cur, g.max }; }
  public static int Write(int rel) { var s = new Set { h = Source(-4, Marshal.SizeOf(typeof(Set))), rel = rel }; return DisplayConfigSetDeviceInfo(ref s); }
}
'@
$steps = 100, 125, 150, 175, 200, 225, 250, 300, 350, 400, 450, 500
$v = [Dpi]::Read(); $rec = -$v[0]
"current: $($steps[$rec + $v[1]])% (rel $($v[1])), recommended: $($steps[$rec])%, range rel $($v[0])..$($v[2])"
if ($null -ne $Rel) {
  $r = [Dpi]::Write($Rel); Start-Sleep -Milliseconds 500; $v = [Dpi]::Read()
  "set rel $Rel -> result $r; now $($steps[$rec + $v[1]])%"
}
