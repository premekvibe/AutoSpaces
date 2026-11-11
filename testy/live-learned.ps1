# Live test of the learned list (plan 6.7). Visibly may switch your virtual desktop!
# Run:  pwsh -NoProfile -STA -File testy\live-learned.ps1   (needs target\release\autospaces.exe)
# Backs up %LOCALAPPDATA%\AutoSpaces\learned.json, writes a blocked entry for the test window's
# process/class, checks that maximize does NOTHING, then removes the entry, restarts the app
# and checks that the normal enter/exit happens. Restores the original learned.json at the end.
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
  public static uint PidOf(IntPtr h) { uint p; GetWindowThreadProcessId(h, out p); return p; }
  // Polls every 5 ms for ms milliseconds; returns "" when desktop count, current desktop and the
  // window's desktop all stayed as given, else a description of the first change.
  public static string Watch(IntPtr h, int baseCount, Guid cur0, Guid win0, int ms) {
    Stopwatch sw = Stopwatch.StartNew();
    while (sw.ElapsedMilliseconds < ms) {
      Guid[] ids = Ids();
      if (ids.Length != baseCount) return "desktop count " + baseCount + " -> " + ids.Length + " at " + sw.ElapsedMilliseconds + " ms";
      if (Cur() != cur0) return "current desktop changed at " + sw.ElapsedMilliseconds + " ms";
      if (WinDesk(h) != win0) return "window desktop changed at " + sw.ElapsedMilliseconds + " ms";
      Thread.Sleep(5);
    }
    return "";
  }
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
      if (entered) good = (ids.Length == baseCount || ids.Length == baseCount + 1) && cur != origin && (!checkWin || wd == cur);
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
function Check($name, $ok, $detail) {
  if ($ok) { Write-Host ("PASS {0,-58} {1}" -f $name, $detail) }
  else { $script:fail++; Write-Host ("FAIL {0,-58} {1}" -f $name, $detail) -ForegroundColor Red }
}

$dir = Join-Path $env:LOCALAPPDATA 'AutoSpaces'
New-Item -ItemType Directory -Force $dir | Out-Null
$logFile = Join-Path $dir 'log.txt'
$learned = Join-Path $dir 'learned.json'
$backup = Join-Path $dir 'learned.json.livebak'
$hadLearned = Test-Path $learned
if ($hadLearned) { Copy-Item $learned $backup -Force; Remove-Item $learned -Force }

function Start-App() {
  $env:AUTOSPACES_LOG = '1'
  $p = Start-Process $exe -PassThru
  Start-Sleep -Seconds 2
  return $p
}
function Stop-App($p) {
  if ($p -and -not $p.HasExited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue }
  Get-Process autospaces -ErrorAction SilentlyContinue | Stop-Process -Force
  Start-Sleep -Milliseconds 300
}
function LogSince($n) { if (Test-Path $logFile) { @(Get-Content $logFile) | Select-Object -Skip $n } else { @() } }
function LogCount() { if (Test-Path $logFile) { @(Get-Content $logFile).Count } else { 0 } }

