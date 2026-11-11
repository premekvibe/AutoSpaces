# AutoSpaces – plán stavby aplikace

> Pracovní název: **AutoSpaces**. Dokument je určen k revizi a úpravám. Každé tvrzení je označeno:
> **[OVĚŘENO]** = prakticky vyzkoušeno na referenčním stroji, **[PŘEVZATO]** = z komunitních zdrojů, neověřeno, **[NEOVĚŘENO]** = předpoklad, nutno ověřit.
>
> Referenční stroj: Windows 11 Pro, build 26340.9882 (DisplayVersion 26H2), `twinui.pcshell.dll` 10.0.26100.9949 (= rodina 24H2), PowerShell 7.6, bez .NET SDK.

---

## 1. Cíl

Standalone aplikace pro Windows 10/11, která napodobí chování fullscreen „Spaces“ z macOS:

1. Když se okno **maximalizuje**, **přepne do fullscreenu** nebo se **rovnou otevře maximalizované/fullscreen**, vytvoří se **nová virtuální plocha na konci** seznamu, okno se na ni přesune a plocha se zobrazí.
2. Když se takové okno **zmenší (restore)**, **minimalizuje**, **zavře** nebo **skryje**, vrátí se (pokud ještě existuje) na **původní plochu** – tu, na které bylo před maximalizací.
3. Pokud na pomocné ploše **nezůstalo žádné jiné okno**, plocha se **automaticky smaže**.
4. Pokud se uživatel na pomocnou plochu právě dívá, přepne se zpět na původní plochu.
5. Musí fungovat na **všech verzích Windows s virtuálními plochami** (Windows 10 i 11), s co nejmenší závislostí na konkrétní verzi.

### Upřesněné požadavky uživatele
- Pořadí ploch je nepodstatné: nová plocha **vždy na konci** (není potřeba `MoveDesktop`).
- Každé maximalizované okno = vlastní plocha (2 plochy → maximalizace → plocha 3; další maximalizace → plocha 4).
- Plocha se maže, jen pokud je prázdná.

### Mimo rozsah (v1)
- Samostatné plochy pro každý monitor (Windows to neumí – viz rizika).
- Animace přechodů navíc nad rámec systémových.
- Exkluzivní fullscreen her (DirectX exclusive) – detekce ano, ale defaultně vyloučeno.

---

## 2. Výsledky ověření proveditelnosti

Testovací skripty: `testy/` (`vdtest.ps1`, `run2.ps1`+`VdTest2.cs`, `run3.ps1`+`VdTest3.cs`, `run4.ps1`+`VdTest4.cs`). Spouštět `pwsh -NoProfile -STA -File testy\runX.ps1`.

| # | Ověřovaná věc | Výsledek |
|---|---|---|
| 1 | Získání `IVirtualDesktopManagerInternal` přes `ImmersiveShell` + `IServiceProvider::QueryService` | **[OVĚŘENO]** |
| 2 | Výběr varianty rozhraní zkoušením známých IID (QueryService vrací `E_NOINTERFACE` pro nesprávné) | **[OVĚŘENO]** – prošla jen varianta 24H2 |
| 3 | `CreateDesktop` (vznikne na konci), `RemoveDesktop(desk, fallback)` | **[OVĚŘENO]** |
| 4 | Veřejné `IVirtualDesktopManager::MoveWindowToDesktop` na okno cizího procesu | **[OVĚŘENO] – selže `0x80070005` (E_ACCESSDENIED)** |
| 5 | Interní `MoveViewToDesktop` na okno cizího procesu, tam i zpět | **[OVĚŘENO]** |
| 6 | `SetWinEventHook` (out-of-context): maximalizace, obnovení, minimalizace, zavření cizího okna | **[OVĚŘENO]** – viz tabulka událostí níže |
| 7 | Okno spuštěné rovnou maximalizované → `IsZoomed = true` | **[OVĚŘENO]** |
| 8 | Okno bez rámečku přes celý monitor → `IsZoomed = false`, geometrická detekce fullscreenu = true | **[OVĚŘENO]** |
| 9 | Počítání „skutečných“ oken na ploše přes veřejné `GetWindowDesktopId` | **[OVĚŘENO]** (1 po přesunu, 0 po návratu) |
| 10 | Seznam ploch, pořadí a aktuální plocha z registru | **[OVĚŘENO]** |
| 11 | Záložní přesun okna bez interního API: skrýt → přepnout plochu → zobrazit | **[OVĚŘENO] u Win32 (charmap.exe)**, **selhalo u WinForms okna** (zůstalo skryté) |
| 12 | Celý scénář se 2 okny (viz kap. 6.4) | **[OVĚŘENO]** |
| 13 | Klávesové zkratky Win+Ctrl+D / Win+Ctrl+F4 / Win+Ctrl+←/→ přes `SendInput` | **[NEOVĚŘENO]** (záměrně, kvůli zásahu do relace) |
| 14 | Chování u oken spuštěných jako správce (elevated) | **[NEOVĚŘENO]** |
| 15 | Jiné verze Windows než 24H2 | **[NEOVĚŘENO]** |

