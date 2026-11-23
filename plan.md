# AutoSpaces – plán stavby aplikace

> Pracovní název: **AutoSpaces**. Dokument je určen k revizi a úpravám. Každé tvrzení je označeno:
> **[OVĚŘENO]** = prakticky vyzkoušeno na referenčním stroji, **[PŘEVZATO]** = z komunitních zdrojů, neověřeno, **[NEOVĚŘENO]** = předpoklad, nutno ověřit.
>
> Referenční stroj: Windows 11 Pro, build 26340.9882 (DisplayVersion 26H2), `twinui.pcshell.dll` 10.0.26100.9949 (= rodina 24H2), PowerShell 7.6, bez .NET SDK.

>
> **Revize 2** (původní verze: `plan-v1.md`). Změny jsou shrnuté v kap. 0 a zapracované do textu.

---

## 0. Revize 2 – co se změnilo a proč

Nové cíle: **rychlé a plynulé přepínání**, **minimální paměť**, **samoučící seznam aplikací, které nejdou přesunout**.

| # | Změna | Důvod | Kde |
|---|---|---|---|
| Z1 | **Rust místo C#/.NET + WinForms**, bez UI knihovny (čisté Win32) | paměť: odhad 2–4 MB místo 40–60 MB; žádný runtime, exe < 1 MB | kap. 3, 4.1 |
| Z2 | **Zrušen pevný debounce 150 ms** pro maximalizaci; reaguje se hned na první událost se změnou stavu | 150 ms bylo 25× víc než celá operace (naměřeno ~6 ms) | 6.1 |
| Z3 | **Výchozí přepnutí je okamžité** (`SwitchDesktop`), animované jen volitelně | naměřeno: okamžité 5,6 ms, animované 88 + 354 ms | 6.3, 6.6 |
| Z4 | **Výstup jedním voláním `RemoveDesktop(plocha, cíl)`** – Windows samy přesunou okno i přepnou | naměřeno 5 ms, méně mezistavů = žádné probliknutí; ověřeno, že okno i pohled skončí na cíli | 6.3 |
| Z5 | **Kontrola přesunutelnosti PŘED vytvořením plochy** (`GetViewForHwnd` + `CanViewMoveDesktops`, ~1,5 ms) | nepřesunutelné okno nezpůsobí žádné bliknutí ani osiřelou plochu | 6.3 |
| Z6 | **Vstup jako transakce s ověřením a návratem zpět** | podklad pro učení; po selhání nezůstane nepořádek | 6.3 |
| Z7 | **Naučený seznam** `learned.json` (proces + třída okna, důvod, počet selhání) | požadavek uživatele | nová kap. 6.7 |
| Z8 | **Žádná evidence všech oken**; drží se jen spravovaná okna a malá množina „odmítnutých“ | paměť a CPU | 6.1 |
| Z9 | Varianta rozhraní se vybírá podle **IID a zároveň rozsahu buildů** | sebetest sám nestačí: volání špatného slotu může shodit vlastní proces | 5.1 |
| Z10 | Doplněna rizika R10 (četnost událostí), R11 (chybné naučení) a výkonnostní cíle | | 7, 9 |

### Výkonnostní cíle (akceptační)
| Metrika | Cíl | Podklad |
|---|---|---|
| Doba od události maximalizace po přepnutí plochy | ≤ 30 ms | operace samy ~6 ms **[OVĚŘENO]** |
| Doba od zmenšení/zavření po návrat na původní plochu | ≤ 30 ms | ~5 ms **[OVĚŘENO]** |
| Paměť v klidu (private working set) | ≤ 5 MB | odhad pro Rust **[NEOVĚŘENO]** |
| CPU v klidu | < 0,1 % | závisí na R10 **[NEOVĚŘENO]** |
| Velikost exe | < 1,5 MB | odhad **[NEOVĚŘENO]** |

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
| 16 | Rychlost volání (ms): `GetViewForHwnd` 0,9 · `CanViewMoveDesktops` 0,6 · `CreateDesktop` 0,35 · `MoveViewToDesktop` 1,1 · `SwitchDesktop` 5,6 · `GetWindowDesktopId` 2,6 · spočítání oken na ploše 3,9 | **[OVĚŘENO]** (`testy/run5.ps1`, jedno měření, řádově) |
| 17 | Celý vstup (Create + Move + Switch) **5,7 ms**; celý výstup (`RemoveDesktop`) **5,0 ms** | **[OVĚŘENO]** |
| 18 | `SwitchDesktopWithAnimation` (24H2) funguje: volání 88 ms + `WaitForAnimationToComplete` 354 ms | **[OVĚŘENO]** – tím je potvrzeno i pořadí metod až po konec rozhraní 24H2 (`FindDesktop` vrací shodu) |
| 19 | `RemoveDesktop(plocha, cíl)` na ploše, na kterou se dívám a kde je okno: okno se přesune na cíl a pohled se přepne na cíl | **[OVĚŘENO]** (vizuální plynulost neposouzena – měřen jen čas a výsledný stav) |
| 20 | `SwitchDesktopAndMoveForegroundView` (přesun aktivního okna a přepnutí jedním voláním) | **[NEOVĚŘENO]** – kandidát na ještě plynulejší vstup |

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
| Jazyk | **Rust** (stable, MSVC toolchain), crate `windows-sys` | Žádný runtime ani GC, nejmenší paměť; volání COM přes sloty vtable je v Rustu přirozené |
| UI | **Bez UI knihovny**: skryté okno pro zprávy, `Shell_NotifyIcon`, `TrackPopupMenu` | Tray ikona + kontextové menu stačí; nastavení se edituje jako JSON (menu „Otevřít nastavení“) |
| Závislosti | `windows-sys`, `serde` + `serde_json` (nebo `miniserde`), nic dalšího | Malá binárka, rychlý start |
| Vlákna | **Jedno vlákno** (STA + smyčka zpráv); hooky, COM i logika na něm | Žádné zámky, žádné přepínání kontextu, minimální paměť |
| Distribuce | `cargo build --release` s `opt-level="z"`, `lto=true`, `panic="abort"`, `strip=true`; x64 + arm64 | Jeden .exe, odhad < 1 MB |
| Práva | Běží **bez práv správce** | Viz riziko R6 |
| Autostart | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` | Bez admin práv |
| Podpis | Authenticode (OV/EV nebo Azure Trusted Signing) | SmartScreen, antiviry |

Potřebné k sestavení: `rustup` + Visual Studio Build Tools (linker MSVC, Windows SDK).

**Proč ne C#** (revize 2): samostatný .NET + WinForms znamená odhadem 40–60 MB paměti a exe v desítkách MB. C# s NativeAOT a bez WinForms by se dostal na odhadem 8–15 MB – přijatelná záložní volba, pokud by Rust byl překážkou; architektura (kap. 4–6) je na jazyku nezávislá. Odhady paměti jsou **[NEOVĚŘENO]**, změří se ve fázi 0.

---

## 4. Architektura

```
┌──────────────────────────── autospaces.exe (jedno vlákno STA + smyčka zpráv) ────────────────────────────┐
│                                                                                                         │
│  WindowWatcher ──(surové události)──► PostMessage ──(ihned, mimo hook)──► SpaceOrchestrator (stavový stroj)│
│   SetWinEventHook                                                        │                              │
│   WindowClassifier                                                       ▼                              │
│                                                              DesktopBackend (trait)                     │
│                                                    ┌───────────────┴───────────────┐                    │
│                                          InternalComBackend                 FallbackBackend             │
│                                     (varianty z variants.json)     (registr + veřejné API + SendInput)  │
│                                                                                                         │
│  state.json   settings.json   learned.json (naučený seznam)   Tray   Log   ExplorerRestartWatcher       │
└─────────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

### 4.1 Struktura projektu
```
autospaces/                (jeden crate)
  src/main.rs              start, jedna instance, smyčka zpráv, tray
  src/core/                orchestrator.rs (stavový stroj), learned.rs, model.rs  – bez Win32 → testovatelné
  src/windowing/           watcher.rs (hooky), classifier.rs
  src/desktops/            backend.rs (trait), internal.rs (vtable volání), fallback.rs, registry.rs
  src/store.rs             settings.json, state.json, learned.json
  variants.json            popis variant interního API (vložen do exe + přepsatelný souborem vedle exe)
  tests/                   unit testy stavového stroje s fake backendem
  testy/                   integrační skripty z fáze ověření
```

### 4.2 `DesktopBackend`
```rust
trait DesktopBackend {
    fn name(&self) -> &str;                              // "internal:win11-24h2", "fallback"
    fn desktops(&self) -> Result<Vec<GUID>>;             // v pořadí
    fn current(&self) -> Result<GUID>;
    fn can_move(&self, hwnd: HWND) -> Result<bool>;      // levná kontrola PŘED vytvořením plochy
    fn create(&self) -> Result<GUID>;                    // vždy na konci
    fn move_window(&self, hwnd: HWND, desk: GUID) -> Result<()>;
    fn switch(&self, desk: GUID, animated: bool) -> Result<()>;
    fn remove(&self, desk: GUID, fallback: GUID) -> Result<()>; // přesune zbylá okna na fallback a přepne, pokud se na plochu díváme
    fn window_desktop(&self, hwnd: HWND) -> Option<GUID>; // veřejné API
}
// Chyby rozlišovat: Transient (RPC nedostupné, Explorer se restartuje) × Permanent (HRESULT selhání operace).
```

---

## 5. Vrstvy nezávislosti na verzi Windows

Klíčový fakt: veřejné API Windows **neumí vytvořit plochu ani přesunout cizí okno** (test 4). Interní API to umí, ale jeho **IID a pořadí metod (vtable) se mění** mezi feature updaty. Za běhu je nelze zjistit – v `HKCR\Interface` ani v `actxprxy.dll`/`twinui*.dll` nejsou názvy rozhraní **[OVĚŘENO]**. Proto tři úrovně:

### 5.1 Úroveň 1 – `InternalComBackend` (plná funkce)
1. Vytvořit `ImmersiveShell` (CLSID `C2F03A33-21F5-47FA-B4BB-156362A2F239`) → `IServiceProvider` (`6D5140C1-7436-11CE-8034-00AA006009FA`).
2. Pro každou variantu z `variants.json` zavolat `QueryService(SID C5E0CDCA-7B6E-41B2-9FC4-D93975CC467B, IID varianty)`. **Vybrat podle odezvy, ne podle čísla buildu** (funguje i na Insider buildech sdílejících rozhraní).
3. **Rozsah buildů jako druhá podmínka** (revize 2): každá varianta má `minBuild`/`maxBuild` (z `RtlGetVersion`, příp. verze `twinui.pcshell.dll`). Když IID projde u více variant (stejné IID, jiný vtable), rozhodne build. Neznámý build + známé IID → zkusit nejnovější variantu s tím IID, ale jen přes sebetest.
4. **Sebetest před použitím**: `GetCount()` musí odpovídat počtu ploch v registru a `GetCurrentDesktop().GetId()` aktuální ploše z registru. Při nesouladu variantu zamítnout. Pozor: rozhraní žije v `explorer.exe`, voláme ho přes COM proxy – volání špatného slotu **může shodit náš proces i Průzkumníka** (při jedné dřívější zkoušce se to stalo), protože špatný slot je jiná metoda s jinými parametry. Proto sebetest pouštět jen čtecími metodami; pád během sebetestu = při dalším startu tuto variantu přeskočit (skutečné provedení: soubor `selftest-guard.txt`, ne `state.json` – viz „Pořadí ochran“ níže).
5. Volání metod přes **indexy slotů ve vtable** (surové ukazatele na funkce) → nová verze Windows = úprava `variants.json`, **bez rekompilace**.
6. Volitelné sloty (pokud je varianta má): `SwitchDesktopWithAnimation`, `WaitForAnimationToComplete`, `SwitchDesktopAndMoveForegroundView`, `SetDesktopName`. Chybějící slot = funkce se tiše nepoužije.

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