$asProc = $null; $winProc = $null; $hwnd = [IntPtr]::Zero
$baseCount = [AsLive]::Ids().Length
$origin = [AsLive]::Cur()
$logStart = LogCount
try {
  # test window: conhost (the app reports it as cmd.exe / ConsoleWindowClass)
  $winProc = Start-Process conhost.exe -ArgumentList "cmd.exe /k title $title" -PassThru
  for ($i = 0; $i -lt 100 -and $hwnd -eq [IntPtr]::Zero; $i++) { Start-Sleep -Milliseconds 100; $hwnd = [AsLive]::Find($title, $PID) }
  if ($hwnd -eq [IntPtr]::Zero) { throw 'test window not found' }
  $origin = [AsLive]::WinDesk($hwnd)
  $wpid = [AsLive]::PidOf($hwnd)
  $exePath = (Get-Process -Id $wpid).MainModule.FileName
  $item = Get-Item $exePath
  $stamp = "{0}:{1}" -f $item.Length, ([DateTimeOffset]$item.LastWriteTimeUtc).ToUnixTimeSeconds()
  Write-Host ("Test window hwnd=0x{0:X} pid={1} exe={2} stamp={3}" -f [int64]$hwnd, $wpid, $exePath, $stamp)

  # 0) probe: backend name and the process/class the app reports (no learned.json present)
  $mark = LogCount
  $asProc = Start-App
  $probe = LogSince $mark
  $m = $probe | Where-Object { $_ -match '^backend: (.+)$' } | Select-Object -First 1
  if (-not $m) { throw "no 'backend:' line in log: $($probe -join ' | ')" }
  $backend = ($m -replace '^backend: ', '').Trim()
  [void][AsLive]::Show($hwnd, 3); Start-Sleep -Milliseconds 1500
  $enter = LogSince $mark | Where-Object { $_ -match '^enter 0x[0-9a-f]+ (\S+) (\S+) ' } | Select-Object -First 1
  Check 'probe: app reports process/class of the test window' ($enter -match ' cmd\.exe ConsoleWindowClass ') $enter
  [void][AsLive]::Show($hwnd, 9); Start-Sleep -Milliseconds 1500
  Stop-App $asProc
  if ([AsLive]::Ids().Length -ne $baseCount) { throw 'probe left an extra desktop' }
  if (Test-Path $learned) { Remove-Item $learned -Force }

  # 1) blocked entry -> nothing may happen
  $today = (Get-Date).ToString('yyyy-MM-dd')
  $json = '{"version":1,"entries":[{"process":"cmd.exe","class":"ConsoleWindowClass","reason":"CannotMove","strikes":1,"blocked":true,"backend":"' + $backend + '","exeStamp":"' + $stamp + '","first":"' + $today + '","last":"' + $today + '"}]}'
  Set-Content -Path $learned -Value $json -Encoding utf8
  $mark = LogCount
  $asProc = Start-App
  $cur0 = [AsLive]::Cur(); $win0 = [AsLive]::WinDesk($hwnd)
  [void][AsLive]::Show($hwnd, 3)
  $chg = [AsLive]::Watch($hwnd, $baseCount, $cur0, $win0, 500)
  Check '(1) blocked: nothing changes within 500 ms of maximize' ($chg -eq '') $chg
  Start-Sleep -Milliseconds 800
  Check '(1) window really is maximized' ([AsLive]::Zoomed($hwnd)) ''
  $chg = [AsLive]::Watch($hwnd, $baseCount, $cur0, $win0, 300)
  Check '(1) still nothing after 1.3 s' ($chg -eq '') $chg
  $skip = LogSince $mark | Where-Object { $_ -match 'on learned list' } | Select-Object -First 1
  Check '(1) log says the window was skipped (learned list)' ([bool]$skip) $skip
  [void][AsLive]::Show($hwnd, 9); Start-Sleep -Milliseconds 800
  Stop-App $asProc

  # 2) entry removed, restart -> normal enter / exit
  Remove-Item $learned -Force
  $asProc = Start-App
  $ms = [AsLive]::Run($hwnd, 3, $true, $baseCount, $origin, $true, 3000)
  Check '(2) no entry: maximize -> new desktop, window + current there' ($ms -ge 0) ("{0} ms {1}" -f $ms, [AsLive]::Last)
  Start-Sleep -Milliseconds 1500
  $ms = [AsLive]::Run($hwnd, 9, $false, $baseCount, $origin, $true, 3000)
  Check '(2) restore -> back on origin, desktop removed' ($ms -ge 0) ("{0} ms {1}" -f $ms, [AsLive]::Last)
  Start-Sleep -Milliseconds 500
}
finally {
  Get-Process autospaces -ErrorAction SilentlyContinue | Stop-Process -Force
  if ($hwnd -ne [IntPtr]::Zero -and [AsLive]::Alive($hwnd)) { [void][AsLive]::Show($hwnd, 9); Start-Sleep -Milliseconds 300 }
  if ($winProc) {
    Get-Process -Id $winProc.Id -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Get-CimInstance Win32_Process -Filter "ParentProcessId=$($winProc.Id)" -ErrorAction SilentlyContinue | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
  }
  Start-Sleep -Milliseconds 300
  if (Test-Path $learned) { Remove-Item $learned -Force }
  if ($hadLearned) { Move-Item $backup $learned -Force; Write-Host 'Original learned.json restored.' }
  $now = [AsLive]::Ids().Length
  if ($now -ne $baseCount) {
    $script:fail++
    Write-Host "!!!! CLEANUP PROBLEM: $($now - $baseCount) extra virtual desktop(s) remain (was $baseCount, now $now). !!!!" -ForegroundColor Red -BackgroundColor Black
  } else { Write-Host "Cleanup OK: $now desktops, as before." }
  if ([AsLive]::Cur() -ne $origin) { Write-Host 'NOTE: current desktop differs from the initial one.' -ForegroundColor Yellow }
  if (Test-Path $logFile) { Write-Host '--- autospaces log (this run) ---'; Get-Content $logFile | Select-Object -Skip $logStart | Select-Object -First 40 }
  $left = @(Get-Process autospaces -ErrorAction SilentlyContinue).Count
  Write-Host "autospaces processes left: $left; failed checks: $($script:fail)"
}
