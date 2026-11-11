# Live test of AutoSpaces (visibly switches your virtual desktop!).
# Run:  pwsh -NoProfile -STA -File testy\live.ps1
# Builds nothing: expects target\release\autospaces.exe. Starts it with AUTOSPACES_LOG=1,
# spawns ONE maximizable foreign window and checks maximize / restore / minimize / close.
# Desktop state is read from the registry (HKCU\...\Explorer\VirtualDesktops) and the public
# IVirtualDesktopManager.GetWindowDesktopId; latency = ShowWindow call -> expected state seen (5 ms poll).
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$exe = Join-Path $root 'target\release\autospaces.exe'
$title = 'AutoSpacesLive'
if (-not (Test-Path $exe)) { throw "Missing $exe (run cargo build --release first)" }
if (Get-Process autospaces -ErrorAction SilentlyContinue) { throw 'autospaces.exe is already running; stop it first.' }

Add-Type -TypeDefinition @'
using System;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using Microsoft.Win32;

[ComImport, InterfaceType(ComInterfaceType.InterfaceIsIUnknown), Guid("a5cd92ff-29be-454c-8d04-d82879fb3f1b")]
public interface IVDM {
  [PreserveSig] int IsWindowOnCurrentVirtualDesktop(IntPtr h, out bool b);
  [PreserveSig] int GetWindowDesktopId(IntPtr h, out Guid g);
  [PreserveSig] int MoveWindowToDesktop(IntPtr h, ref Guid g);
}
[ComImport, Guid("aa509086-5ca9-4c25-8f95-589d3c07b48a")] public class VDMClass { }

public static class AsLive {
  delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc p, IntPtr l);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] static extern bool IsWindow(IntPtr h);
  [DllImport("user32.dll")] static extern bool IsZoomed(IntPtr h);
  [DllImport("user32.dll")] static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] static extern bool ShowWindowAsync(IntPtr h, int cmd);
  [DllImport("user32.dll")] static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);

  static IVDM vdm = (IVDM)new VDMClass();
  static RegistryKey key = Registry.CurrentUser.OpenSubKey(@"SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\VirtualDesktops");
  public static string Last = "";

  public static Guid[] Ids() {
    byte[] b = (byte[])key.GetValue("VirtualDesktopIDs");
    if (b == null) return new Guid[0];
    Guid[] r = new Guid[b.Length / 16];
    for (int i = 0; i < r.Length; i++) { byte[] g = new byte[16]; Array.Copy(b, i * 16, g, 0, 16); r[i] = new Guid(g); }
    return r;
  }
  public static Guid Cur() {
    byte[] b = (byte[])key.GetValue("CurrentVirtualDesktop");
    return (b != null && b.Length == 16) ? new Guid(b) : Guid.Empty;
  }
  public static Guid WinDesk(IntPtr h) { Guid g; return vdm.GetWindowDesktopId(h, out g) == 0 ? g : Guid.Empty; }
  public static bool Alive(IntPtr h) { return IsWindow(h); }
  public static bool Zoomed(IntPtr h) { return IsZoomed(h); }
  public static bool Iconic(IntPtr h) { return IsIconic(h); }
  public static bool Show(IntPtr h, int cmd) { return ShowWindowAsync(h, cmd); }

  public static IntPtr Find(string title, uint excludePid) {
    IntPtr found = IntPtr.Zero;
    EnumWindows((h, l) => {
      if (!IsWindowVisible(h)) return true;
      StringBuilder sb = new StringBuilder(256); GetWindowText(h, sb, 256);
      uint pid; GetWindowThreadProcessId(h, out pid);
      if (pid != excludePid && sb.ToString() == title) { found = h; return false; }
      return true;
    }, IntPtr.Zero);
    return found;
  }

  static int Idx(Guid[] ids, Guid g) { return Array.IndexOf(ids, g) + 1; }

  // Performs the action (ShowWindow cmd, or close when cmd < 0) and polls every 5 ms until the
  // expected state shows up. Returns ms, or -1 on timeout. Last = description of the final state.
  public static long Run(IntPtr h, int cmd, bool entered, int baseCount, Guid origin, bool checkWin, int timeoutMs) {
    Stopwatch sw = Stopwatch.StartNew();
    if (cmd >= 0) ShowWindowAsync(h, cmd); else PostMessage(h, 0x10, IntPtr.Zero, IntPtr.Zero);
    long ok = -1;
    string desc = "";
    while (sw.ElapsedMilliseconds < timeoutMs) {
      Guid[] ids = Ids(); Guid cur = Cur(); Guid wd = checkWin ? WinDesk(h) : Guid.Empty;
      desc = "desktops=" + ids.Length + " current=#" + Idx(ids, cur) + (checkWin ? " window=#" + Idx(ids, wd) : "");
      bool good;
      if (entered) good = ids.Length == baseCount + 1 && ids.Length > 0 && cur == ids[ids.Length - 1] && (!checkWin || wd == cur);
      else good = ids.Length == baseCount && cur == origin && (!checkWin || wd == origin);
      if (good) { ok = sw.ElapsedMilliseconds; break; }
      Thread.Sleep(5);
    }
    Last = desc;
    return ok;
  }
}
'@