### Pozorované sekvence událostí (test 6)
| Akce | Události (idObject = OBJID_WINDOW) |
|---|---|
| maximalizace | `EVENT_OBJECT_LOCATIONCHANGE` → stav maximalizováno |
| obnovení | `EVENT_OBJECT_LOCATIONCHANGE` → stav normální |
| minimalizace | `LOCATIONCHANGE` (iconic) + `EVENT_SYSTEM_MINIMIZESTART` |
| zavření | `EVENT_OBJECT_DESTROY` |

### Implementační poznámky z testů
- V .NET (Core) **nelze marshalovat `InterfaceIsIInspectable`** → `IApplicationView` deklarovat jako `IUnknown` (metody se nevolají, jen se předává ukazatel). **[OVĚŘENO]**
- `GetWindowDesktopId` vrací `TYPE_E_ELEMENTNOTFOUND (0x8002802B)` pro skryté okno a krátce po znovuzobrazení → nutný retry/ošetření. **[OVĚŘENO]**
- Okno spuštěné se `SW_HIDE`/`SW_MINIMIZE` ve STARTUPINFO převezme tento stav (artefakt testu, ne aplikace).

---

## 3. Technologie

| Volba | Rozhodnutí | Důvod |
|---|---|---|
| Jazyk / runtime | **C# / .NET 8 (LTS)**, `net8.0-windows` | Dobré COM interop, rychlý vývoj, WinForms pro tray |
| UI | WinForms `NotifyIcon` + malé okno nastavení | Minimum závislostí |
| Distribuce | `dotnet publish -r win-x64 --self-contained -p:PublishSingleFile=true` (+ win-arm64) | Jeden .exe, bez instalace .NET |
| NativeAOT | **Ne** ve v1 | Vyžaduje `ComWrappers` místo `[ComImport]`; zvážit později (menší .exe, rychlejší start) |
| Práva | Běží **bez práv správce** | Viz riziko R6 |
| Autostart | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` | Bez admin práv |
| Podpis | Authenticode (OV/EV nebo Azure Trusted Signing) | SmartScreen, antiviry |

Alternativy pro revizi: Rust (`windows` crate, menší binárka, lze převzít logiku z `Ciantic/VirtualDesktopAccessor`), C++/WinRT. C# zvolen kvůli rychlosti prototypu; architektura je přenositelná.

---

## 4. Architektura

```
┌──────────────────────────── AutoSpaces.exe (STA hlavní vlákno + message loop) ───────────────────────────┐
│                                                                                                         │
│  WindowWatcher ──(surové události)──► EventQueue ──(debounce 150 ms)──► SpaceOrchestrator (stavový stroj)│
│   SetWinEventHook                                                        │                              │
│   WindowClassifier                                                       ▼                              │
│                                                              IDesktopBackend (rozhraní)                 │
│                                                    ┌───────────────┴───────────────┐                    │
│                                          InternalComBackend                 FallbackBackend             │
│                                     (varianty z variants.json)     (registr + veřejné API + SendInput)  │
│                                                                                                         │
│  StateStore (state.json)   Settings (settings.json)   TrayUi   Logger   ExplorerRestartWatcher          │
└─────────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