**Skutečný stav (fáze 8, 2026-11-13):** `variants.json` má 6 variant. Převzaty jsou jen IID a čísla slotů (z definic rozhraní, žádný kód), každá varianta je doložena aspoň dvěma nezávislými zdroji. V JSON má `source` (zdroje a rozsah buildů) a `verified`. Chybějící `verified` = neověřená. Neověřená varianta musí mít uzavřený rozsah (`maxBuild`).

| Varianta (`name`) | Windows / buildy | IVirtualDesktopManagerInternal | IVirtualDesktop | Sloty Count/Cur/Desktops/Switch/Create/Remove/Find | Zdroje | Stav |
|---|---|---|---|---|---|---|
| `win11-24h2` | 11 24H2+ (26100–) | `53F5CA0B-…` | `3F07F4BE-…` | 3/6/7/9/11/13/14 (+ `SwitchDesktopAndMoveForegroundView` 10) | MScholtes 11-24H2.cs, Ciantic winvd 0.0.48+, pyvda OVER_26100 | **[OVĚŘENO]** 26340 |
| `win11-23h2-3085` | 11 22H2/23H2 od UBR 3085 (22621–22631) | `53F5CA0B-…` (**stejné IID jako 24H2**, chybí slot 10) | `3F07F4BE-…` | 3/6/7/9/10/12/13 | MScholtes v1.17 (`4dd34ce`), Ciantic `0276add` (winvd 0.0.46); pyvda má stejné pořadí, ale IID `4970BA3D-…` | neověřeno |
| `win11-22h2-2215` | 11 22H2/23H2 UBR 2215–3084 (22621–22631) | `A3175F2D-…` | `3F07F4BE-…` | 3/6/7/9/10/12/13 | MScholtes v1.16 `VirtualDesktop11-23H2.cs` (`546b673`), Ciantic `c918946` (winvd 0.0.40), pyvda OVER_22621 | neověřeno |
| `win11-22h2` | 11 22H2 před UBR 2215 (22621) | `B2F925B9-…` | `536D3495-…` | HMONITOR; 3/6/8/10/11/13/14 (navíc `GetAllCurrentDesktops` 7) | MScholtes v1.16 `VirtualDesktop11.cs`, pyvda OVER_22449 | neověřeno |
| `win11-21h2` | 11 21H2 (22000) | `B2F925B9-…` (**stejné IID jako raná 22H2**) | `536D3495-…` | HMONITOR; 3/6/7/9/10/12/13 | MScholtes v1.16 `VirtualDesktop11-21H2.cs`, pyvda OVER_21313 | neověřeno |
| `win10` | 10 1809–22H2 (17763–19045) | `F31574D6-…` | `FF72FFDD-…` | 3/6/7/9/10/11/12 | MScholtes `VirtualDesktop.cs` v1.21, pyvda (větev 9000) | neověřeno |
| společné | | `IApplicationViewCollection` `1841C6D7-…` (`GetViewForHwnd` 6) | | `IVirtualDesktop::GetId` 4 všude | všechny zdroje | [OVĚŘENO na 24H2] |

- **Nepřidáno:** `4970BA3D-FD4E-4647-BEA3-D89076EF4B9C` (jen pyvda, ostatní dva zdroje mají pro 22631.3085+ `53F5CA0B`; jen se zkouší QueryService pro diagnostiku), Insider 20231 (`094AFE11-…`, jen pyvda, nevydaný build), `IVirtualDesktopManagerInternal2` Win10 2004 (názvy ploch – aplikace je nepoužívá), Windows Server 2016/2022 a Win10 1607/1709/1803 (mimo rozsah zadání). `SwitchDesktopWithAnimation`/`WaitForAnimationToComplete` mají jen 24H2: u 22621/22631 je uvádí jen MScholtes (pyvda má jiný konec rozhraní), takže tam animace chybí a přepíná se bez ní.
- **Stejné IID, různý vtable:** `B2F925B9` (21H2 vs. raná 22H2) a `53F5CA0B` (23H2 od 3085 vs. 24H2). Rozliší je jen rozsah buildů – parser proto odmítne dvě varianty se stejným IID a překrývajícím se rozsahem. Rozlišení podle UBR není potřeba: v rámci 22621/22631 má každé rozložení jiné IID.
- **Zjištěno navíc:** Ciantic přidal `SwitchDesktopAndMoveForegroundView` až v 26100.2605, ale MScholtes ho má už ve verzi pro 24H2 Insider (6/2024) a pyvda pro celé 26100+; ověřená varianta proto zůstává od 26100.

**Pořadí ochran (`src/desktops/select.rs`), od nejbezpečnější:**
1. Při načtení `variants.json`: sloty jsou jedinečné a v pořadí společném všem známým rozložením (GetCount < Move < CanMove < GetCurrent < GetDesktops < Switch < Create < Remove < Find; `SwitchDesktopAndMoveForegroundView` mezi Switch a Create, ostatní volitelné za Find); neověřená varianta má `maxBuild`; stejné IID = disjunktní rozsahy. Chybný soubor vedle exe se ignoruje (použije se vložený).
2. Rozsah buildů – varianta mimo rozsah se vůbec neotevře.
3. Paměť pádu: před sebetestem **neověřené** varianty se zapíše `%LOCALAPPDATA%\AutoSpaces\selftest-guard.txt` (`pending <varianta> <build>`), po dokončení se smaže. Najde-li se při dalším startu, varianta se na tomto buildu už nezkouší (`crashed …`). Ověřené varianty soubor nezapisují.
4. `QueryService` s IID varianty (+ kolekce pohledů) – špatné IID vrátí E_NOINTERFACE, nic se nevolá.
5. Jen čtecí volání, každé až po shodě předchozího s registrem: `GetCount` → `GetCurrentDesktop` + QI na IID plochy varianty + `GetId` → `GetDesktops` (počet, pořadí a GUID shodné s `VirtualDesktopIDs`) → `FindDesktop(aktuální)` vrátí tutéž plochu. U variant s HMONITOR se předává NULL jako u ostatních volání. `FindDesktop` je nejvyšší povinný slot, takže měnící metody (Switch/Create/Remove) leží mezi ověřenými sloty.
6. Během sebetestu se výsledky zapisují do zvětšených vynulovaných bufferů (zápis za očekávanou velikost = špatné rozložení) a vrácený ukazatel se použije, jen pokud míří do potvrzené čitelné paměti a jeho vtable vede do spustitelného kódu načteného modulu (`VirtualQuery`); jinak se ani neuvolní.
- Žádná jiná metoda varianty se nevolá, dokud tohle neprojde. Neověřená varianta je v logu, v `--selftest` i v „O aplikaci“ označena jako neověřená.
- **Zbývající riziko:** volání špatného slotu probíhá přes COM proxy podle skutečné signatury. Kontroly 1–4 brání tomu, aby se k čtecímu volání vůbec došlo s cizím rozložením; zůstává případ, kdy zdroje shodně uvádějí špatné pořadí pro konkrétní build. Pak může čtecí slot ve skutečnosti být jiná metoda: s parametrem rozhraní obvykle spadne už náš proces při marshalování (krok 3 to příště přeskočí), ale nelze vyloučit volání měnící metody v explorer.exe (např. přepnutí plochy). Proto neověřené varianty zůstávají označené, dokud je někdo nevyzkouší na skutečném systému (`--selftest --select-only` je jen pro čtení).
- `autospaces.exe --selftest --select-only`: jen výběr varianty se sebetestem, výpis do konzole, žádný soubor, žádná plocha se nevytváří (plné `--selftest` dál vytváří plochu a přesouvá charmap). Na 26340: `win11-24h2` PASS za 13 ms, ostatní mimo rozsah.
- Velikost release exe 766 464 → 793 088 B (+26 KB: tabulka variant, sebetest, diagnostika). Paměť za běhu beze změny (výběr proběhne jednou při startu / po restartu Exploreru; v režimu bez podpory zůstane jen seznam diagnostických řádků).

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

**Rozhodnutí (fáze 8, 2026-11-13): úroveň 2 se nestaví.** Vyhodnocení po funkcích:
| Funkce | Veřejnými prostředky | Spolehlivě? |
|---|---|---|
| EnterSpace | skrýt okno → Win+Ctrl+D (vytvoří a přepne) → zobrazit | **ne**: hide/show u WinForms okna selhalo (okno zůstalo skryté = uživatel přijde o okno, test 11); UWP/`ApplicationFrameHost` okna cizí `ShowWindow` neskryje; maximalizaci často spouští sám uživatel přes Win+↑ nebo dvojklik, takže vložené Win+Ctrl+D se míchá s fyzicky drženými klávesami; UIPI blokuje `SendInput` i `ShowWindow`, je-li v popředí okno správce; skrytí mění pořadí na hlavním panelu a stav některých aplikací |
| ExitSpace | Win+Ctrl+F4 na pomocné ploše | **ne**: okna jdou na *sousední* plochu vlevo, ne na původní; vrácení = další hide/show se stejnými riziky; nejdřív by se na plochu muselo přepnout (2 animace) |
| kompakce | přepnout na prázdnou plochu + Win+Ctrl+F4 | ne: každé smazání = viditelné přepnutí, souboj s uživatelovým vstupem |
| profil rozložení, startovní rozmístění | N× přesun přes hide/show s přepínáním | ne: totéž ×N, přepínání pohledu, které profil právě nesmí dělat |
| čtení stavu (počet, aktuální plocha, plocha okna) | registr + `GetWindowDesktopId` | ano – ale samo nic nepřesune |

Protože ani základní EnterSpace/ExitSpace nejde udělat bez rizika ztráty okna nebo přesunu na špatnou plochu, polofunkční režim by škodil víc než pomohl. Místo něj je čistá úroveň 3.