$script:fail = 0
function Report($name, $ms, $expect) {
  if ($ms -ge 0) { Write-Host ("PASS {0,-46} {1,5} ms   {2}" -f $name, $ms, [AsLive]::Last) }
  else { $script:fail++; Write-Host ("FAIL {0,-46} timeout    expected: {1}; got: {2}" -f $name, $expect, [AsLive]::Last) -ForegroundColor Red }
}

$asProc = $null; $winProc = $null; $hwnd = [IntPtr]::Zero
$baseCount = [AsLive]::Ids().Length
$origin = [AsLive]::Cur()
$logFile = Join-Path $env:LOCALAPPDATA 'AutoSpaces\log.txt'
$logStart = if (Test-Path $logFile) { @(Get-Content $logFile).Count } else { 0 }

function Start-TestWindow($kind) {
  if ($kind -eq 'conhost') {
    $p = Start-Process conhost.exe -ArgumentList "cmd.exe /k title $title" -PassThru
  } else {
    $cmd = "Add-Type -AssemblyName System.Windows.Forms; `$f = New-Object Windows.Forms.Form; `$f.Text = '$title'; `$f.Width = 640; `$f.Height = 400; [void]`$f.ShowDialog()"
    $p = Start-Process powershell.exe -ArgumentList '-NoProfile', '-STA', '-Command', $cmd -PassThru
  }
  $h = [IntPtr]::Zero
  for ($i = 0; $i -lt 100 -and $h -eq [IntPtr]::Zero; $i++) { Start-Sleep -Milliseconds 100; $h = [AsLive]::Find($title, $PID) }
  return @($p, $h)
}

