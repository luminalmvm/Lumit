# Capture a window to PNG. Finds it by title substring via EnumWindows
# (FindWindow through P/Invoke from PowerShell mangles the null lpClassName
# into an empty string and never matches), grabs it with PrintWindow flag 2
# (PW_RENDERFULLCONTENT) so an obscured or GPU-composited window still reads.
#
#   powershell -ExecutionPolicy Bypass -File capture.ps1 -Title Lumit -Out C:\x.png [-Width 1600 -Height 900]
#
# -Width/-Height resize the window first and wait a moment for the relayout.
param(
  [string]$Title = 'Lumit',
  [Parameter(Mandatory=$true)][string]$Out,
  [int]$Width = 0,
  [int]$Height = 0,
  # print = PrintWindow (works obscured); screen = front the window and copy
  # the desktop under it (works when the window's content is GPU-composited
  # and PrintWindow hands back an empty surface).
  [string]$Mode = 'print'
)

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
public class Win {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern void SwitchToThisWindow(IntPtr h, bool alt);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr h);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
  public static List<IntPtr> Windows = new List<IntPtr>();
  public static List<string> Titles = new List<string>();
  public static void Scan() {
    Windows.Clear(); Titles.Clear();
    EnumWindows(delegate(IntPtr h, IntPtr l) {
      if (!IsWindowVisible(h)) return true;
      StringBuilder sb = new StringBuilder(512);
      GetWindowTextW(h, sb, 512);
      string t = sb.ToString();
      if (t.Length == 0) return true;
      Windows.Add(h); Titles.Add(t);
      return true;
    }, IntPtr.Zero);
  }
}
"@

[Win]::Scan()
# Exact title first: the shell running the test usually has the repo path in
# its own title, and "Lumit" is a substring of that path.
$hwnd = [IntPtr]::Zero
for ($i = 0; $i -lt [Win]::Windows.Count; $i++) {
  if ([Win]::Titles[$i] -eq $Title) { $hwnd = [Win]::Windows[$i]; break }
}
if ($hwnd -eq [IntPtr]::Zero) {
  for ($i = 0; $i -lt [Win]::Windows.Count; $i++) {
    if ([Win]::Titles[$i] -like "*$Title*") { $hwnd = [Win]::Windows[$i]; break }
  }
}
if ($hwnd -eq [IntPtr]::Zero) {
  Write-Output "NOWINDOW: no visible window matching '*$Title*'. Visible titles:"
  [Win]::Titles | ForEach-Object { Write-Output "  [$_]" }
  exit 1
}

if ($Width -gt 0 -and $Height -gt 0) {
  # 0x0014 = SWP_NOMOVE | SWP_NOZORDER
  [void][Win]::SetWindowPos($hwnd, [IntPtr]::Zero, 0, 0, $Width, $Height, 0x0014)
  Start-Sleep -Milliseconds 700
}

$r = New-Object Win+RECT
[void][Win]::GetWindowRect($hwnd, [ref]$r)
$w = $r.Right - $r.Left
$h = $r.Bottom - $r.Top
if ($w -le 0 -or $h -le 0) { Write-Output "BADRECT: ${w}x${h}"; exit 1 }

if ($Mode -eq 'desktop') {
  Add-Type -AssemblyName System.Windows.Forms
  $b = [System.Windows.Forms.SystemInformation]::VirtualScreen
  $bmp = New-Object System.Drawing.Bitmap($b.Width, $b.Height)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  [void][Win]::ShowWindow($hwnd, 9)
  [Win]::SwitchToThisWindow($hwnd, $true)
  Start-Sleep -Milliseconds 800
  $g.CopyFromScreen($b.X, $b.Y, 0, 0, (New-Object System.Drawing.Size($b.Width, $b.Height)))
  $g.Dispose()
  $dir = Split-Path -Parent $Out
  if ($dir -and -not (Test-Path $dir)) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
  $bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
  $bmp.Dispose()
  Write-Output "OK desktop $($b.Width)x$($b.Height) -> $Out"
  exit 0
}

$bmp = New-Object System.Drawing.Bitmap($w, $h)
$g = [System.Drawing.Graphics]::FromImage($bmp)
if ($Mode -eq 'screen') {
  # SetForegroundWindow alone is refused for a process that does not own the
  # foreground; without this the capture is of whatever window is actually on
  # top at those coordinates.
  [void][Win]::ShowWindow($hwnd, 9)   # SW_RESTORE
  [Win]::SwitchToThisWindow($hwnd, $true)
  [void][Win]::BringWindowToTop($hwnd)
  [void][Win]::SetForegroundWindow($hwnd)
  Start-Sleep -Milliseconds 800
  [void][Win]::GetWindowRect($hwnd, [ref]$r)
  $g.CopyFromScreen($r.Left, $r.Top, 0, 0, (New-Object System.Drawing.Size($w, $h)))
} else {
  $dc = $g.GetHdc()
  $ok = [Win]::PrintWindow($hwnd, $dc, 2)
  $g.ReleaseHdc($dc)
  if (-not $ok) { $g.Dispose(); $bmp.Dispose(); Write-Output "PRINTWINDOW-FAILED"; exit 1 }
}
$g.Dispose()

$dir = Split-Path -Parent $Out
if ($dir -and -not (Test-Path $dir)) { New-Item -ItemType Directory -Force -Path $dir | Out-Null }
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
Write-Output "OK ${w}x${h} -> $Out"