### 5.3 Úroveň 3 – bez podpory
Pokud žádná varianta neprojde (neznámý build, Windows Server bez virtuálních ploch, chyba v rozhraní): aplikace běží v trayi, **nic nepřesouvá** a říká to:
- tooltip ikony „AutoSpaces – nepodporovaná verze Windows, nic se nepřesouvá“, jednou za běh oznámení (balón) s číslem sestavení;
- v nabídce ikony položka „Nahlásit problém…“ (12.12) vytvoří `%LOCALAPPDATA%\AutoSpaces\report.txt` (dřív `unsupported.txt`): verze aplikace, build.UBR, příčina, výsledek každé varianty (rozsah / HRESULT QueryService / důvod neúspěchu sebetestu) a seznam IID, které Explorer přijme (jen `QueryService` na IID všech variant + `4970BA3D`, `094AFE11`, `0F3A72B0` – žádné volání metod);
- „O aplikaci“: „Sestavení Windows N není podporováno, aplikace nic nepřesouvá.“ (cs/en).
Při startu se backend zkouší 20× po 1 s (Explorer může ještě startovat), teprve pak se ukáže tento režim; po restartu Exploreru se zkusí znovu.

**Volba úrovně:** skrytý klíč `"backend"` v `settings.json` (bez UI, zapisuje se jen když není `auto`): `auto` (výchozí, i pro neznámé hodnoty) = interní rozhraní; `none` = režim bez podpory i na podporovaném systému (pro vyzkoušení); `fallback` = totéž jako `none` (úroveň 2 není postavená, log to uvede). Při vynuceném vypnutí se neopakuje, tooltip zní „přesouvání vypnuto v settings.json“ a v hlášení je to uvedeno jako režim.

---

## 6. Logika aplikace

### 6.1 Sledování oken (`WindowWatcher`)
- `SetWinEventHook(..., WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS)` pro:
  `EVENT_OBJECT_LOCATIONCHANGE (0x800B)`, `EVENT_OBJECT_SHOW (0x8002)`, `EVENT_OBJECT_HIDE (0x8003)`, `EVENT_OBJECT_DESTROY (0x8001)`, `EVENT_SYSTEM_MINIMIZESTART (0x16)`, `EVENT_SYSTEM_MOVESIZESTART/END (0x0A/0x0B)`.
- Filtr v callbacku: `idObject == OBJID_WINDOW (0)`, `idChild == 0`, top-level (`GetAncestor(GA_ROOT) == hwnd`).
- **Callback musí být extrémně levný**: `EVENT_OBJECT_LOCATIONCHANGE` chodí i pro každý pohyb kurzoru a kurzoru textu. První řádek: `if idObject != OBJID_WINDOW || idChild != 0 { return }`. Teprve pak `IsZoomed`/styl okna. (riziko R10)
- **V callbacku nevolat COM** – jen `PostMessage` vlastnímu oknu (hwnd, typ události); zpracuje se v dalším kole smyčky zpráv, tj. prakticky ihned, ale mimo callback (re-entrance).
- **Bez pevného debounce** (revize 2). Maximalizace je diskrétní stav – `IsZoomed` se překlopí jednou, takže se reaguje **hned na první událost**. Krátké ustálení (50–80 ms, časovač) jen pro geometrickou detekci fullscreenu, kde mezistavy při změně velikosti mohou dát falešný poplach.
- **Zámek přechodu**: během vlastního vstupu/výstupu (a ~50 ms po něm) se události pro dané okno ignorují; po uvolnění se stav jednou přečte znovu.
- Stav okna: `Normal | Maximized | Fullscreen | Minimized | Hidden | Destroyed`.
- **Žádný slovník všech oken** (revize 2). Drží se jen: (a) spravovaná okna, (b) malá množina „odmítnutých“ hwnd (okno je maximalizované, ale rozhodli jsme se ho nespravovat – aby se nezkoušelo při každé události; maže se při `DESTROY` nebo když okno přestane být maximalizované). Pravidlo: *maximalizované/fullscreen a není v (a) ani (b)* → vstup.
- Název procesu se zjišťuje jen při rozhodování o vstupu (`QueryFullProcessImageNameW` s `PROCESS_QUERY_LIMITED_INFORMATION`, funguje i pro procesy správce) a ukládá se k hwnd v (a)/(b).
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
| → Maximized / Fullscreen | W není spravované ani odmítnuté, je kandidát, není v naučeném seznamu (6.7) | `EnterSpace(W)` |
| → Maximized / Fullscreen | W už je spravované | nic |
| → Normal / Minimized / Hidden | W je spravované | `ExitSpace(space, closed=false)` |
| → Destroyed | W je spravované | `ExitSpace(space, closed=true)` |
| W ručně přesunuto na jinou plochu | zjištěno při kontrole | přestat spravovat W; plochu nechat, pokud není prázdná |

```
EnterSpace(W):                                   # transakce; cíl ≤ 30 ms, naměřeno ~6 ms
    proc = název procesu(W); class = třída okna(W)
    if learned.blocked(proc, class): odmítnuté += W; return
    if !backend.can_move(W):                     # ~1,5 ms, PŘED vytvořením plochy → žádné bliknutí
        learned.fail(proc, class, CannotMove, hard); odmítnuté += W; return
    origin  = window_desktop(W) ?? current()
    viewing = current() == origin
    D = create()
    if move_window(W, D) selže  ||  window_desktop(W) != D:      # ověření výsledku
        remove(D, origin)                        # návrat do původního stavu
        learned.fail(proc, class, MoveFailed/MoveNotApplied, hard); odmítnuté += W; return
    if viewing: switch(D, settings.animated)     # výchozí: okamžitě (5,6 ms)
    aktivovat W; zapsat ManagedSpace{enteredAt = now}; uložit state

ExitSpace(space, closed):
    target  = ResolveOrigin(space.Origin)
    others  = CountWindowsOn(space.DesktopId) - (closed ? 0 : 1)   # jiná okna než W
    if now - space.enteredAt < 800 ms: learned.fail(proc, class, Flapping, soft)   # viz 6.7
    if others == 0:
        remove(space.DesktopId, target)          # JEDNO volání: přesune W, přepne pohled, smaže plochu (~5 ms)
    else:
        if !closed: move_window(W, target)
        if current() == space.DesktopId && settings.switchBackWhenOthersRemain: switch(target)
        # plocha zůstává jako běžná, už ji nespravujeme
    smazat záznam; uložit state

ResolveOrigin(o):
    while o neexistuje v GetDesktops():
        o = spaces[o]?.OriginDesktopId ?? první plocha
    return o
```

Poznámky k plynulosti (revize 2):
- **Okamžité přepnutí je výchozí.** Animace maximalizace samotného okna (cca 200 ms, dělá ji Windows) doběhne už na nové ploše; uživatel vidí jen to, že okolní okna zmizí. Animované přepnutí (posun ploch jako na macOS) stojí ~440 ms navíc a je volitelné (`switchAnimation`).
- Pořadí `move_window` → `switch` nechává ~1 ms okno, kdy je vidět původní plocha bez okna – pod hranicí jednoho snímku. Pokud by při testech bylo vidět probliknutí, vyzkoušet `SwitchDesktopAndMoveForegroundView` (jedno volání; test 20).
- Výstup přes `remove()` má nejméně mezistavů. Vizuální dojem (zda Windows při mazání aktuální plochy nepřehrají animaci) posoudit ve fázi 3 – měřen byl jen čas a výsledný stav.
- `state.json` zapisovat **po** dokončení přepnutí (mimo kritickou cestu), ne během něj.

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
| `backend` | `auto` | skrytý: `auto/none/fallback` (`fallback` = `none`, viz 5.2/5.3) |
| `switchAnimation` | false | true = `SwitchDesktopWithAnimation` (+~440 ms), jen kde ho varianta má |
| `fullscreenSettleMs` | 60 | ustálení jen pro geometrickou detekci fullscreenu |
| `learning` | true | samoučící seznam (6.7) |
| `learnedRetryDays` | 30 | po kolika dnech zkusit „měkce“ zablokovanou aplikaci znovu |
| `logLevel` | `warn` | log do `%LOCALAPPDATA%\AutoSpaces\log.txt`, rotace 1 MB; ve výchozím stavu se v klidu nic nezapisuje |

### 6.7 Naučený seznam nepřesunutelných aplikací (`learned.json`) – revize 2

Cíl: aplikaci, u které přesun nefunguje, **zkusit jednou (resp. dvakrát) a pak už ne** – ani po restartu aplikace nebo AutoSpaces.

**Klíč záznamu:** název procesu malými písmeny (`foo.exe`) + třída okna (`*` = všechna okna procesu). Třída je tam proto, aby jedno problémové okno (úvodní obrazovka, překryv) nezablokovalo celou aplikaci. V menu se zobrazují názvy procesů.

**Co se pozná a jak:**
| Důvod | Jak se pozná | Tvrdost | Zablokuje po |
|---|---|---|---|
| `NoView` | `GetViewForHwnd` selže (okno nemá „view“ v shellu) | tvrdé | 1 |
| `CannotMove` | `CanViewMoveDesktops` = false | tvrdé | 1 |
| `MoveFailed` | `MoveViewToDesktop` vrátí chybu (uloží se HRESULT) | tvrdé | 1 |
| `MoveNotApplied` | po přesunu `GetWindowDesktopId` ≠ cílová plocha (kontrola hned a po 100 ms) | tvrdé | 2 |
| `Flapping` | okno opustí maximalizovaný stav do 800 ms po vstupu (aplikace se „pere“, nebo je to krátkodobé okno) | měkké | 3× během 10 min |
| `ShortLived` | fullscreen okno zanikne/skryje se do 1,5 s (úvodní obrazovky, výřezy obrazovky, překryvy) | měkké | 2 |
| `HideShowFailed` | jen fallback: po `SW_SHOW` není okno viditelné | tvrdé | 1 (platí jen pro fallback) |
| `AccessDenied` | `E_ACCESSDENIED` (okno správce) | tvrdé | 1 |

Okna, která **vůbec nejdou maximalizovat** (nemají `WS_MAXIMIZEBOX`), událost maximalizace nikdy nevyvolají – není co se učit. Zjevné překryvy (`WS_EX_TOPMOST` + `WS_EX_LAYERED`/`WS_EX_TRANSPARENT`/`WS_EX_NOACTIVATE` přes celý monitor) vyřadí rovnou klasifikátor (6.2), bez učení.

**Co se nesmí počítat jako selhání** (jinak se seznam zanese – riziko R11):
- přechodné chyby COM/RPC (`0x800706BA`, `0x800706BE`, `0x80010108`, …) a cokoli během restartu Exploreru → reinicializace backendu, žádný záznam;
- selhání `create()` (to není vina aplikace);
- `Flapping`, pokud výstup vyvolal uživatel prokazatelně vstupem (stisk myši/klávesy v posledních 300 ms – `GetLastInputInfo`); cílem je chytit aplikace, které se zmenšují samy.

**Formát:**
```json
{ "version": 1, "entries": [
  { "process": "foo.exe", "class": "*", "reason": "MoveFailed", "hresult": "0x80070005",
    "strikes": 1, "blocked": true, "backend": "internal:win11-24h2",
    "exeStamp": "size:mtime", "first": "2026-11-11", "last": "2026-11-11" } ] }
```

