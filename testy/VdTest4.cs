using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

[ComImport, InterfaceType(ComInterfaceType.InterfaceIsIUnknown), Guid("372E1D3B-38D3-42E4-A15B-8AB2B178F513")]
public interface IApplicationView { }

[ComImport, InterfaceType(ComInterfaceType.InterfaceIsIUnknown), Guid("92CA9DCD-5622-4BBA-A805-5E9F541BD8C9")]
public interface IObjectArray {
  void GetCount(out int count);
  void GetAt(int index, ref Guid iid, [MarshalAs(UnmanagedType.Interface)] out object obj);
}

[ComImport, InterfaceType(ComInterfaceType.InterfaceIsIUnknown), Guid("3F07F4BE-B107-441A-AF0F-39D82529072C")]
public interface IVirtualDesktop {
  bool IsViewVisible(IApplicationView view);
  Guid GetId();
}

[ComImport, InterfaceType(ComInterfaceType.InterfaceIsIUnknown), Guid("53F5CA0B-158F-4124-900C-057158060B27")]
public interface IVirtualDesktopManagerInternal {
  int GetCount();
  void MoveViewToDesktop(IApplicationView view, IVirtualDesktop desktop);
  bool CanViewMoveDesktops(IApplicationView view);
  IVirtualDesktop GetCurrentDesktop();
  void GetDesktops(out IObjectArray desktops);
  [PreserveSig] int GetAdjacentDesktop(IVirtualDesktop from, int direction, out IVirtualDesktop desktop);
  void SwitchDesktop(IVirtualDesktop desktop);
  void SwitchDesktopAndMoveForegroundView(IVirtualDesktop desktop);
  IVirtualDesktop CreateDesktop();
  void MoveDesktop(IVirtualDesktop desktop, int nIndex);
  void RemoveDesktop(IVirtualDesktop desktop, IVirtualDesktop fallback);
}

[ComImport, InterfaceType(ComInterfaceType.InterfaceIsIUnknown), Guid("1841C6D7-4F9D-42C0-AF41-8747538F10E5")]
public interface IApplicationViewCollection {
  int GetViews(out IObjectArray array);
  int GetViewsByZOrder(out IObjectArray array);
  int GetViewsByAppUserModelId(string id, out IObjectArray array);
  int GetViewForHwnd(IntPtr hwnd, out IApplicationView view);
}

[ComImport, InterfaceType(ComInterfaceType.InterfaceIsIUnknown), Guid("a5cd92ff-29be-454c-8d04-d82879fb3f1b")]
public interface IVirtualDesktopManager {
  bool IsWindowOnCurrentVirtualDesktop(IntPtr topLevelWindow);
  Guid GetWindowDesktopId(IntPtr topLevelWindow);
  [PreserveSig] int MoveWindowToDesktop(IntPtr topLevelWindow, ref Guid desktopId);
}

[ComImport, InterfaceType(ComInterfaceType.InterfaceIsIUnknown), Guid("6D5140C1-7436-11CE-8034-00AA006009FA")]
public interface IServiceProvider10 {
  [PreserveSig] int QueryService(ref Guid service, ref Guid riid, out IntPtr obj);
}

public static class T {
  static IServiceProvider10 shell;
  static IVirtualDesktopManagerInternal I;
  static IApplicationViewCollection V;
  static IVirtualDesktopManager P;
  static readonly Guid SVC_VDMI = new Guid("C5E0CDCA-7B6E-41B2-9FC4-D93975CC467B");

