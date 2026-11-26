# Dev helper: launch Condor, screenshot its window, optionally close it gracefully.
# powershell -File shot.ps1 -Exe C:\...\condor.exe -Out C:\...\shot.png [-CArgs "..."] [-Wait 5] [-Close]
param([string]$Exe, [string]$Out, [string]$CArgs = "", [int]$Wait = 5, [switch]$Close)
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System; using System.Runtime.InteropServices;
public class W {
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint f);
  public struct RECT { public int L, T, R, B; }
}
"@
[W]::SetProcessDPIAware() | Out-Null
if ($CArgs) { $p = Start-Process -FilePath $Exe -ArgumentList $CArgs -PassThru } else { $p = Start-Process -FilePath $Exe -PassThru }
Start-Sleep -Seconds $Wait
$p.Refresh()
$h = $p.MainWindowHandle
$r = New-Object W+RECT
[W]::GetWindowRect($h, [ref]$r) | Out-Null
$w = $r.R - $r.L; $ht = $r.B - $r.T
$bmp = New-Object System.Drawing.Bitmap $w, $ht
$g = [System.Drawing.Graphics]::FromImage($bmp)
$hdc = $g.GetHdc(); [W]::PrintWindow($h, $hdc, 2) | Out-Null; $g.ReleaseHdc($hdc)
$bmp.Save($Out)
"pid=$($p.Id) rect=$($r.L),$($r.T),$w x $ht"
if ($Close) { $p.CloseMainWindow() | Out-Null; $p.WaitForExit(5000) | Out-Null; "exited=$($p.HasExited)" }