**Kdy se záznam zruší / zkusí znovu:**
- změnil se `exeStamp` (aplikace byla aktualizována) → záznam se smaže;
- změnil se backend nebo jeho varianta (jiná verze Windows, přechod fallback ↔ internal) → záznamy z jiného backendu se nepoužijí;
- měkké důvody po `learnedRetryDays`; tvrdé nikdy samy;
- ručně: tray menu **Naučené výjimky ▸** (seznam procesů, klik = zapomenout) a **Zapomenout vše**.

**Implementace:** v paměti `HashMap<(proces, třída), Entry>` (desítky záznamů, zanedbatelná paměť), dotaz O(1) na začátku `EnterSpace`. Zápis na disk jen při změně. Odděleno od ručního `excludedProcesses` v nastavení (ten má vždy přednost a nikdy se nemaže).

**Revize 3 (konzervativní učení, po chybných záznamech u uživatele: Edge, Audacity, zamykací obrazovka …):**
- *Příčina:* `GetViewForHwnd` při události SHOW ještě nevrací view (Explorer ho vytvoří 7–200 ms po zobrazení, změřeno) a backend to hlásil jako `CanViewMoveDesktops = false`. Okno otevřené rovnou maximalizované (Audacity, Music Maker, nové okno Edge) tak napoprvé zablokovalo celou aplikaci; okna zamykací obrazovky a dotykové klávesnice view nemají nikdy.
- „Bez view“ (`TYPE_E_ELEMENTNOTFOUND`) není selhání: okno se zkouší znovu po 40 ms (nejvýš 25×), pak se jen nechá být – nikdy se nezapisuje.
- Systémová okna se vůbec nezpracovávají: procesy jen shellu (`textinputhost`, `shellexperiencehost`, `searchhost`, `startmenuexperiencehost`, `lockapp`, `logonui`) a třídy `Windows.UI.Core.CoreWindow`, `LockScreenBackstopFrame`, `Progman`, `WorkerW`, `Shell_TrayWnd` … Nikdy se neučí nic o `SYSTEM_HOSTS` (i `explorer.exe`, `applicationframehost.exe` – hostí mnoho aplikací; jejich okna se ale dál zpracovávají).
- Selhání se počítá jen při skutečném pokusu: okno je kandidát, není cloaked, je na právě zobrazené ploše, uživatel ho právě maximalizoval (vstup ≤ 1 s), je zobrazené déle než 2 s, neběží profil ploch ani restart Exploreru. Každé okno dá nejvýš jeden zásah, takže jedno problémové okno aplikaci samo nezablokuje.
- Prahy: `NoView`, `CannotMove`, `HideShowFailed` 3 zásahy (různá okna); `MoveFailed`, `AccessDenied`, `MoveNotApplied`, `ShortLived` 2; `Flapping` 3 během 10 min.
- Při načtení se zahodí záznamy, které by podle těchto pravidel nevznikly (systémové procesy a třídy, blokace s menším počtem zásahů než je práh) a soubor se přepíše.

---

## 7. Fáze a akceptační kritéria

| Fáze | Obsah | Hotovo, když | Odhad |
|---|---|---|---|
| 0 | Rust toolchain, kostra (skryté okno, smyčka zpráv, tray ikona, jedna instance), CI. **Změřit paměť a CPU prázdné kostry s nasazenými hooky.** | paměť ≤ 5 MB, CPU v klidu < 0,1 % při běžné práci s myší; jinak přehodnotit (R10) | 1 d |
| 1 | `internal.rs`: vtable volání, `variants.json`, výběr varianty (IID + build) + sebetest, `registry.rs` | integrační test vytvoří/přesune/smaže plochu na 24H2; nesprávná varianta je zamítnuta | 1,5–2 d |
| 2 | `watcher.rs` + `classifier.rs`, bez debounce, zámek přechodu | přesně jedna změna stavu na akci pro: Notepad, Edge/Chrome, Explorer, UWP (Kalkulačka), WinForms, F11 v prohlížeči | 1 d |
| 3 | `orchestrator.rs` (EnterSpace/ExitSpace jako v 6.3) + `state.json`, unit testy s fake backendem. **Změřit latenci událost → přepnutí; posoudit plynulost očima.** | scénář 6.4 prochází v unit testu i naživo; latence ≤ 30 ms; úklid po zabití procesu funguje | 1,5 d |
| 4 | `learned.rs` (6.7) + položky v tray menu | aplikace se selháním přesunu se podruhé nezkouší (ani po restartu); přechodná chyba při restartu Exploreru nic nezapíše; aktualizace exe záznam zruší | 1 d |
| 5 | `fallback.rs` | scénář 6.4 prochází při vynuceném `backend=fallback`; nekompatibilní aplikace se samy naučí (`HideShowFailed`) | 1–1,5 d |
| 6 | Nastavení, autostart, restart Exploreru, log | ruční QA checklist (kap. 8) | 0,5–1 d |
| 7 | Testy na verzích Windows, doplnění variant, podpis, release | matice kap. 8 vyplněná | 1–2 d |

