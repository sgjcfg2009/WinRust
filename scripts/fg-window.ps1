# Prints what topbar sees of the window in front, 10 s after start (switch to the game meanwhile).
# Used to tell why a game is or is not treated as fullscreen.
Add-Type @'
using System; using System.Runtime.InteropServices; using System.Text;
public static class Fg {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [StructLayout(LayoutKind.Sequential)] public struct MI { public int cb; public RECT mon, work; public int flags; }
  [DllImport("user32")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32")] public static extern int GetWindowLongW(IntPtr h, int i);
  [DllImport("user32")] public static extern bool IsZoomed(IntPtr h);
  [DllImport("user32")] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32")] public static extern IntPtr MonitorFromWindow(IntPtr h, uint f);
  [DllImport("user32")] public static extern bool GetMonitorInfoW(IntPtr m, ref MI mi);
  [DllImport("user32")] public static extern bool SetProcessDPIAware();
}
'@
[Fg]::SetProcessDPIAware() | Out-Null # topbar is system DPI aware too
Write-Host 'Switch to the game now, reading in 10 s...'
Start-Sleep 10
$h = [Fg]::GetForegroundWindow()
$r = New-Object Fg+RECT; [Fg]::GetWindowRect($h, [ref]$r) | Out-Null
$mi = New-Object Fg+MI; $mi.cb = [Runtime.InteropServices.Marshal]::SizeOf($mi)
[Fg]::GetMonitorInfoW([Fg]::MonitorFromWindow($h, 2), [ref]$mi) | Out-Null
$cls = New-Object Text.StringBuilder 64; [Fg]::GetClassNameW($h, $cls, 64) | Out-Null
$pid_ = 0; [Fg]::GetWindowThreadProcessId($h, [ref]$pid_) | Out-Null
$style = [Fg]::GetWindowLongW($h, -16)
[pscustomobject]@{
  Process = (Get-Process -Id $pid_ -ErrorAction SilentlyContinue).Name
  Class   = $cls.ToString()
  Caption = ($style -band 0xC00000) -eq 0xC00000
  Zoomed  = [Fg]::IsZoomed($h)
  Window  = "$($r.L),$($r.T) - $($r.R),$($r.B)"
  Monitor = "$($mi.mon.L),$($mi.mon.T) - $($mi.mon.R),$($mi.mon.B)"
} | Format-List | Out-String | Tee-Object "$env:TEMP\fg-window.txt"
Read-Host 'Saved to %TEMP%\fg-window.txt, Enter to close'