try {
  Write-Host "Desktops: $baseCount, current: $origin"
  # 1) test window (before AutoSpaces runs): conhost first, WinForms fallback if it cannot be maximized.
  $kind = 'conhost'
  $r = Start-TestWindow $kind; $winProc = $r[0]; $hwnd = $r[1]
  $ok = $false
  if ($hwnd -ne [IntPtr]::Zero) {
    [void][AsLive]::Show($hwnd, 3); Start-Sleep -Milliseconds 600
    $ok = [AsLive]::Zoomed($hwnd)
    [void][AsLive]::Show($hwnd, 9); Start-Sleep -Milliseconds 600
  }
  if (-not $ok) {
    Write-Host "conhost window not found or not maximizable; falling back to a WinForms window" -ForegroundColor Yellow
    Get-Process conhost, cmd -ErrorAction SilentlyContinue | Where-Object { $_.Id -eq $winProc.Id } | Stop-Process -Force -ErrorAction SilentlyContinue
    if ($hwnd -ne [IntPtr]::Zero) { [void][AsLive]::Show($hwnd, 9) }
    $kind = 'winforms'
    $r = Start-TestWindow $kind; $winProc = $r[0]; $hwnd = $r[1]
    if ($hwnd -eq [IntPtr]::Zero) { throw 'test window not found' }
  }
  Write-Host ("Test window ($kind) hwnd=0x{0:X} on desktop {1}" -f [int64]$hwnd, [AsLive]::WinDesk($hwnd))
  $origin = [AsLive]::WinDesk($hwnd)
  if ($origin -ne [AsLive]::Cur()) { Write-Host 'WARN: window is not on the current desktop' -ForegroundColor Yellow }

  # 2) AutoSpaces
  $env:AUTOSPACES_LOG = '1'
  $asProc = Start-Process $exe -PassThru
  Start-Sleep -Seconds 5
  $asProc.Refresh()
  Write-Host ("autospaces idle after 5 s: private {0:N0} KB, working set {1:N0} KB" -f ($asProc.PrivateMemorySize64 / 1KB), ($asProc.WorkingSet64 / 1KB))

  # (a) maximize
  $ms = [AsLive]::Run($hwnd, 3, $true, $baseCount, $origin, $true, 3000)
  Report '(a) maximize -> new desktop, window + current there' $ms 'count+1, window and current on last'
  Start-Sleep -Milliseconds 1200
  # (b) restore
  $ms = [AsLive]::Run($hwnd, 9, $false, $baseCount, $origin, $true, 3000)
  Report '(b) restore -> back on origin, desktop removed' $ms 'count back, window and current on origin'
  Start-Sleep -Milliseconds 1200
  # (c) maximize, minimize
  $ms = [AsLive]::Run($hwnd, 3, $true, $baseCount, $origin, $true, 3000)
  Report '(c1) maximize again' $ms 'count+1, window and current on last'
  Start-Sleep -Milliseconds 1200
  $ms = [AsLive]::Run($hwnd, 6, $false, $baseCount, $origin, $true, 3000)
  Report '(c2) minimize -> back on origin, desktop removed' $ms 'count back, window and current on origin'
  Start-Sleep -Milliseconds 1200
  # (d) maximize (from minimized), close
  $ms = [AsLive]::Run($hwnd, 3, $true, $baseCount, $origin, $true, 3000)
  Report '(d1) maximize again (from minimized)' $ms 'count+1, window and current on last'
  Start-Sleep -Milliseconds 1200
  $ms = [AsLive]::Run($hwnd, -1, $false, $baseCount, $origin, $false, 3000)
  Report '(d2) close (WM_CLOSE) -> current on origin, desktop removed' $ms 'count back, current on origin'
  Start-Sleep -Milliseconds 500
}
finally {
  if ($asProc -and -not $asProc.HasExited) { Stop-Process -Id $asProc.Id -Force -ErrorAction SilentlyContinue }
  if ($hwnd -ne [IntPtr]::Zero -and [AsLive]::Alive($hwnd)) {
    [void][AsLive]::Show($hwnd, 9); Start-Sleep -Milliseconds 300
  }
  if ($winProc) {
    Get-Process -Id $winProc.Id -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Get-CimInstance Win32_Process -Filter "ParentProcessId=$($winProc.Id)" -ErrorAction SilentlyContinue | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
  }
  Get-Process autospaces -ErrorAction SilentlyContinue | Stop-Process -Force
  Start-Sleep -Milliseconds 300
  $now = [AsLive]::Ids().Length
  if ($now -ne $baseCount) {
    $script:fail++
    Write-Host "!!!! CLEANUP PROBLEM: $($now - $baseCount) extra virtual desktop(s) remain (was $baseCount, now $now). Remove them manually (Win+Tab). !!!!" -ForegroundColor Red -BackgroundColor Black
  } else { Write-Host "Cleanup OK: $now desktops, as before." }
  if ([AsLive]::Cur() -ne $origin) { Write-Host 'NOTE: current desktop differs from the initial one.' -ForegroundColor Yellow }
  if (Test-Path $logFile) {
    Write-Host '--- autospaces log (this run) ---'
    Get-Content $logFile | Select-Object -Skip $logStart | Select-Object -First 40
  }
  $left = @(Get-Process autospaces -ErrorAction SilentlyContinue).Count
  Write-Host "autospaces processes left: $left; failed checks: $($script:fail)"
}
