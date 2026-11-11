$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

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
  [return: MarshalAs(UnmanagedType.IUnknown)]
  object QueryService(ref Guid service, ref Guid riid);
}

public static class VD {
  static readonly Guid CLSID_ImmersiveShell = new Guid("C2F03A33-21F5-47FA-B4BB-156362A2F239");
  static readonly Guid CLSID_VDMInternal = new Guid("C5E0CDCA-7B6E-41B2-9FC4-D93975CC467B");
  static readonly Guid CLSID_VDM = new Guid("AA509086-5CA9-4C25-8F95-589D3C07B48A");
  public static IVirtualDesktopManagerInternal Internal;
  public static IApplicationViewCollection Views;
  public static IVirtualDesktopManager Public;
  public static void Init() {
    var shell = (IServiceProvider10)Activator.CreateInstance(Type.GetTypeFromCLSID(CLSID_ImmersiveShell));
    Guid s = CLSID_VDMInternal, i = typeof(IVirtualDesktopManagerInternal).GUID;
    Internal = (IVirtualDesktopManagerInternal)shell.QueryService(ref s, ref i);
    Guid s2 = typeof(IApplicationViewCollection).GUID, i2 = s2;
    Views = (IApplicationViewCollection)shell.QueryService(ref s2, ref i2);
    Public = (IVirtualDesktopManager)Activator.CreateInstance(Type.GetTypeFromCLSID(CLSID_VDM));
  }
  public static IApplicationView ViewFor(IntPtr hwnd) { IApplicationView v; Views.GetViewForHwnd(hwnd, out v); return v; }
  public static int Count() { return Internal.GetCount(); }
  public static IVirtualDesktop Current() { return Internal.GetCurrentDesktop(); }
  public static IVirtualDesktop Create() { return Internal.CreateDesktop(); }
  public static Guid Id(object d) { return ((IVirtualDesktop)d).GetId(); }
  public static void Remove(object d, object fb) { Internal.RemoveDesktop((IVirtualDesktop)d, (IVirtualDesktop)fb); }
  public static bool CanMove(IntPtr h) { return Internal.CanViewMoveDesktops(ViewFor(h)); }
  public static void Move(IntPtr h, object d) { Internal.MoveViewToDesktop(ViewFor(h), (IVirtualDesktop)d); }
  public static Guid WinDesk(IntPtr h) { return Public.GetWindowDesktopId(h); }
  public static int PublicMove(IntPtr h, Guid g) { return Public.MoveWindowToDesktop(h, ref g); }
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindow(string cls, string title);
}
'@

[VD]::Init()
$orig = [VD]::Current()
"OK: interni API dostupne. Pocet ploch: $([VD]::Count()), aktualni: $([VD]::Id($orig))"

# spust cizi proces s oknem
$title = "VDTEST-" + (Get-Random)
$p = Start-Process powershell.exe -PassThru -ArgumentList '-NoProfile','-STA','-Command',"`"Add-Type -AssemblyName System.Windows.Forms; `$f=New-Object Windows.Forms.Form; `$f.Text='$title'; `$f.ShowDialog()`""
$hwnd = [IntPtr]::Zero
for ($k=0; $k -lt 160 -and $hwnd -eq [IntPtr]::Zero; $k++) { Start-Sleep -Milliseconds 250; $p.Refresh(); $hwnd = $p.MainWindowHandle }
if ($hwnd -eq [IntPtr]::Zero) { throw "okno nenalezeno" }
"Testovaci okno (PID $($p.Id)) hwnd=$hwnd, plocha: $([VD]::WinDesk($hwnd))"

$new = $null
try {
  $new = [VD]::Create()
  "Vytvorena plocha: $([VD]::Id($new)), pocet: $([VD]::Count())"
  $g = [VD]::Id($new)
  $hr = [VD]::PublicMove($hwnd, $g)
  "Verejne API MoveWindowToDesktop na cizi okno: HRESULT 0x{0:X8}" -f $hr
    "CanViewMoveDesktops: $([VD]::CanMove($hwnd))"
  [VD]::Move($hwnd, $new)
  Start-Sleep -Milliseconds 300
  "Po MoveViewToDesktop je okno na: $([VD]::WinDesk($hwnd)) (ocekavano $g)"
  [VD]::Move($hwnd, $orig)
  Start-Sleep -Milliseconds 300
  "Po navratu je okno na: $([VD]::WinDesk($hwnd))"
} finally {
  if ($new) { [VD]::Remove($new, $orig); "Testovaci plocha odstranena, pocet: $([VD]::Count())" }
  Stop-Process -Id $p.Id -ErrorAction SilentlyContinue
}