**Celkem cca 8,5–11 dní** (o ~2 dny víc než revize 1: Rust místo C# a naučený seznam). Fázi 5 lze odložit za v1, pokud stačí známé verze Windows.

---

## 8. Testování

### Unit (Core)
- Fake `DesktopBackend` + skriptované sekvence stavů oken; scénář 6.4; ResolveOrigin s řetězem zaniklých ploch; ručně smazaná plocha; duplicitní/rychlé události.

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
| R10 | `EVENT_OBJECT_LOCATIONCHANGE` chodí pro každý pohyb kurzoru → časté probouzení procesu | CPU v klidu, výdrž baterie | Callback s okamžitým návratem (6.1); změřit ve fázi 0. Pokud nevyhoví: hook `LOCATIONCHANGE` nasazovat jen na proces okna v popředí (přehazovat při `EVENT_SYSTEM_FOREGROUND`) a pro ostatní spoléhat na `SHOW`/`MINIMIZESTART`/`DESTROY` |
| R11 | Naučený seznam se zanese chybnými záznamy (přechodná chyba, uživatel rychle kliká) | Aplikace přestane „fungovat“ bez zjevné příčiny | Pravidla v 6.7 (přechodné chyby se nepočítají, měkké důvody až po opakování a s expirací), viditelnost a mazání v tray menu, oznámení při prvním zablokování aplikace |

---

## 10. Otevřené otázky (pro revizi)

1. Je správné **přepínat zpět na původní plochu i v případě, že na pomocné ploše zůstala jiná okna** (krok 7)? Nyní default ano.
2. Má se nová plocha přepnout **i když okno bylo maximalizováno na pozadí** (jiná plocha než aktuální, např. aplikace se sama maximalizuje)? Návrh: ne – plochu vytvořit, ale nepřepínat.
3. Má se okno spuštěné maximalizované při **startu Windows / obnovení relace** chovat stejně? Návrh: ignorovat okna existující při startu AutoSpaces (volba).
4. Chování na **více monitorech** (R3) – vypnout pro sekundární monitory?
5. Pojmenovávat plochy podle aplikace (jen Win11)?
6. ~~C#/.NET, nebo Rust?~~ **Rozhodnuto v revizi 2: Rust** (kap. 3).
7. ~~Podporovat fallback ve v1, nebo jen úroveň 1 + diagnostiku?~~ **Rozhodnuto ve fázi 8: jen úroveň 1 + režim „nepodporováno“ s diagnostikou** (5.2).
8. Okamžité, nebo animované přepnutí jako výchozí? Revize 2 volí okamžité (rychlost); animované je blíž macOS, ale o ~440 ms pomalejší.
9. Má aplikace při prvním zablokování procesu zobrazit oznámení („Aplikaci X nelze přesouvat, přidána do výjimek“), nebo učit potichu?

## 11. Checklist pro revidující model
- [x] Zkontrolovat sloty vtable a IID všech variant v kap. 5.1 proti aktuálním zdrojům. (fáze 8)
- [x] Posoudit, zda sebetest (5.1 bod 3) dostatečně chrání před R2 – navrhnout lepší (např. ověření délky vtable, kontrola modulu, ze kterého pochází vtable). (fáze 8: pořadí ochran v 5.1)
- [ ] Ověřit správnost sady WinEvent událostí a strategie bez debounce (6.1), zejména pro UWP a Electron.
- [ ] Posoudit stavový stroj (6.3) na race conditions: událost během vlastního přesunu/přepínání, rychlé max/restore, zavření během přepínání.
- [ ] Zvážit, zda nepoužít `IVirtualDesktopNotification` místo polling/registry.
- [ ] Posoudit fallback (5.2) – existuje lepší veřejný způsob přesunu cizího okna?
- [ ] Zhodnotit odhady a pořadí fází.

---

## 12. Revize 3 – správa počtu ploch, nastavení, instalátor, O aplikaci

Stav: fáze 0–4 hotové (viz git log). Tato kapitola má přednost před staršími částmi plánu, kde si odporují (zejména 6.3 „vypůjčená plocha se nemaže“ a pravidlo „mazat jen plochy, které aplikace vytvořila“).

### 12.1 Pravidla správy ploch (zadání uživatele)

Pojmy:
- **Obsazená plocha** = je na ní aspoň jedno okno aplikace (viditelné i minimalizované; stejný filtr jako `is_candidate`). Okna vyloučených aplikací se počítají také. Okna připnutá na všechny plochy se nepočítají.
- **Výchozí počet B** = počet ploch při startu aplikace (bez pomocných ploch převzatých ze `state.json`). Uživatel ručně přidá plochu → B+1; ručně smaže → B−1 (min. 1). Ruční změna = změna seznamu ploch, kterou aplikace sama neprovedla.

**Základní pravidlo vstupu (zadání uživatele):** okno dostane při maximalizaci / celoobrazovkovém režimu (podle režimu spouštění) vlastní plochu jen tehdy, je-li na jeho ploše jiné viditelné maximalizované nebo celoobrazovkové okno aplikace (i vyloučené aplikace a naučené výjimky; minimalizované a cloaked se nepočítá; systémová okna nejsou okna aplikací). Jinak zůstává, kde je, není spravované a je „ponechané“ do zmenšení (pak se při nové maximalizaci posuzuje znovu). Platí za běhu i při startu a na všech plochách včetně první; v režimu „jen celoobrazovkový“ spouští jen celoobrazovkové okno, ale plochu zabírají maximalizovaná i celoobrazovková okna.

Invarianty (vynucuje „kompakce“, spouští se po každé události, která může změnit obsazenost: zánik/skrytí/objevení okna, přesun okna, výstup spravovaného okna, změna seznamu ploch):
1. **Plocha 1 se nikdy nemaže**, i když je prázdná.
2. **Žádná prázdná plocha mezi obsazenými**: prázdná plocha (kromě první), za kterou je ještě nějaká obsazená, se smaže; následující se tím posunou.
3. **Počet ploch ≥ B**: klesne-li pod B, přidají se prázdné plochy na konec. (Smazání mezery při počtu = B tedy znamená „smaž a vytvoř novou na konci“.)
4. **Počet ploch ≤ max(B, index poslední obsazené)**: prázdné plochy na konci nad B se mažou.
5. Prázdné plochy tedy existují jen na konci a jen do počtu B.

Přepnutí při smazání plochy, na kterou se uživatel dívá: cíl = původní plocha okna, jehož odchodem se plocha vyprázdnila (spravované okno → `ResolveOrigin`); jinak předchozí plocha v pořadí. Použít `remove(plocha, cíl)` – jedno volání.

Maximalizace: okno jde na první prázdnou plochu (jsou jen na konci), jinak se vytvoří nová na konci. Návrat (zmenšení/minimalizace): okno zpět na původní plochu, pak kompakce. Rychlá cesta z 6.3 (jedno `remove`) zůstává tam, kde plocha po odchodu okna stejně zanikne.

Prodleva: kompakce vyvolaná zánikem/skrytím okna se provede po `empty_desktop_grace_ms` (výchozí 300 ms) a jen pokud je plocha stále prázdná – jinak by plochu smazala aplikace, která zavírá úvodní okno a vzápětí otevírá hlavní. Výstup spravovaného okna (restore/minimize) prodlevu nemá.

Příklad (B = 3): `1 okna | 2 okna | 3 prázdná` → max. A → `… | 3 A` → max. B → `… | 3 A | 4 B` → zavřena všechna okna na 2 → `1 okna | 2 A | 3 B` → zavřeno A i B → `1 okna | 2 prázdná | 3 prázdná`.

Důsledky: příznak `borrowed` a nastavení `reuse_empty_desktops` ztrácejí smysl (nahrazuje je B). Je potřeba sledovat i nespravovaná okna (události SHOW/HIDE/DESTROY už chodí), ale stále bez slovníku všech oken – při kompakci se okna jednou vyjmenují.

### 12.2 Nastavení (dialog) – plán
Nativní Win32 dialog (šablona v .rc, `DialogBoxParamW`, vzniká jen na vyžádání → žádná paměť navíc v klidu), otevírá se z nabídky ikony. Soubor `%APPDATA%\AutoSpaces\settings.json`; stav, naučený seznam a log zůstávají v `%LOCALAPPDATA%\AutoSpaces`.
- **Co spouští přesun**: (a) všechna maximalizovaná okna i fullscreen (výchozí), (b) jen fullscreen (např. F11 v prohlížeči).
- **Výjimky**: seznam procesů, které se nikdy nepřesouvají; přidání výběrem ze seznamu právě běžících aplikací (název procesu + titulek okna) nebo ručně; odebrání. Okna výjimek zůstávají, kde jsou, dokud je uživatel nepřesune sám.
- **Naučené výjimky**: zobrazení a mazání (dnes v nabídce ikony), přepínač učení.
- Spouštět po startu Windows; animované přepnutí; prodleva mazání prázdné plochy.
- **Záložky** (dialog měl ~380 DU, na malých displejích se nevešel; nyní ~198 DU): *Obecné* (režim, rozmístění po startu + tlačítko Rozložení ploch…, spouštění po startu Windows, jazyk), *Aplikace* (vyloučené aplikace, naučené výjimky + učení), *Pokročilé* (tip k hlavnímu panelu + Otevřít nastavení Windows, záznam činnosti), *Klávesy* (přenášení okna, 12.13). Každá záložka je podřízený dialog (`DS_CONTROL`, `WS_EX_CONTROLPARENT`, `EnableThemeDialogTexture`) uvnitř `SysTabControl32`; OK čte všechny záložky, Ctrl+Tab / Ctrl+Shift+Tab přepíná zachycením přes `WH_MSGFILTER`, po vytvoření se dialog umístí do pracovní oblasti monitoru (je-li vyšší, zkrátí se záložky a tlačítka zůstanou vidět).

### 12.3 Instalátor – plán
Inno Setup, jeden `setup.exe`: `PrivilegesRequired=lowest` + `PrivilegesRequiredOverridesAllowed=dialog`.
- Výchozí: instalace pro aktuálního uživatele do `%LOCALAPPDATA%\Programs\AutoSpaces` (bez práv správce).
- Volitelně: pro všechny uživatele do `Program Files\AutoSpaces` (vyžádá práva správce).
- Zástupce v nabídce Start, volitelně spouštění po startu (HKCU Run / HKLM Run), odinstalace ukončí běžící aplikaci (`AppMutex`), nastavení v profilu uživatele se při aktualizaci zachová; při odinstalaci dotaz, zda je smazat.
- Verze z `Cargo.toml` → VERSIONINFO v exe → instalátor. Podpis exe i instalátoru.

**Skutečný stav (fáze 7) a odchylky:** `installer/autospaces.iss` + `tools/build-installer.ps1` (verze z `Cargo.toml` → `/DAppVersion`, výstup `dist/AutoSpaces-Setup-<verze>.exe`, `dist` v `.gitignore`).
- Jazyk: `[Languages]` angličtina první (záloha), čeština druhá; `ShowLanguageDialog=no`, `LanguageDetectionMethod=uilanguage` → česky jen při českém UI Windows, jinak anglicky. Dialog volby režimu instalace se zobrazuje ve stejném jazyce.
- Ukončení běžící aplikace: `AppMutex` je jen záloha (dotaz uživateli). Instalátor i odinstalátor nejdřív pošlou skrytému oknu `AutoSpacesHiddenWindow` `WM_CLOSE` a počkají (max ~15 s) na zmizení mutexu `Local\AutoSpaces_SingleInstance`. Aplikace nově `WM_CLOSE` zpracuje stejnou cestou jako „Ukončit“ v nabídce (`exit_clean`: vrátí okna, srovná plochy); nový parametr `--quit` totéž provede zvenčí a počká (kód 0 = skončila/neběžela, 1 = vypršel čas). `CloseApplications=no` (Restart Manager se nepoužívá).
- Autostart: úloha zapisuje stejnou hodnotu jako aplikace, `HKCU\...\Run\AutoSpaces = "<cesta k exe>"`. I při instalaci pro všechny uživatele je to HKCU (jen pro instalujícího uživatele), ne HKLM Run – zaškrtávátko v nastavení aplikace čte jen HKCU a z HKLM by hodnotu nešlo vypnout. Ostatní uživatelé si autostart zapnou v nastavení aplikace. Odinstalace maže hodnotu Run vždy (i když ji zapnula aplikace), ale jen u odinstalujícího uživatele.
- Odinstalace se po odebrání souborů zeptá (dvojjazyčně), zda smazat `%APPDATA%\AutoSpaces` a `%LOCALAPPDATA%\AutoSpaces` aktuálního uživatele; výchozí i při tichém režimu = ponechat.
- Zástupce v nabídce Start se vytváří vždy (`/NOICONS` ho nevypíná, protože je stránka skupiny skrytá). Podpis exe i instalátoru zatím není (SmartScreen bude varovat „neznámý vydavatel“) – fáze 9.

### 12.4 O aplikaci – plán
Nativní dialog z nabídky ikony: ikona, název, verze (`CARGO_PKG_VERSION`), autor, odkaz na web/GitHub, možnosti podpory (donate) ve stejné podobě jako v aplikaci Kolik platím (odkazy otevřít přes `ShellExecuteW`).

### 12.5 Upravené fáze
| Fáze | Obsah | Stav |
|---|---|---|
| 0–4 | kostra, plochy, logika, naučený seznam, ikona | hotovo |
| 4b | správa počtu ploch podle 12.1 | probíhá |
| 5 | settings.json + dialog Nastavení (12.2), spouštění po startu | hotovo |
| 5b | profil rozložení ploch po startu (12.7) | hotovo (vyzkoušeno živě při 96 DPI) |
| 6 | dialog O aplikaci (12.4), VERSIONINFO | hotovo (vyzkoušeno živě při 96 DPI; vysoké DPI neověřeno; odkaz na web a otevření peněženky živě neotevřeny) |
| 7 | instalátor (12.3) | hotovo (živě vyzkoušena tichá instalace, aktualizace a odinstalace při běžící aplikaci, autostart, průvodce česky i anglicky; režim pro všechny uživatele a smazání dat při odinstalaci živě ne; exe i instalátor nepodepsané) |
| 8 | varianty rozhraní pro Win10 1809–22H2 a Win11 21H2–23H2 (5.1), zpřísněný sebetest, režim „nepodporováno“ (5.3), skrytý klíč `backend`; záložní úroveň 2 vyhodnocena a nestavěna (5.2) | hotovo (5 nových variant **neověřeno** na skutečném systému – jen ze zdrojů; živě jen `--selftest --select-only` na 26340 → `win11-24h2`; režim „nepodporováno“ živě nevyzkoušen) |
| 9 | testy na dalších verzích Windows, podpis, vydání | čeká |

### 12.6 Podklady pro O aplikaci (zjištěno v projektu kolikplatim)
Kolik platím nemá samostatné okno O aplikaci, jen blok podpory na hlavní stránce (`src/components/SubscriptionApp.tsx`, texty v `src/lib/i18n.tsx`):
- Autor: odkaz „autora“ na `https://www.premysl-vavrousek.cz` (jméno v kódu vypsané není → do dialogu doplnit podle uživatele).
- Podpora 1: BTC přes Lightning Network na adresu `premek.v@bitlifi.com` (odkaz `lightning:premek.v@bitlifi.com`).
- Podpora 2: „účet“ – dialog s QR kódem (`src/assets/donate-qr.png`); číslo účtu je jen v obrázku. Pro AutoSpaces převzít stejný obrázek (vložit do exe jako prostředek, nebo otevřít z webu).
- Verze se tam nezobrazuje; v AutoSpaces bude z `CARGO_PKG_VERSION`.
- **Rozhodnuto uživatelem (2026-11-11):** autor v dialogu = „Přemysl Vavroušek“; QR obrázek převzat do `assets/donate-qr.png`.

### 12.7 Rozložení ploch po startu („profil ploch“) – plán

Cíl: v nastavení určit, kolik ploch má po startu existovat a které aplikace se na kterých mají otevřít (např. plocha 2 – Outlook, plocha 3 – TradingView).

**Nastavení (záložka „Plochy“ v dialogu 12.2):**
- Počet ploch po startu `N` (stane se výchozím počtem B z 12.1).
- Tabulka pravidel: plocha č. | název plochy (jen Win11) | aplikace | argumenty | spustit při startu | vždy držet na této ploše. Více aplikací na jedné ploše je možné.
- Výběr aplikace: z běžících oken, z nabídky Start (i aplikace ze Storu přes `shell:AppsFolder\<AUMID>`), nebo procházením k .exe.
- Tlačítko **Převzít současné rozložení**: z aktuálně otevřených oken vytvoří pravidla.
- Prodleva po přihlášení (výchozí 5 s), aby se aplikace nespouštěly do nenaběhlého systému.

**Formát (`settings.json`):**
```json
"layout": { "desktopCount": 3, "startDelayMs": 5000, "rules": [
  { "desktop": 2, "name": "Pošta",  "launch": "olk.exe",         "args": "", "match": { "process": "olk.exe" },                 "runAtStart": true, "keepOnDesktop": true },
  { "desktop": 3, "name": "Trhy",   "launch": "TradingView.exe", "args": "", "match": { "process": "tradingview.exe", "title": "" }, "runAtStart": true, "keepOnDesktop": false } ] }
```

**Postup při startu:**
1. Zajistit počet ploch ≥ N (`create` na konci), případně je pojmenovat (`SetDesktopName`, jen kde ho varianta má).
2. Pro každé pravidlo s `runAtStart`: pokud okno odpovídající `match` už existuje, jen ho přesunout; jinak spustit (`ShellExecuteW`) a čekat na první vyhovující okno (událost SHOW, bez dotazování; časový limit 30 s).
3. Okno přesunout na cílovou plochu (`move_window`), plochu nepřepínat. Uživatel zůstává na ploše 1.
4. Neúspěch (limit vypršel, okno nejde přesunout) zapsat do logu a ukázat v dialogu u pravidla; nepočítá se do naučeného seznamu.

**Souhra s pravidly 12.1:**
- **Plochy z profilu jsou pevné** – chovají se jako plocha 1: nemažou se, ani když jsou prázdné nebo uprostřed. Pomocné plochy pro maximalizovaná okna vznikají až za nimi a pro ně pravidla 12.1 platí dál.
- Okno aplikace z profilu se ze své přiřazené plochy při maximalizaci nepřesouvá (má plochu „svou“).
- `keepOnDesktop`: každé další okno té aplikace, které se objeví jinde, se přesune na její plochu.
- Dokud běží spouštění podle profilu, kompakce je pozastavená.

**Rizika / k ověření:**
- Párování okna s aplikací: spouštěč často skončí a okno patří jinému procesu (Outlook `olk.exe`, aplikace ze Storu, webové aplikace v Edge/Chrome) → párovat podle názvu procesu + volitelně části titulku, ne podle PID.
- Aplikace s jedinou instancí, které už běží; aplikace obnovující více oken; aplikace spuštěné jako správce (nelze přesunout).
- TradingView existuje jako desktopová aplikace i jako web v prohlížeči – druhý případ potřebuje párování podle titulku.

**Zařazení:** nová fáze 5b (po dialogu Nastavení, před O aplikaci). Odhad 2–3 dny.

**Revize po zkoušce se skutečnými aplikacemi (Outlook, TradingView):**
- *Příčiny chyb:* aplikace po přesunu aktivovaly své okno a Windows přepnul pohled na jeho plochu (profil končil hned po přesunu prvního okna, takže nic nevracel); okno se nemaximalizovalo; hlavní okno po úvodní obrazovce už nebylo okno profilu, otevřelo se maximalizované na aktuální ploše a běžná logika mu založila pomocnou plochu za plochami profilu (vypadalo to jako prázdná plocha mezi nimi); při SHOW ještě neexistuje view, takže přesun selhal a pravidlo se zahodilo.
- *Spouštění:* klasické aplikace `CreateProcessW` se `STARTF_USESHOWWINDOW` + `SW_SHOWMINNOACTIVE` (první okno minimalizované a neaktivní; `ShellExecute` exe naopak natáhl do procesu desítky DLL rozšíření shellu). Balíčkové aplikace (WindowsApps) přes `shell:AppsFolder\<AUMID>` (AUMID z `GetApplicationUserModelId` u běžícího procesu, u vybraného exe z `AppxManifest.xml`); staré záznamy s cestou do WindowsApps se při načtení převedou. Po dobu běhu profilu `LockSetForegroundWindow` (best effort, uvolní ho jakýkoli vstup uživatele).
- *Umístění:* okno se přesune, jakmile má view (do té doby časovač 40 ms), zaregistruje se jako umístěné profilem (běžná logika ho nikdy nedá na pomocnou plochu) a spuštěné aplikace se maximalizují bez aktivace: minimalizované okno přes `SetWindowPlacement(WPF_RESTORETOMAXIMIZED)` + `SW_SHOWNOACTIVATE`, ostatní `ShowWindowAsync(SW_MAXIMIZE)` až po vrácení fokusu (ověřeno: bez přepnutí pohledu). O minimalizaci cizího okna AutoSpaces nikdy nežádá (mod animace minimalizace ve Windhawku by do procesu nahrál Direct3D, +63 MB). Obnoví-li si aplikace vlastní umístění, maximalizuje se znovu (nejvýš 3×).
- *Doběh:* po prvním okně se aplikace 10 s „sleduje“ – další její nová okna (hlavní okno po úvodní obrazovce) jdou na stejnou plochu. Po celou dobu hlídač vrací pohled (bez animace) a fokus uživateli, pokud se přepnul kvůli oknu profilu; přepnutí na jinou plochu s vlastním oknem v popředí bere jako vůli uživatele.
- Aplikace z profilu, která při startu už běží (i minimalizovaná nebo na jiné ploše), se nespouští znovu: její okno se přesune na plochu pravidla a maximalizuje jako u spuštěné, bez přepnutí pohledu a bez sebrání fokusu (běží-li víc oken, bere se první nepřiřazené v pořadí výčtu oken).
- Maximalizované okno aplikace z profilu na ploše svého pravidla tam zůstává (i nové okno té aplikace). Vypnutý profil = žádná z těchto cest se nepoužije (test `profile_off_behaves_like_no_profile`).
- *Ověřeno živě* (Poznámkový blok přes AUMID, msinfo32): obě okna na svých plochách maximalizovaná do 2 s, 0 přepnutí pohledu, fokus uživateli nezmizel ani v jednom 15ms vzorku.

**Souhra s rozmístěním při startu (2026-11-13):** při startu aplikace se profil nespouští samostatným časovačem, ale jako součást jedné startovní sekvence s 12.11 (popis tam): plochy profilu vzniknou a připnou se dřív, než se rozmístí ostatní okna; `startDelayMs` platí pro celou sekvenci, jen při startu s Windows (ručně 500 ms); pohled se při startu přepne nejvýš jednou, na plochu okna z popředí (i když ho přesunul profil), a hlídač pohledu se vztahuje k ní, ne k ploše 1. „Použít teď“ v dialogu (`start_layout`) pohled nepřepíná, jako dosud. Úvodní okna spouštěných aplikací, která nejsou kandidáty, se přesouvají také.

**Odchylky při implementaci (fáze 5b):** profil je samostatný dialog „Rozložení ploch…“ (ne záložka) a pravidlo má jen plochu, název, `launch`, `args` a `match.process` – `runAtStart` je vždy zapnuté a `keepOnDesktop`, `match.title`, názvy ploch a tabulka s argumenty chybí; neúspěch je jen v logu, ne v dialogu. Oproti návrhu přibyl přepínač `enabled`, tlačítko „Použít teď“ a okno profilu se při čekání hledá i 500ms časovačem (jen po dobu čekání), protože při události SHOW ještě nemusí být kandidátem; plochy profilu se při výběru pomocné plochy pro maximalizované okno také přeskakují.

### 12.8 Přepínání do existujícího okna

- **Rozhodnutí uživatele:** vlastní zachytávání spouštění aplikací (Start, zástupci) se **nedělá**.
- V dialogu nastavení (12.2) je tlačítko „Otevřít nastavení Windows…“ (`ms-settings:multitasking`) s krátkým návodem: v části Plochy nastavit „Na hlavním panelu zobrazovat všechna otevřená okna“ na „Na všech plochách“. Přímý zápis do registru se **nedělá** (rozhodnutí uživatele): na sestavení 26340 Windows volbu neukládají do `VirtualDesktopTaskbarFilter`, ale do nedokumentovaného binárního záznamu CloudStore, a zápis do původní hodnoty se na panelu neprojevil.
- **Nová instance:** Shift + klik na ikonu v hlavním panelu (standard Windows); dialog to uvede jako nápovědu (klik na ikonu běžící aplikace přepne na její plochu). Alt zamítnut (Alt + dvojklik = Vlastnosti).
- Vedlejší účinek: hlavní panel ukazuje okna ze všech ploch.

### 12.9 Připomínka podpory – pravidla
- Dialog „O aplikaci“ a připomínka sdílejí šablonu i blok plateb (Revolut výchozí / Účet v CZK / Lightning s logem); připomínka je nemodální, zobrazí se bez aktivace na aktuální ploše.
- Podmínky: od prvního spuštění uplynulo aspoň 30 dní **a** aplikace v tomto běhu běží aspoň 30 minut. Jednorázový časovač (žádný polling); při vypršení se podmínky znovu ověří.
- Stav v `%LOCALAPPDATA%\AutoSpaces\support.json` (`firstRun`, `nextAfter`, `never`), načítá se tolerantně; ostatní soubory se nemění. Čas zpět: první spuštění v budoucnosti se bere jako „teď“, vzdálené `nextAfter` se omezí na 30 dní.
- Po zavření: zaškrtnuté „už nikdy nezobrazovat“ = nikdy; jinak nejdřív za dalších 30 dní (po 30 minutách běhu). Nejvýš jednou za běh.
- Ohleduplnost: při celoobrazovkovém okně v popředí, běžícím profilu rozložení, čekajících přechodech nebo otevřeném jiném dialogu se odloží o 5 minut. Rozhodovací logika je čistá funkce času (`core/support.rs`, testy). Ladicí sestavení: proměnná `AUTOSPACES_SHOW_SUPPORT=1` zobrazí připomínku hned.

### 12.10 Jazyk (čeština / angličtina)
- **Pravidlo:** jazyk uživatelského rozhraní Windows (`GetUserDefaultUILanguage`, primární jazyk čeština) → česky, jinak anglicky. Volba v `settings.json`: `"language": "auto" | "cs" | "en"` (výchozí `auto`; chybějící či neznámá hodnota = `auto`). V dialogu nastavení je rozbalovací seznam „Jazyk / Language“ (Automaticky / Čeština / English).
- **Použití:** menu ikony, tooltip a hlášky se řídí jazykem hned po potvrzení nastavení tlačítkem OK; dialogy otevřené později jsou už v novém jazyce (bez restartu aplikace). Otevřený dialog nastavení se nepřekresluje (dialog to říká v poznámce pod seznamem).
- **Kde jsou texty:** šablony dialogů jednou v `assets/dialogs.inc`, texty v `assets/strings-cs.inc` a `assets/strings-en.inc` (makro `T(NAME)` → `T_<JAZYK>_NAME`); `autospaces.rc` je zahrnuje dvakrát pod `LANGUAGE 0x05,0x01` a `LANGUAGE 0x09,0x01`. Varianta se načítá výslovně (`FindResourceExW` + `DialogBoxIndirectParamW` v `src/i18n.rs`), ne podle výchozího chování systému. Ostatní texty (menu, tooltip, hlášky, „Plocha N – …“, „nejméně N“, filtry souborových dialogů) jsou v tabulce `strings!` v `src/i18n.rs`. RC ořezává texty ovládacích prvků na 256 znaků, proto dva dlouhé odstavce (nastavení, rozložení) jsou v tabulce a nastavují se za běhu; test hlídá, aby žádný text v `.inc` nepřesáhl limit.
- **VERSIONINFO:** české (`040504B0`) i anglické (`040904B0`) bloky. QR obrázky jsou beze změny.
- **Přidání jazyka:** (1) varianta v `Lang` (`LANG_COUNT`, `LANGS`, `code`, `primary`, `native_name`), (2) další sloupec v každém řádku `strings!` (kompilátor vyžaduje), (3) soubor `assets/strings-<kód>.inc` + další blok `LANGUAGE … #include` v `autospaces.rc` a položka v `build.rs` (`rerun-if-changed`). Testy hlídají úplnost tabulky i `.inc` souborů, shodu zástupných znaků `{0}` a detekci jazyka.
- Log soubory zůstávají v jazyce, v jakém jsou. Pozor na různou délku textů: dialogy jsou dimenzovány na delší (anglickou) variantu.

### 12.11 Rozmístění už maximalizovaných oken po startu
- **Volba:** `"arrangeAtStart"` v `settings.json` (výchozí `true`, chybějící či špatně typovaný klíč = `true`); v dialogu nastavení zaškrtávací políčko ve skupině „Co spouští přesun…“: „Po spuštění rozmístit už maximalizovaná okna na samostatné plochy“ / „At startup, move already maximized windows to their own desktops“. Vypnuto = dosavadní chování (okna maximalizovaná při startu se nechají být, 12.1).
- **Kdy:** jen při startu aplikace, jako součást jedné startovní sekvence spolu s profilem 12.7 (viz „Startovní sekvence“ níže; ne při přepnutí „Zapnuto“, změně nastavení, po restartu Exploreru ani při `--restore`). Jen pokud jsou okna aplikací (i minimalizovaná) na nejvýš jedné ploše („obsazená plocha“ z 12.1, prázdné plochy se nepočítají) a není adoptovaný stav; okna na dvou a více plochách = vlastní rozložení uživatele, nic se nehýbe. Podmínka se vyhodnocuje na začátku sekvence, dřív než profil cokoli přesune nebo přidá plochy. Okna maximalizovaná během čekání na sekvenci se přidají vždy (běžná logika by je přesunula také).
- **Co:** okna podle režimu spouštění (všechna maximalizovaná → maximalizovaná i celoobrazovková; jen celoobrazovkový → jen celoobrazovková); minimalizovaná ne. Platí všechna pravidla kandidáta (systémová okna, vyloučené aplikace, naučené výjimky, připnutá okna). Okna aplikací z profilu rozložení se nechají profilu (žádný dvojí přesun ani plocha navíc). **První okno zůstává:** při rozmístění nejstarší z oken (pořadí výše) zůstane na své ploše, pokud s ním na ní nezůstává jiné maximalizované okno (mimo rozmístěná; vyloučená aplikace nebo nepřesunutelné okno ano) – obecné pravidlo 12.1 pro skupinu; ostatní mu pak plochu „zabírají“ a přesunou se. Okna profilu jdou vždy na své plochy, plocha 1 tak může zůstat prázdná.
- **Jak:** běžnou cestou `enter_space` (stejná evidence, návrat po zmenšení/zavření, úklid ploch, `--restore`, „Ukončit“; původní plocha = plocha 1), v hromadném režimu: bez přepnutí pohledu a bez aktivace po každém okně a neúspěchy se nepočítají do učení výjimek. Pořadí ploch: podle času startu procesu okna (`GetProcessTimes`, nejstarší = nejnižší plocha; neznámý čas poslední; shoda = z-pořadí odshora). Čas procesu `explorer.exe` odpovídá přihlášení, ne otevření okna, takže okna Průzkumníka jdou na plochy první; levný způsob, jak zjistit stáří okna, není. Prázdné existující plochy se využijí dřív než nové; plochy profilu (jeho počet `count()`) zůstávají profilu, chybějící se vytvoří předem a okna mimo profil jdou za ně. Okna Průzkumníka souborů (`explorer.exe`, `CabinetWClass`) jsou běžný kandidát; `SYSTEM_HOSTS` jen brání učení výjimek a výběr „z běžících“ pro rozložení ho nabízí jako „Průzkumník souborů“ také pro vyloučené aplikace. Pohled viz sekvence. Nepovedený přesun = okno zůstane, dokud se neobnoví (jako dosud).
- **Startovní sekvence (12.11 + 12.7, po zkoušce uživatele 2026-11-13):** `Orchestrator::startup_schedule(at_logon)` hned po `adopt` vrátí časovač, `run_startup` pak provede vše v jednom kroku:
  1. zapíše do logu plochy (počet, GUID, pohled, B) a okno v popředí; vybere okna k rozmístění (podmínka výše) – ještě se nic nehnulo;
  2. profil doplní své plochy 2..N (od té chvíle připnuté, `pinned = N`) a přesune + maximalizuje okna běžících aplikací profilu;
  3. ostatní maximalizovaná okna dostanou plochy za plochami profilu (nejstarší proces první);
  4. pohled se přepne (bez animace) nejvýš jednou: na plochu okna, které měl uživatel v popředí, pokud bylo přesunuto (i okno aplikace profilu, např. Outlook); jinak zůstává, kde byl (při startu běžně plocha 1). Hlídač pohledu profilu se pak vztahuje k této ploše (`return_to`) a fokus vrací jen oknu na právě zobrazené ploše;
  5. teprve potom se spustí chybějící aplikace profilu (`LockSetForegroundWindow` až od tohoto kroku).
  Kompakce (12.1) je od naplánování sekvence až do konce běhu profilu pozastavená a proběhne jednou na konci; okno maximalizované během čekání se nepřesune hned, ale ve sekvenci.
- **Prodleva:** při startu s Windows (proces vznikl do 2 min po procesu shellu, `GetShellWindow`; bez shellu = přihlašování) platí `startDelayMs` profilu pro celou sekvenci – aplikace teprve startují a obnovují okna, rozmístit je dřív by je minulo. Při ručním startu okna už existují, takže 500 ms (stačí, aby se fokus vrátil z nabídky Start / Průzkumníka). Nic se nedělá, je-li vypnutý profil i rozmístění.
- **Úvodní okna spouštěných aplikací:** okno aplikace profilu, které není kandidát (úvodní obrazovka bez titulku), ale je viditelné a nevlastněné, se přesune na plochu aplikace hned při první události (bez maximalizace, bez evidence). Windows ho ale vždy vytvoří na zobrazené ploše, takže tam může na okamžik bliknout; okno bez view (nástrojové) přesunout nejde a zůstane vidět, dokud se nezavře (zapíše se do logu).
- **Příčiny chyb ze zkoušky (doloženo testy `startup_user_scenario_*`):** rozmístění proběhlo hned a vrátilo `pinned` na 0, profil běžel až za 5 s. Po celou dobu byly plochy 2 a 3 prázdné a za nimi pomocné plochy 4 a 5 (viditelný mezistav), a první běžná kompakce (stačí skrytí libovolného okna) je jako „prázdné mezery“ smazala. Profil pak plochy 2..N doplnil na konec, za pomocné, takže TradingView skončil na ploše 5 (Outlook na 4). Hlídač profilu vracel pohled na plochu, kde uživatel byl při spuštění profilu (při oknu v popředí, které se nehýbalo, plocha 1), a fokus oknu, které mezitím profil přesunul jinam (Windows proto přepnul pohled) – odtud „přesun na plochu jedna“. Úvodní okno TradingView (není kandidát) profil vůbec neviděl.
- **Log:** `"log": true` v `settings.json` (zaškrtávátko v nastavení, viz 12.12; výchozí `false`, zapíše se jen když je zapnuto) zapne stejný log jako `AUTOSPACES_LOG=1`: `%LOCALAPPDATA%\AutoSpaces\log.txt` s časem, po 1 MB se přejmenuje na `log.old.txt`. Startovní sekvence loguje plochy na začátku a na konci, každé okno (proces, třída, cílová plocha jako „#pořadí GUID“, výsledek), spuštění aplikací profilu, každé přepnutí pohledu i kdo ho vyvolal.
- **Živé zkoušky:** ladicí sestavení s `AUTOSPACES_ONLY=a.exe,b.exe` (jen v debug) přesouvá jen okna vyjmenovaných procesů a podmínku rozmístění vyhodnocuje jen nad nimi; kompakce vidí všechna okna.

### 12.12 Nahlášení problému (diagnostika, GitHub Issues)
- **Princip:** aplikace nemá síťový kód. „Nahlásit problém…“ v nabídce ikony (vždy; nahradilo „Diagnostika pro nahlášení…“ i soubor `unsupported.txt`) otevře dialog (šablona 105, `src/report_dlg.rs`): text, co se stane; **Vytvořit diagnostický soubor** zapíše `%LOCALAPPDATA%\AutoSpaces\report.txt` a ukáže ho v Průzkumníku (`explorer /select`), **Zobrazit** ho otevře v Poznámkovém bloku, **Otevřít hlášení na GitHubu** otevře `ISSUE_URL` (`src/report.rs`, jediné místo: `…/issues/new?template=bug_report.yml`; do adresy se nedávají žádná data uživatele ani systému). Soubor uživatel přetáhne do hlášení sám.
- **Záznam činnosti:** v nastavení (záložka Pokročilé) zaškrtávátko „Zaznamenávat činnost pro diagnostiku“ = klíč `"log"` (platí hned po OK) + poznámka, že log obsahuje jen názvy programů, typy oken a čísla ploch. Je-li vypnutý, dialog hlášení to řekne (zapnout, chybu zopakovat, pak soubor vytvořit) a nabídne „Zapnout záznam činnosti“ (zapíše `"log": true` a log začne hned).
- **Obsah `report.txt`** (anglické názvy polí): verze AutoSpaces, build Windows + UBR, jazyk UI, velikost a DPI primárního monitoru, počet monitorů, režim, vybraná varianta rozhraní (ověřená / neověřená) a výsledek výběru variant (dřívější `unsupported.txt`: rozsahy, QueryService, sebetest, IID, příčina), počet ploch (z registru: Explorer volání COM z obsluhy tlačítka odmítá, 0x8001010D), `settings.json`, `learned.json`, posledních 400 řádků `log.txt` (při čerstvé rotaci doplněno z `log.old.txt`).
- **Anonymizace** (čisté funkce v `src/report.rs`, testy): složky profilu (`USERPROFILE`, `LOCALAPPDATA`, `APPDATA`, `TEMP`) → `%USERPROFILE%` atd., cizí `X:\Users\<jméno>\` → `<user>`, jméno uživatele, počítače a domény (celá slova, bez ohledu na velikost písmen) → `<user>` / `<computer>` / `<domain>`. V JSON souborech navíc: čitelné názvy aplikací (`excludedNames`, `name` v pravidlech a výjimkách; mohou pocházet z titulku okna) a argumenty spouštění → `(omitted)`, spouštěcí příkaz → jen název exe nebo AUMID. Nečitelný JSON se nekopíruje. Titulky oken se do souboru ani do logu nedostanou (log má proces, třídu okna, hwnd, GUID ploch).
- **Změny logu kvůli soukromí:** „layout: launched/cannot launch“ zapisuje jen `launch_label` (název exe / AUMID, ne cestu); poznámka o `variants.json` jen název souboru (dřív celá cesta k exe).
- **GitHub:** `.github/ISSUE_TEMPLATE/bug_report.yml` (formulář, popisky anglicky s českým překladem, příloha `report.txt` přetažením) a `config.yml` (prázdná hlášení povolena).
- **Ladění:** ladicí sestavení: `AUTOSPACES_OPEN_REPORT=1` otevře dialog hlášení, `AUTOSPACES_INSTANCE=x` mění název mutexu (zkušební kopie vedle nainstalované).

### 12.13 Přenášení okna mezi plochami (verze 0.2.0)
- **Ovládání:** držení Win+Ctrl+Shift + šipka vpravo/vlevo přenese aktivní (popředí) okno o jednu plochu, každý stisk jeden krok (`RegisterHotKey`, `MOD_WIN|MOD_CONTROL|MOD_SHIFT|MOD_NOREPEAT`, `VK_LEFT`/`VK_RIGHT`, id 1/2 na skrytém okně). Puštění kteréhokoli z modifikátorů = upuštění (okno i pohled zůstanou na cílové ploše), Esc = okno i pohled zpět na původní plochu. Zkratka se registruje jen s backendem (v režimu „nepodporováno“ je modul vypnutý) a jen když je `carryEnabled`; selže-li registrace (obsazeno), modul se nezapne, zapíše se to do logu a záložka Klávesy v nastavení to řekne. Ověřeno 2026-11-23 na 26340: obě kombinace volné (jen `RegisterHotKey` + hned `UnregisterHotKey`).
- **Hook:** `WH_KEYBOARD_LL` existuje jen po dobu nesení (instaluje se při prvním kroku, odinstaluje hned po dokončení), v klidu žádný hook ani polling. Callback nic neloguje ani neukládá: puštění Win/Ctrl/Shift → `PostMessage` (upuštění), Esc → `PostMessage` (zpět) a Esc se spolkne (dolů i nahoru), vše ostatní propouští. Pojistka po dobu nesení: časovač 250 ms kontroluje `GetAsyncKeyState` (kdyby Windows hook pro překročení časového limitu odpojil). Aby puštění Win neotevřelo nabídku Start (šipka šla do zkratky, Esc byl spolknut), pošle se při začátku nesení a po Esc jedna nepřiřazená klávesa (VK 0xFF, označená `dwExtraInfo`, hook ji přeskočí) – jen když je Win dole.
- **Okraj:** výchozí rotace (vpravo z poslední → plocha 1, vlevo z plochy 1 → poslední); `carryCreateNew` (výchozí vypnuto): vpravo z poslední vytvoří novou plochu (`create`, na konci), vlevo se zastaví na ploše 1. Jedna plocha + rotace = nic.
- **Rychlé mačkání:** cíl (a ukazatel) se posune hned; animované přepnutí (`ANIMATION_MS` = 440, jen kde varianta animaci má) dožene cíl jedním přepnutím rovnou na poslední cíl, mezikroky se nepřehrávají. Upuštění během animace se dokončí až po ní (připnuté okno se neodepíná uprostřed animace). Čisté funkce `core/carry.rs`: `step_target`, `next_action`, `choose_style`, `Osd`.
- **Pravidla pro okna:** ručně přenesené okno (skončí jinde, než začalo) přestane být spravované – záznam jeho prostoru se zruší, při zmenšení/zavření se nikam nevrací. Upuštění samo nic nepřesouvá ani nemaximalizuje; maximalizované okno zůstane i vedle jiného maximalizovaného (je „ponechané“ do zmenšení). Pak platí obecné pravidlo 12.1: maximalizuje-li ho uživatel na cílové ploše a je tam jiné maximalizované okno, jde na první volnou prázdnou plochu (`find_empty` – ověřeno testem, že prázdné plochy základu B na konci využije dřív, než vytvoří novou), jinak na novou; bez jiného maximalizovaného zůstává. Během nesení se žádná událost okna nezpracuje (uloží se a po upuštění se jednou znovu posoudí), kompakce ani časovač prázdné plochy neběží; úklid (12.1: B, plocha 1 nikdy, plochy profilu ne) proběhne jednou po upuštění. Nesení se nespustí během startovní sekvence ani běhu profilu.
- **Nepřesunutelné okno** (není kandidát / systémové / AutoSpaces, proces s vyšší úrovní integrity – UIPI, naučená výjimka – jen čtení, `CanViewMoveDesktops` = false, okno připnuté na všechny plochy nebo mimo zobrazenou plochu): nic se neděje, ukazatel na 1,2 s ukáže „Toto okno nelze přesunout“. Do naučených výjimek se nezapisuje nic.
- **Vizuální způsob** – skrytý klíč `"carryStyle": "auto" | "pin" | "moveSwitch" | "switchMove"` (bez UI, zapisuje se jen když není `auto`); `auto` = nejlepší dostupný: `pin` → `moveSwitch` → `switchMove`; nedostupný zvolený = další v tomto pořadí. Použitý způsob je v logu („carry: … method …“).
  1. `pin`: okno se po dobu nesení připne na všechny plochy (`IVirtualDesktopPinnedApps::PinView`, `QueryService(B5A399E7-1C87-46B8-88E9-FC5747B171BD, 4CE81583-1E4C-4632-A621-07A53543148F)`, sloty `IsViewPinned` 6, `PinView` 7, `UnpinView` 8), plochy se přepínají animací (`SwitchDesktopWithAnimation`), po upuštění `UnpinView` + `MoveViewToDesktop` na aktuální plochu; Esc = přepnutí zpět, odepnutí, přesun.
  2. `moveSwitch`: `SwitchDesktopAndMoveForegroundView` (slot 10 jen u `win11-24h2`); přesouvá okno v popředí – když nesené okno popředí ztratí, krok se udělá jako `switchMove`.
  3. `switchMove`: `MoveViewToDesktop` na cíl a pak animované přepnutí; funguje všude.
- **Zdroje rozhraní pro připínání:** IID, service ID i pořadí metod (IsAppIdPinned, PinAppID, UnpinAppID, IsViewPinned, PinView, UnpinView) shodně v MScholtes/VirtualDesktop – `VirtualDesktop.cs` (Win10), `VirtualDesktop11-21H2.cs`, `VirtualDesktop11.cs` a `VirtualDesktop11-23H2.cs` v commitu 546b673, aktuální `VirtualDesktop11.cs` (23H2 od 3085) a `VirtualDesktop11-24H2.cs` – a v mrob95/pyvda `com_defns.py` (bez podmínky na build); pro 22621+ navíc Ciantic/VirtualDesktopAccessor `src/interfaces.rs`. Proto je `pinnedApps` u všech šesti variant (čteno jen na webu, nic staženo ani spuštěno). Výstup `IsViewPinned` je BOOL (pyvda) nebo 1bajtový bool (Ciantic) – čte se do vynulovaného 8bajtového bufferu.
- **Ověření před prvním použitím v běhu (jen čtení):** `QueryService` s IID (špatné IID = E_NOINTERFACE) a `IsViewPinned` na nesené okno, o kterém veřejné `GetWindowDesktopId` říká, že je na konkrétní ploše (tedy připnuté není): výsledek musí být false a nic nesmí být zapsáno za 4 bajty, jinak se `pin` pro tento běh vypne (`moveSwitch`/`switchMove`). Odchylka od zadání: vlastní okna AutoSpaces (skryté okno, ukazatel = nástrojové okno, dialogy) nemají application view, takže na nich `IsViewPinned` nejde zavolat; nesené okno je s nezávislým veřejným API porovnatelný ekvivalent. Jako u sebetestu neověřené varianty se před voláním zapíše `pending pin:<varianta> <build>` do `selftest-guard.txt`; pád během kontroly = na tomto buildu se už nezkouší.
- **Okno nikdy nezůstane připnuté:** seznam připnutých oken (hwnd + pid) se zapíše do `state.json` (`"pinned"`) **před** `PinView` a smaže po `UnpinView`. Odepíná se při upuštění, při chybě `PinView` (hned `UnpinView`, nesení pokračuje bez připnutí), při chybě přepnutí, při zavření okna, při ukončení (Ukončit, `WM_CLOSE`/`--quit`, `WM_DESTROY` → `carry_abort`), po restartu Průzkumníka (`set_backend`) a při startu (`release_pins` po `adopt`, také v `--restore`): okno, které ještě žije a `IsViewPinned` vrátí true, se odepne.
- **Ukazatel (OSD):** `src/carry_ui.rs`, okno `WS_POPUP` s `WS_EX_TOPMOST|WS_EX_TOOLWINDOW|WS_EX_NOACTIVATE|WS_EX_TRANSPARENT|WS_EX_LAYERED` (neprůhledné, layered kvůli propouštění kliknutí, `WM_NCHITTEST` → `HTTRANSPARENT`), uprostřed pracovní oblasti monitoru neseného okna, velikost podle `GetDpiForMonitor`, barvy podle `SystemUsesLightTheme`, zaoblené (`SetWindowRgn`). Kroužky s čísly ploch, cílová zvýrazněná; u okraje ↻/↺ (rotace) nebo „+“ (nová plocha); nad 16 ploch „k / n“. Nástrojové okno nemá application view, takže ho přepnutí plochy neskryje (je vidět na každé ploše bez připínání) – živě neověřeno. Vlastní proces: watcher ho přeskakuje (`WINEVENT_SKIPOWNPROCESS`) a není kandidát (`WS_EX_TOOLWINDOW`). Zobrazuje se po dobu nesení a 0,8 s po upuštění, pak se okno zničí.
- **Nastavení:** nová záložka „Klávesy“ / „Keys“ (šablona 109): „Přenášet okno zkratkou Win+Ctrl+Shift+←/→“ (`carryEnabled`, výchozí zapnuto), pod ním „Za poslední plochou vytvořit novou plochu (jinak se plochy točí dokola)“ (`carryCreateNew`, výchozí vypnuto, zašedlé při vypnutém přenášení), popis ovládání a stavový řádek (zkratka obsazená / Windows nepodporováno). Klíče se načítají tolerantně.
- **Ladění (jen debug):** `AUTOSPACES_SHOW_CARRY_OSD=<počet>:<cíl>[:new]` nebo `=nomove` ukáže ukazatel na 10 s bez přesunu, `AUTOSPACES_OSD_THEME=dark|light`, `AUTOSPACES_SETTINGS_TAB=<n>`, `AUTOSPACES_NO_HOTKEY=1` (zkratku nechá nainstalované instanci), `AUTOSPACES_NO_NOTICE=1`. `--selftest --select-only` vypíše, co varianta pro přenášení popisuje (nic nevolá).
- **Ověřeno jen testy** (falešný backend s pin/moveSwitch, 30 nových testů); živě vyzkoušeno jen: volnost zkratky, `--selftest --select-only`, snímky ukazatele a nastavení. Vzhled všech tří způsobů, chování hooku, maskování klávesy Win a viditelnost ukazatele během animace vyzkouší uživatel.