### 4.1 Struktura řešení
```
AutoSpaces.sln
  src/AutoSpaces.App/          WinForms tray, start, DI, message loop
  src/AutoSpaces.Core/         SpaceOrchestrator, pravidla, model (bez Win32 → testovatelné)
  src/AutoSpaces.Windowing/    WindowWatcher, WindowClassifier, Win32 P/Invoke
  src/AutoSpaces.Desktops/     IDesktopBackend, InternalComBackend, FallbackBackend, RegistryDesktopReader
  src/AutoSpaces.App/variants.json   popis variant interního API
  tests/AutoSpaces.Core.Tests/       unit testy stavového stroje s fake backendem
  tests/integration/                 skripty z fáze ověření, rozšířené
```

### 4.2 `IDesktopBackend`
```csharp
interface IDesktopBackend {
    string Name { get; }                       // "internal:24H2", "fallback"
    IReadOnlyList<Guid> GetDesktops();         // v pořadí
    Guid GetCurrent();
    Guid Create();                             // vždy na konci; NEpřepíná (fallback přepíná – viz 5.3)
    void Remove(Guid desk, Guid fallback);
    void Switch(Guid desk);
    bool TryMoveWindow(IntPtr hwnd, Guid desk); // false = okno nelze přesunout
    Guid? GetWindowDesktop(IntPtr hwnd);       // veřejné API, null při chybě/skrytém okně
    bool CreatesAndSwitches { get; }           // true u fallbacku
}
```

---

## 5. Vrstvy nezávislosti na verzi Windows

Klíčový fakt: veřejné API Windows **neumí vytvořit plochu ani přesunout cizí okno** (test 4). Interní API to umí, ale jeho **IID a pořadí metod (vtable) se mění** mezi feature updaty. Za běhu je nelze zjistit – v `HKCR\Interface` ani v `actxprxy.dll`/`twinui*.dll` nejsou názvy rozhraní **[OVĚŘENO]**. Proto tři úrovně:

### 5.1 Úroveň 1 – `InternalComBackend` (plná funkce)
1. Vytvořit `ImmersiveShell` (CLSID `C2F03A33-21F5-47FA-B4BB-156362A2F239`) → `IServiceProvider` (`6D5140C1-7436-11CE-8034-00AA006009FA`).
2. Pro každou variantu z `variants.json` zavolat `QueryService(SID C5E0CDCA-7B6E-41B2-9FC4-D93975CC467B, IID varianty)`. **Vybrat podle odezvy, ne podle čísla buildu** (funguje i na Insider buildech sdílejících rozhraní).
3. **Sebetest před použitím** (ochrana proti stejnému IID s jiným vtable): `GetCount()` musí odpovídat počtu ploch v registru a `GetCurrentDesktop().GetId()` aktuální ploše z registru. Při nesouladu variantu zamítnout.
4. Volání metod přes **indexy slotů ve vtable** a `delegate* unmanaged[Stdcall]` (C# function pointers) → nová verze Windows = úprava `variants.json`, **bez rekompilace**.

`variants.json` – návrh schématu:
```json
{
  "variants": [
    {
      "name": "win11-24h2",
      "iidManagerInternal": "53F5CA0B-158F-4124-900C-057158060B27",
      "iidDesktop": "3F07F4BE-B107-441A-AF0F-39D82529072C",
      "monitorParam": false,
      "slots": { "GetCount": 3, "MoveViewToDesktop": 4, "GetCurrentDesktop": 6, "GetDesktops": 7,
                 "SwitchDesktop": 9, "CreateDesktop": 11, "RemoveDesktop": 13 },
      "desktopSlots": { "GetId": 4 }
    }
  ]
}
```
(Sloty počítány včetně 3 metod `IUnknown`. Pro 24H2 odpovídají pořadí ověřenému v testech; ostatní varianty doplnit a ověřit – viz tabulka.)

| Varianta | IVirtualDesktopManagerInternal | IVirtualDesktop | Poznámka | Stav |
|---|---|---|---|---|
| Windows 10 (1809–22H2) | `F31574D6-B682-4CDC-BD56-1827860ABEC6` | `FF72FFDD-BE7E-43FC-9C03-AD81681E88E4` | bez `MoveDesktop`, bez názvů ploch | [PŘEVZATO] |
| Windows 11 21H2 (22000) | `B2F925B9-5A0F-4D2E-9F4D-2B1507593C10` | `536D3495-B208-4CC9-AE26-DE8111275BF8` | metody s parametrem `HWND/HMONITOR` | [PŘEVZATO] |
| Windows 11 22H2 rané buildy | `B2F925B9-…` (stejné IID!) | `536D3495-…` | **jiný vtable při stejném IID** → nutný sebetest | [PŘEVZATO] |
| Windows 11 22H2/23H2 (22621.2215+) | `A3175F2D-239C-4BD2-8AA0-EEBA8B0B138E` | `3F07F4BE-B107-441A-AF0F-39D82529072C` | | [PŘEVZATO] |
| Windows 11 24H2+ (26100+) | `53F5CA0B-158F-4124-900C-057158060B27` | `3F07F4BE-B107-441A-AF0F-39D82529072C` | | **[OVĚŘENO]** |
| společné | `IApplicationViewCollection` `1841C6D7-4F9D-42C0-AF41-8747538F10E5`, `IApplicationView` `372E1D3B-38D3-42E4-A15B-8AB2B178F513` | | stabilní napříč verzemi | [OVĚŘENO na 24H2] |

Zdroje k převzetí/kontrole layoutů: `MScholtes/VirtualDesktop`, `Ciantic/VirtualDesktopAccessor`, `Slion/VirtualDesktop`. **Před převzetím kódu ověřit licence.**

### 5.2 Úroveň 2 – `FallbackBackend` (jen veřejné prostředky)
| Operace | Realizace | Stav |
|---|---|---|
| seznam/pořadí ploch | registr `HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\VirtualDesktops\VirtualDesktopIDs` (pole 16B GUID) | [OVĚŘENO] |
| aktuální plocha | tamtéž `CurrentVirtualDesktop`; na Win10 v `…\Explorer\SessionInfo\<id>\VirtualDesktops` | [OVĚŘENO Win11] / [PŘEVZATO Win10] |
| plocha okna | veřejné `IVirtualDesktopManager::GetWindowDesktopId` | [OVĚŘENO] |
| vytvoření plochy | `SendInput` Win+Ctrl+D (vytvoří na konci **a přepne**) | [NEOVĚŘENO] |
| přepnutí na plochu X | rozdíl indexů z registru → Win+Ctrl+←/→ × k, ověřit registrem | [NEOVĚŘENO] |
| smazání plochy | přepnout na ni → Win+Ctrl+F4 → přepnout na cíl | [NEOVĚŘENO] |
| přesun okna | `ShowWindow(SW_HIDE)` → přepnout → `ShowWindow(SW_SHOWNA)` → obnovit max. stav | [OVĚŘENO Win32], [SELHALO WinForms] |
| změny ploch | `RegNotifyChangeKeyValue` na klíči `VirtualDesktops` | [NEOVĚŘENO] |

Omezení fallbacku: viditelné přepínání (víc animací), některé aplikace hide/show nepřežijí (seznam nekompatibilních → okno nepřesouvat, jen nechat být), `SendInput` nefunguje, když je v popředí okno s vyšší integritou nebo zabezpečená plocha UAC.

### 5.3 Úroveň 3 – bez podpory
Pokud selže úroveň 1 i 2 (např. Windows bez virtuálních ploch, Windows Server bez shellu): notifikace „Nepodporovaná verze“, aplikace zůstane v trayi neaktivní, loguje diagnostiku (build, výsledky QueryService) pro doplnění `variants.json`.

Volba úrovně: automaticky při startu a po restartu Exploreru; v nastavení lze vynutit `auto | internal | fallback`.

---

## 6. Logika aplikace

### 6.1 Sledování oken (`WindowWatcher`)
- `SetWinEventHook(..., WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS)` pro:
  `EVENT_OBJECT_LOCATIONCHANGE (0x800B)`, `EVENT_OBJECT_SHOW (0x8002)`, `EVENT_OBJECT_HIDE (0x8003)`, `EVENT_OBJECT_DESTROY (0x8001)`, `EVENT_SYSTEM_MINIMIZESTART (0x16)`, `EVENT_SYSTEM_MOVESIZESTART/END (0x0A/0x0B)`.
- Filtr v callbacku: `idObject == OBJID_WINDOW (0)`, `idChild == 0`, top-level (`GetAncestor(GA_ROOT) == hwnd`).
- **V callbacku nevolat COM ani nic blokujícího** – jen zapsat do fronty (re-entrance, hook se volá během pumpování zpráv).
- **Debounce 150 ms na hwnd**, pak vyhodnotit *aktuální* stav okna (ne stav z události).
- Stav okna: `Normal | Maximized | Fullscreen | Minimized | Hidden | Destroyed`. Reagovat jen na **změnu** stavu oproti poslední známé hodnotě (přepnutí plochy samo generuje LOCATIONCHANGE).
- `DESTROY` – hwnd už nejde dotazovat → párovat přes slovník sledovaných oken.
- Při tažení myší (`MOVESIZESTART` … `END`, např. stažení maximalizovaného okna dolů) akci odložit do `MOVESIZEEND`.

### 6.2 Klasifikace oken (`WindowClassifier`)
Kandidát na vlastní plochu, pokud:
- `IsWindowVisible`, má titulek, není `WS_EX_TOOLWINDOW`, není owned (nebo má `WS_EX_APPWINDOW`), není `WS_CHILD`;
- není shell/systémové okno (třídy `Progman`, `WorkerW`, `Shell_TrayWnd`, `Shell_SecondaryTrayWnd`, `Windows.UI.Core.CoreWindow`, okna Task View / Alt+Tab / Start …);
- není připnuté „na všech plochách“ (interní `IVirtualDesktopPinnedApps` nebo heuristika: `GetWindowDesktopId` = `Guid.Empty`);
- proces není v seznamu vyloučených (settings), není to AutoSpaces;
- UWP: pracovat s `ApplicationFrameWindow` (top-level), ne s `CoreWindow`.

Fullscreen = nemá `WS_CAPTION` **a** obdélník okna pokrývá celý monitor (`MonitorFromWindow` + `GetMonitorInfo().rcMonitor`). **[OVĚŘENO]** Doplňkově lze použít `SHQueryUserNotificationState` (QUNS_RUNNING_D3D_FULL_SCREEN) pro detekci exkluzivních her → defaultně ignorovat.

Pro počítání „oken na ploše“ (rozhodnutí o smazání) použít stejný filtr kromě podmínky na maximalizaci.

### 6.3 Stavový stroj (`SpaceOrchestrator`)
Model:
```
ManagedSpace { Guid DesktopId; IntPtr Hwnd; int Pid; Guid OriginDesktopId; DateTime Created; }
spaces: Dictionary<Guid DesktopId, ManagedSpace>
byWindow: Dictionary<IntPtr, Guid DesktopId>
```
Přechody okna W:

| Událost | Podmínka | Akce |
|---|---|---|
| → Maximized / Fullscreen | W není spravované, je kandidát | `origin = GetWindowDesktop(W) ?? GetCurrent()`; `D = Create()`; `TryMoveWindow(W, D)`; pokud uživatel byl na `origin` (tj. akce vznikla tam, kde se dívá) → `Switch(D)`; zapsat ManagedSpace; uložit state |
| → Maximized / Fullscreen | W už je spravované | nic |
| → Normal / Minimized / Hidden | W je spravované | `target = ResolveOrigin(space.Origin)`; `TryMoveWindow(W, target)`; `ExitSpace(space, target)` |
| → Destroyed | W je spravované | `ExitSpace(space, ResolveOrigin(space.Origin))` |
| W ručně přesunuto na jinou plochu | zjištěno při kontrole | přestat spravovat W; plochu nechat, pokud není prázdná |

```
ExitSpace(space, target):
    remove mapping W
    viewing = GetCurrent() == space.DesktopId
    if CountWindowsOn(space.DesktopId) == 0:
        if viewing: Switch(target)
        Remove(space.DesktopId, target)
        delete space
    else:
        space se mění na „běžnou“ plochu (už ji nespravujeme, nemazat)
        if viewing && settings.switchBackWhenOthersRemain: Switch(target)

ResolveOrigin(o):
    while o neexistuje v GetDesktops():
        o = spaces[o]?.OriginDesktopId ?? první plocha
    return o
```

Poznámky:
- Ukládat **GUID ploch, nikdy indexy** – indexy se posouvají při mazání. **[OVĚŘENO v testu 12, krok 4]**
- Minimalizace z maximalizovaného stavu: Windows při obnovení z hlavního panelu vrátí okno **maximalizované** → znovu vznikne plocha (žádoucí, macOS-like).
- Okna otevřená z fullscreen aplikace (dialogy) se objeví na pomocné ploše; owned okna se nepočítají, při smazání plochy je Windows přesune na `fallback` plochu.
- Po `Switch` aktivovat okno (`SetForegroundWindow`); pokud Windows aktivaci odepře, použít `AllowSetForegroundWindow`/trik s `AttachThreadInput` nebo interní `SwitchDesktopAndMoveForegroundView` (je-li ve variantě).

### 6.4 Referenční scénář (akceptační test, [OVĚŘENO] v `testy/run4.ps1`)
Start: 2 plochy, uživatel na ploše 1.
1. Maximalizace okna A → 3 plochy, uživatel na ploše 3, A na ploše 3.
2. Okno B otevřené z plochy 3 → B je na ploše 3.
3. Maximalizace B → 4 plochy, uživatel na ploše 4.
4. Zavření A na pozadí → 3 plochy, uživatel zůstává u B (B se posune na pozici 3), **bez přepnutí**.
5. Obnovení B → B na ploše 1 (původní plocha zanikla → ResolveOrigin), 2 plochy, uživatel na ploše 1.
6. B znovu maximalizováno, na jeho ploše otevřeno okno C → 2 okna na ploše.
7. Minimalizace B → plocha s C zůstává, uživatel přepnut na plochu 1.

### 6.5 Robustnost
- **Restart Exploreru**: `RegisterWindowMessage("TaskbarCreated")` → znovu inicializovat backend (COM objekty jsou neplatné), znovu zvolit variantu, zrekonciliovat stav.
- **Uživatel ručně smazal pomocnou plochu**: detekovat při každém vyhodnocení (GUID chybí v `GetDesktops`) nebo přes `RegNotifyChangeKeyValue`/`IVirtualDesktopNotification` → odstranit ze stavu.
- **Pád / ukončení aplikace**: `state.json` v `%LOCALAPPDATA%\AutoSpaces\`. Při startu: pro každý uložený ManagedSpace, který existuje a je prázdný → smazat; neprázdný → převzít, pokud okno (hwnd+pid) stále žije, jinak zapomenout.
- **Ukončení aplikace z tray**: volba „vrátit okna a uklidit plochy“ vs. „nechat jak je“.
- **Výjimky z COM**: každé volání obalit; 3 chyby za sebou → přepnout na fallback a notifikovat.
- **Jedna instance**: pojmenovaný mutex.

### 6.6 Nastavení (`settings.json`)
| Klíč | Default | Popis |
|---|---|---|
| `enabled` | true | globální vypínač (i z tray, příp. klávesová zkratka) |
| `triggerOnMaximize` | true | |
| `triggerOnFullscreen` | true | borderless fullscreen |
| `ignoreExclusiveFullscreenGames` | true | |
| `excludedProcesses` | `[]` | např. `["vlc.exe"]` |
| `switchBackWhenOthersRemain` | true | krok 7 scénáře |
| `nameDesktopsAfterApp` | true | jen pokud varianta umí `SetDesktopName` (Win11) |
| `backend` | `auto` | `auto/internal/fallback` |
| `debounceMs` | 150 | |
| `logLevel` | `info` | log do `%LOCALAPPDATA%\AutoSpaces\logs` |

---

## 7. Fáze a akceptační kritéria

| Fáze | Obsah | Hotovo, když | Odhad |
|---|---|---|---|
| 0 | Repo, solution, CI (build + unit testy), logování | `dotnet build` a testy v CI zelené | 0,5 d |
| 1 | `InternalComBackend`: vtable volání, `variants.json`, výběr varianty + sebetest, `RegistryDesktopReader` | integrační test vytvoří/přesune/smaže plochu na 24H2; nesprávná varianta je zamítnuta sebetestem | 1–1,5 d |
| 2 | `WindowWatcher` + `WindowClassifier` + debounce | log přesně jedné změny stavu na akci pro: Notepad, Edge/Chrome, Explorer, UWP (Kalkulačka), WinForms, F11 v prohlížeči | 1 d |
| 3 | `SpaceOrchestrator` + `StateStore`, unit testy s fake backendem | scénář 6.4 prochází v unit testu i naživo; úklid po zabití procesu funguje | 1–1,5 d |
| 4 | `FallbackBackend` | scénář 6.4 prochází při vynuceném `backend=fallback` (s výjimkou aplikací na seznamu nekompatibilních) | 1–1,5 d |
| 5 | Tray UI, nastavení, autostart, jedna instance, restart Exploreru | ruční QA checklist (kap. 8) | 1 d |
| 6 | Testy na verzích Windows, doplnění variant, podpis, release | matice kap. 8 vyplněná | 1–2 d |

**Celkem cca 6,5–9,5 dne.** Fáze 4 lze odložit za v1, pokud cílíme jen na známé verze.

---

## 8. Testování

### Unit (Core)
- Fake `IDesktopBackend` + skriptované sekvence stavů oken; scénář 6.4; ResolveOrigin s řetězem zaniklých ploch; ručně smazaná plocha; duplicitní/rychlé události.

### Integrační / ruční matice
| Windows | Backend | Scénář 6.4 | Restart Exploreru | Pád aplikace + úklid |
|---|---|---|---|---|
| 10 22H2 | internal / fallback | | | |
| 11 21H2 | internal / fallback | | | |
| 11 22H2 (raný i pozdní build) | internal / fallback | | | |
| 11 23H2 | internal / fallback | | | |
| 11 24H2 | internal / fallback | ✅ (internal, prototyp) | | |
| 11 Insider (Canary/Dev) | auto | | | |

Prostředí: Hyper-V VM s Windows evaluation obrazy, snapshot před každým během.

### Aplikace k testu
Notepad, Edge, Chrome (F11), Firefox, Průzkumník, VS Code (Electron), Kalkulačka (UWP), Office, WinForms/WPF app, hra v borderless i exclusive fullscreenu, aplikace spuštěná jako správce, video přehrávač (fullscreen dvojklikem).

### QA checklist chování
- Maximalizace tlačítkem, dvojklikem na titulek, Win+↑, přetažením k hornímu okraji.
- Obnovení tlačítkem, Win+↓, stažením titulku dolů (akce až po puštění myši).
- Minimalizace a obnovení z hlavního panelu (→ nová plocha znovu).
- Zavření Alt+F4, křížkem, ukončením procesu.
- Aplikace, která se při zavření skryje do trayi.
- Dva monitory: maximalizace na sekundárním monitoru.

---

## 9. Rizika

| ID | Riziko | Dopad | Mitigace |
|---|---|---|---|
| R1 | Feature update Windows změní interní API | Úroveň 1 přestane fungovat | Výběr podle odezvy + sebetest, `variants.json` bez rekompilace, automatický přechod na fallback, diagnostický log |
| R2 | Stejné IID, jiný vtable (22H2 raný vs. pozdní) | Pád Exploreru / chybné volání | Sebetest proti registru před prvním zápisovým voláním; volat nejdřív jen čtecí metody |
| R3 | Více monitorů: plochy jsou ve Windows společné pro všechny monitory | Maximalizace na jednom monitoru „schová“ okna na ostatních | Zdokumentovat; volba „jen primární monitor“ nebo vypnutí pro sekundární |
| R4 | Fallback: aplikace nepřežije hide/show | Okno zůstane skryté | Po `SW_SHOW` ověřit `IsWindowVisible`; pokud ne, okamžitě vrátit a zařadit proces do seznamu nekompatibilních |
| R5 | `SetForegroundWindow` odepřen po přepnutí | Okno není aktivní | Viz 6.3 |
| R6 | Elevated okna: UIPI blokuje `ShowWindow`/`SendInput` z neelevated procesu | Fallback nefunguje pro admin okna | Ověřit; v internal režimu přesun dělá Explorer (pravděpodobně funguje); případně volitelný elevated režim přes Plánovač úloh |
| R7 | Antivirus/SmartScreen (hooky + SendInput + nepodepsaný exe) | Blokace | Podpis, žádná DLL injekce (out-of-context hooky) |
| R8 | Aplikace, které se samy maximalizují při startu (např. prohlížeč) | Neočekávané nové plochy | Je to požadované chování; volitelně vyloučení procesu |
| R9 | Licence převzatého kódu (layouty rozhraní) | Právní | Ověřit licence; definice rozhraní psát vlastní |

---

## 10. Otevřené otázky (pro revizi)

1. Je správné **přepínat zpět na původní plochu i v případě, že na pomocné ploše zůstala jiná okna** (krok 7)? Nyní default ano.
2. Má se nová plocha přepnout **i když okno bylo maximalizováno na pozadí** (jiná plocha než aktuální, např. aplikace se sama maximalizuje)? Návrh: ne – plochu vytvořit, ale nepřepínat.
3. Má se okno spuštěné maximalizované při **startu Windows / obnovení relace** chovat stejně? Návrh: ignorovat okna existující při startu AutoSpaces (volba).
4. Chování na **více monitorech** (R3) – vypnout pro sekundární monitory?
5. Pojmenovávat plochy podle aplikace (jen Win11)?
6. Je C#/.NET správná volba, nebo jít rovnou do Rustu (menší binárka, bez runtime)?
7. Podporovat fallback ve v1, nebo jen úroveň 1 + diagnostiku?

## 11. Checklist pro revidující model
- [ ] Zkontrolovat sloty vtable a IID všech variant v kap. 5.1 proti aktuálním zdrojům.
- [ ] Posoudit, zda sebetest (5.1 bod 3) dostatečně chrání před R2 – navrhnout lepší (např. ověření délky vtable, kontrola modulu, ze kterého pochází vtable).
- [ ] Ověřit správnost sady WinEvent událostí a debounce strategie (6.1), zejména pro UWP a Electron.
- [ ] Posoudit stavový stroj (6.3) na race conditions: událost během vlastního přesunu/přepínání, rychlé max/restore, zavření během přepínání.
- [ ] Zvážit, zda nepoužít `IVirtualDesktopNotification` místo polling/registry.
- [ ] Posoudit fallback (5.2) – existuje lepší veřejný způsob přesunu cizího okna?
- [ ] Zhodnotit odhady a pořadí fází.