  // ---------- Win32 ----------
  delegate void WinEventProc(IntPtr hook, uint ev, IntPtr hwnd, int idObj, int idChild, uint thread, uint time);
  [DllImport("user32.dll")] static extern IntPtr SetWinEventHook(uint min, uint max, IntPtr mod, WinEventProc proc, uint pid, uint tid, uint flags);
  [DllImport("user32.dll")] static extern bool UnhookWinEvent(IntPtr h);
  [DllImport("user32.dll")] static extern bool PeekMessage(out MSG m, IntPtr h, uint a, uint b, uint remove);
  [DllImport("user32.dll")] static extern bool TranslateMessage(ref MSG m);
  [DllImport("user32.dll")] static extern IntPtr DispatchMessage(ref MSG m);
  [StructLayout(LayoutKind.Sequential)] struct MSG { public IntPtr h; public uint msg; public IntPtr w, l; public uint t; public int x, y; }
  [DllImport("user32.dll")] static extern bool ShowWindowAsync(IntPtr h, int cmd);
  [DllImport("user32.dll")] static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] static extern bool IsZoomed(IntPtr h);
  [DllImport("user32.dll")] static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] static extern bool IsWindow(IntPtr h);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] static extern int GetWindowLong(IntPtr h, int i);
  [DllImport("user32.dll")] static extern IntPtr GetWindow(IntPtr h, uint cmd);
  [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr h, uint f);
  [DllImport("user32.dll")] static extern int GetWindowTextLength(IntPtr h);
  [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] static extern IntPtr MonitorFromWindow(IntPtr h, uint f);
  [DllImport("user32.dll")] static extern bool GetMonitorInfo(IntPtr m, ref MONITORINFO mi);
  [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc p, IntPtr l);
  delegate bool EnumProc(IntPtr h, IntPtr l);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [StructLayout(LayoutKind.Sequential)] struct MONITORINFO { public int cb; public RECT mon, work; public uint flags; }

  const uint EVENT_SYSTEM_MINIMIZESTART = 0x16, EVENT_SYSTEM_MINIMIZEEND = 0x17;
  const uint EVENT_OBJECT_DESTROY = 0x8001, EVENT_OBJECT_SHOW = 0x8002, EVENT_OBJECT_LOCATIONCHANGE = 0x800B;

  static void Pump(int ms) {
    var sw = Stopwatch.StartNew(); MSG m;
    while (sw.ElapsedMilliseconds < ms) {
      while (PeekMessage(out m, IntPtr.Zero, 0, 0, 1)) { TranslateMessage(ref m); DispatchMessage(ref m); }
      Thread.Sleep(10);
    }
  }

  // ---------- helpers ----------
  static bool IsRealAppWindow(IntPtr h) {
    if (!IsWindowVisible(h) || GetWindowTextLength(h) == 0) return false;
    int ex = GetWindowLong(h, -20);
    if ((ex & 0x80) != 0) return false;                       // WS_EX_TOOLWINDOW
    if (GetWindow(h, 4) != IntPtr.Zero && (ex & 0x40000) == 0) return false; // owned, not APPWINDOW
    return true;
  }
  public static bool IsFullscreen(IntPtr h) {
    int style = GetWindowLong(h, -16);
    if ((style & 0x00C00000) == 0x00C00000) return false;      // has WS_CAPTION
    RECT r; GetWindowRect(h, out r);
    var mi = new MONITORINFO { cb = Marshal.SizeOf(typeof(MONITORINFO)) };
    GetMonitorInfo(MonitorFromWindow(h, 2), ref mi);
    return r.L <= mi.mon.L && r.T <= mi.mon.T && r.R >= mi.mon.R && r.B >= mi.mon.B;
  }
  static int CountWindowsOn(Guid desk) {
    int n = 0;
    EnumWindows((h, l) => { if (IsRealAppWindow(h)) { try { if (P.GetWindowDesktopId(h) == desk) n++; } catch {} } return true; }, IntPtr.Zero);
    return n;
  }
  static List<IVirtualDesktop> Desktops() {
    IObjectArray a; I.GetDesktops(out a); int c; a.GetCount(out c);
    var l = new List<IVirtualDesktop>(); Guid iid = typeof(IVirtualDesktop).GUID;
    for (int i = 0; i < c; i++) { object o; a.GetAt(i, ref iid, out o); l.Add((IVirtualDesktop)o); }
    return l;
  }
  static int IndexOf(Guid id) { var d = Desktops(); for (int i = 0; i < d.Count; i++) if (d[i].GetId() == id) return i; return -1; }
  static string Order() { var s = new StringBuilder(); foreach (var d in Desktops()) s.Append(d.GetId().ToString().Substring(0, 8)).Append(' '); return s.ToString(); }
  static IApplicationView View(IntPtr h) { IApplicationView v; V.GetViewForHwnd(h, out v); return v; }

  static Process Spawn(string title, string mode) {
    string extra = mode == "max" ? "$f.WindowState='Maximized';" :
                   mode == "full" ? "$f.FormBorderStyle='None'; $f.StartPosition='Manual'; $f.Bounds=[Windows.Forms.Screen]::PrimaryScreen.Bounds;" : "";
    string cmd = "Add-Type -AssemblyName System.Windows.Forms; $f=New-Object Windows.Forms.Form; $f.Text='" + title + "'; " + extra + " $f.ShowDialog()";
    var psi = new ProcessStartInfo("powershell.exe", "-NoProfile -STA -Command \"" + cmd + "\"") { UseShellExecute = false, CreateNoWindow = true };
    return Process.Start(psi);
  }
  static IntPtr WaitHwnd(Process p) {
    for (int i = 0; i < 200; i++) { Pump(100); p.Refresh(); if (p.MainWindowHandle != IntPtr.Zero) return p.MainWindowHandle; }
    throw new Exception("okno se neobjevilo");
  }

  // --- simulace pravidel: kazde maximalizovane okno = nova plocha na konci ---
  static Dictionary<IntPtr, IVirtualDesktop> own = new Dictionary<IntPtr, IVirtualDesktop>();
  static Dictionary<Guid, IVirtualDesktop> originOf = new Dictionary<Guid, IVirtualDesktop>();
  static int Pos(IVirtualDesktop x) { return IndexOf(x.GetId()) + 1; }
  static string State() { return string.Format("ploch {0}, jsem na plose {1}", I.GetCount(), Pos(I.GetCurrentDesktop())); }
  static bool Exists(IVirtualDesktop x) { return IndexOf(x.GetId()) >= 0; }
  static void OnMaximize(IntPtr h) {
    var origin = I.GetCurrentDesktop();
    var nd = I.CreateDesktop();                      // vznikne na konci
    I.MoveViewToDesktop(View(h), nd); I.SwitchDesktop(nd);
    own[h] = nd; originOf[nd.GetId()] = origin;
  }
  static void OnExit(IntPtr h, bool closed) {
    var fd = own[h]; own.Remove(h);
    var origin = originOf[fd.GetId()];
    while (!Exists(origin)) origin = originOf[origin.GetId()];   // puvodni plocha mezitim zanikla
    if (!closed) I.MoveViewToDesktop(View(h), origin);
    Pump(300);
    if (CountWindowsOn(fd.GetId()) == 0) {
      bool viewing = I.GetCurrentDesktop().GetId() == fd.GetId();
      if (viewing) I.SwitchDesktop(origin);
      I.RemoveDesktop(fd, origin);
    } else if (I.GetCurrentDesktop().GetId() == fd.GetId()) I.SwitchDesktop(origin);
  }
  public static void RunF() {
    Guid s0 = SVC_VDMI, i0 = typeof(IVirtualDesktopManagerInternal).GUID; IntPtr o0;
    shell = (IServiceProvider10)Activator.CreateInstance(Type.GetTypeFromCLSID(new Guid("C2F03A33-21F5-47FA-B4BB-156362A2F239")));
    shell.QueryService(ref s0, ref i0, out o0); I = (IVirtualDesktopManagerInternal)Marshal.GetObjectForIUnknown(o0);
    { Guid s = typeof(IApplicationViewCollection).GUID, iid = s; IntPtr o; shell.QueryService(ref s, ref iid, out o); V = (IApplicationViewCollection)Marshal.GetObjectForIUnknown(o); }
    P = (IVirtualDesktopManager)Activator.CreateInstance(Type.GetTypeFromCLSID(new Guid("AA509086-5CA9-4C25-8F95-589D3C07B48A")));
    var start = I.GetCurrentDesktop(); int startCount = I.GetCount();
    Process p1 = null, p2 = null, p3 = null;
    try {
      Console.WriteLine("Start:                                   " + State());
      p1 = Spawn("VDTEST-F1", "normal"); var w1 = WaitHwnd(p1); Pump(300);
      ShowWindowAsync(w1, 3); Pump(400); OnMaximize(w1); Pump(500);
      Console.WriteLine("1) okno A maximalizovano:                " + State() + ", A je na plose " + Pos(own[w1]));
      p2 = Spawn("VDTEST-F2", "normal"); var w2 = WaitHwnd(p2); Pump(300);
      Console.WriteLine("2) okno B otevreno (z plochy s A) na plose " + (IndexOf(P.GetWindowDesktopId(w2)) + 1));
      ShowWindowAsync(w2, 3); Pump(400); OnMaximize(w2); Pump(500);
      Console.WriteLine("3) okno B maximalizovano:                " + State() + ", B je na plose " + Pos(own[w2]));
      PostMessage(w1, 0x10, IntPtr.Zero, IntPtr.Zero); Pump(1200); OnExit(w1, true); Pump(500);
      Console.WriteLine("4) okno A zavreno (na pozadi):           " + State());
      ShowWindowAsync(w2, 9); Pump(400); OnExit(w2, false); Pump(600);
      Console.WriteLine("5) okno B zmenseno:                      " + State() + ", B je na plose " + (IndexOf(P.GetWindowDesktopId(w2)) + 1) + " (puvodni plocha A zanikla -> o uroven vys)");
      ShowWindowAsync(w2, 3); Pump(400); OnMaximize(w2); Pump(500);
      p3 = Spawn("VDTEST-F3", "normal"); var w3 = WaitHwnd(p3); Pump(500);
      Console.WriteLine("6) B znovu max., na jeho plose otevreno okno C: " + State() + ", oken na plose: " + CountWindowsOn(own[w2].GetId()));
      ShowWindowAsync(w2, 6); Pump(400); OnExit(w2, false); Pump(600);
      Console.WriteLine("7) B minimalizovano, C zustava:          " + State() + " (plocha s C zachovana)");
    } finally {
      foreach (var x in new[] { p1, p2, p3 }) try { if (x != null && !x.HasExited) x.Kill(); } catch {}
      Pump(800);
      try { I.SwitchDesktop(start); } catch {}
      var all = Desktops(); for (int i = all.Count - 1; i >= startCount; i--) I.RemoveDesktop(all[i], start);
      Console.WriteLine("Uklizeno:                                " + State());
    }
  }
  public static void RunE() {
    Guid s0 = SVC_VDMI, i0 = typeof(IVirtualDesktopManagerInternal).GUID; IntPtr o0;
    shell = (IServiceProvider10)Activator.CreateInstance(Type.GetTypeFromCLSID(new Guid("C2F03A33-21F5-47FA-B4BB-156362A2F239")));
    shell.QueryService(ref s0, ref i0, out o0); I = (IVirtualDesktopManagerInternal)Marshal.GetObjectForIUnknown(o0);
    { Guid s = typeof(IApplicationViewCollection).GUID, iid = s; IntPtr o; shell.QueryService(ref s, ref iid, out o); V = (IApplicationViewCollection)Marshal.GetObjectForIUnknown(o); }
    P = (IVirtualDesktopManager)Activator.CreateInstance(Type.GetTypeFromCLSID(new Guid("AA509086-5CA9-4C25-8F95-589D3C07B48A")));
    var orig = I.GetCurrentDesktop(); var nd = I.CreateDesktop(); I.MoveDesktop(nd, IndexOf(orig.GetId()) + 1);
    var p = Process.Start("charmap.exe"); var h = WaitHwnd(p); Pump(300);
    try {
      foreach (int mode in new[] { 5, 8, 1 }) {
        ShowWindow(h, 0); Pump(300);
        I.SwitchDesktop(nd); Pump(800);
        ShowWindow(h, mode); Pump(1500);
        string r; try { var g = P.GetWindowDesktopId(h); r = g == nd.GetId() ? "NOVA plocha" : g == orig.GetId() ? "puvodni plocha" : g.ToString(); } catch (Exception ex) { r = "chyba " + ex.HResult.ToString("X"); }
        Console.WriteLine("   ShowWindow({0}): viditelne={1}, okno je na: {2}", mode, IsWindowVisible(h), r);
        I.SwitchDesktop(orig); Pump(600);
        try { I.MoveViewToDesktop(View(h), orig); } catch {}
      }
    } finally { try { I.SwitchDesktop(orig); } catch {} p.Kill(); Pump(500); I.RemoveDesktop(nd, orig); Console.WriteLine("   uklizeno, ploch: " + I.GetCount()); }
  }
  public static void Run() {
    // ===== A: probing known IIDs =====
    shell = (IServiceProvider10)Activator.CreateInstance(Type.GetTypeFromCLSID(new Guid("C2F03A33-21F5-47FA-B4BB-156362A2F239")));
    var known = new Dictionary<string, string> {
      { "F31574D6-B682-4CDC-BD56-1827860ABEC6", "Windows 10" },
      { "B2F925B9-5A0F-4D2E-9F4D-2B1507593C10", "Windows 11 21H2/22H2 (puvodni)" },
      { "A3175F2D-239C-4BD2-8AA0-EEBA8B0B138E", "Windows 11 22H2/23H2" },
      { "53F5CA0B-158F-4124-900C-057158060B27", "Windows 11 24H2+" },
    };
    Console.WriteLine("A) Zkouseni znamych variant rozhrani:");
    foreach (var k in known) {
      Guid s = SVC_VDMI, iid = new Guid(k.Key); IntPtr o;
      int hr = shell.QueryService(ref s, ref iid, out o);
      Console.WriteLine("   {0,-34} {1}", k.Value, hr == 0 ? "PODPOROVANO" : string.Format("ne (0x{0:X8})", hr));
      if (o != IntPtr.Zero) Marshal.Release(o);
    }
    { Guid s = SVC_VDMI, iid = typeof(IVirtualDesktopManagerInternal).GUID; IntPtr o; shell.QueryService(ref s, ref iid, out o);
      I = (IVirtualDesktopManagerInternal)Marshal.GetObjectForIUnknown(o); Marshal.Release(o); }
    { Guid s = typeof(IApplicationViewCollection).GUID, iid = s; IntPtr o; shell.QueryService(ref s, ref iid, out o);
      V = (IApplicationViewCollection)Marshal.GetObjectForIUnknown(o); Marshal.Release(o); }
    P = (IVirtualDesktopManager)Activator.CreateInstance(Type.GetTypeFromCLSID(new Guid("AA509086-5CA9-4C25-8F95-589D3C07B48A")));

    // ===== hook =====
    var events = new List<string>(); IntPtr watch = IntPtr.Zero;
    WinEventProc proc = (hk, ev, h, idObj, idChild, th, tm) => {
      if (idObj != 0 || h == IntPtr.Zero || h != watch) return;
      string state = !IsWindow(h) ? "zniceno" : IsIconic(h) ? "minimalizovano" : IsZoomed(h) ? "maximalizovano" : IsFullscreen(h) ? "fullscreen" : "normalni";
      string name = ev == EVENT_SYSTEM_MINIMIZESTART ? "MINIMIZESTART" : ev == EVENT_SYSTEM_MINIMIZEEND ? "MINIMIZEEND" :
                    ev == EVENT_OBJECT_DESTROY ? "DESTROY" : ev == EVENT_OBJECT_SHOW ? "SHOW" : "LOCATIONCHANGE";
      string e = name + "->" + state;
      if (events.Count == 0 || events[events.Count - 1] != e) events.Add(e);
    };
    var hooks = new[] {
      SetWinEventHook(EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZEEND, IntPtr.Zero, proc, 0, 0, 2),
      SetWinEventHook(EVENT_OBJECT_DESTROY, EVENT_OBJECT_SHOW, IntPtr.Zero, proc, 0, 0, 2),
      SetWinEventHook(EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_LOCATIONCHANGE, IntPtr.Zero, proc, 0, 0, 2),
    };

    var orig = I.GetCurrentDesktop(); Guid origId = orig.GetId();
    int origIdx = IndexOf(origId);
    Console.WriteLine("\nPlochy pred testem: {0} | aktualni je na pozici {1} | poradi: {2}", I.GetCount(), origIdx, Order());

    IVirtualDesktop nd = null; Process p = null, p2 = null, p3 = null;
    try {
      // ===== B: placement right after origin =====
      Console.WriteLine("\nB) Zarazeni nove plochy hned za puvodni:");
      nd = I.CreateDesktop(); Guid ndId = nd.GetId();
      Console.WriteLine("   po CreateDesktop je nova na pozici {0} (konec)", IndexOf(ndId));
      I.MoveDesktop(nd, origIdx + 1);
      Console.WriteLine("   po MoveDesktop je na pozici {0} (ocekavano {1}) | poradi: {2}", IndexOf(ndId), origIdx + 1, Order());

      // ===== C: event detection on a foreign window =====
      Console.WriteLine("\nC) Detekce udalosti ciziho okna:");
      p = Spawn("VDTEST-C", "normal"); IntPtr h = WaitHwnd(p); watch = h; Pump(300); events.Clear();
      ShowWindowAsync(h, 3); Pump(600); Console.WriteLine("   maximalizace : " + string.Join(", ", events)); events.Clear();
      // presun na novou plochu + pocitani oken
      I.MoveViewToDesktop(View(h), nd); Pump(300);
      Console.WriteLine("   okna na nove plose po presunu: {0}", CountWindowsOn(ndId));
      ShowWindowAsync(h, 9); Pump(600); Console.WriteLine("   obnoveni     : " + string.Join(", ", events)); events.Clear();
      ShowWindowAsync(h, 6); Pump(600); Console.WriteLine("   minimalizace : " + string.Join(", ", events)); events.Clear();
      ShowWindowAsync(h, 9); Pump(600); events.Clear();
      I.MoveViewToDesktop(View(h), orig); Pump(300);
      Console.WriteLine("   okna na nove plose po navratu: {0} -> plochu lze smazat", CountWindowsOn(ndId));
      PostMessage(h, 0x10, IntPtr.Zero, IntPtr.Zero); Pump(1500); Console.WriteLine("   zavreni      : " + string.Join(", ", events)); events.Clear();

      // ===== D: window opened directly maximized / fullscreen =====
      Console.WriteLine("\nD) Nove okno rovnou maximalizovane / fullscreen:");
      p2 = Spawn("VDTEST-D1", "max"); var h2 = WaitHwnd(p2); Pump(300);
      Console.WriteLine("   spusteno maximalizovane -> IsZoomed={0}", IsZoomed(h2));
      p3 = Spawn("VDTEST-D2", "full"); var h3 = WaitHwnd(p3); Pump(300);
      Console.WriteLine("   spusteno fullscreen (bez ramecku, cela obrazovka) -> IsZoomed={0}, IsFullscreen={1}", IsZoomed(h3), IsFullscreen(h3));
      p3.Kill(); p3 = null;

      // ===== E: fallback without internal API: hide -> switch -> show =====
      Console.WriteLine("\nE) Zalozni rezim (jen verejne funkce): skryt okno -> prepnout plochu -> zobrazit:");
      I.MoveViewToDesktop(View(h2), orig); Pump(300);
      Console.WriteLine("   okno je na: {0} (puvodni)", P.GetWindowDesktopId(h2).ToString().Substring(0, 8));
      ShowWindow(h2, 0); Pump(200);
      I.SwitchDesktop(nd); Pump(700);
      ShowWindow(h2, 5); Pump(500);
      Guid after = Guid.Empty;
      for (int i = 0; i < 30 && after == Guid.Empty; i++) { try { after = P.GetWindowDesktopId(h2); } catch { Pump(100); } }
      I.SwitchDesktop(orig); Pump(500);
      Console.WriteLine("   po zobrazeni na nove plose je okno na: {0} -> {1}", after.ToString().Substring(0, 8), after == ndId ? "PRESUNUTO (trik funguje)" : "nepresunuto");
    } finally {
      try { I.SwitchDesktop(orig); } catch {}
      foreach (var x in new[] { p, p2, p3 }) try { if (x != null && !x.HasExited) x.Kill(); } catch {}
      Pump(500);
      if (nd != null) { I.RemoveDesktop(nd, orig); Console.WriteLine("\nTestovaci plocha odstranena. Plochy: {0} | poradi: {1}", I.GetCount(), Order()); }
      foreach (var hk in hooks) UnhookWinEvent(hk);
      GC.KeepAlive(proc);
    }
  }
}




