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

Maximalizace: okno jde na první prázdnou plochu (jsou jen na konci), jinak se vytvoří nová na konci. (Upřesnění 0.2.1 – B je jen minimální počet, prohlížená prázdná plocha, plochy profilu: 12.14.) Návrat (zmenšení/minimalizace): okno zpět na původní plochu, pak kompakce. Rychlá cesta z 6.3 (jedno `remove`) zůstává tam, kde plocha po odchodu okna stejně zanikne.

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
| 10 | 0.2.2: stav okna u aplikací rozložení (12.16), tapety ploch (12.17) | hotovo v kódu a testech; živě nevyzkoušeno (zadání: žádné živé testy, které mění tapety nebo hýbou okny) |

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
- **Plochy z profilu jsou pevné** – chovají se jako plocha 1: nemažou se, ani když jsou prázdné nebo uprostřed. Pomocné plochy pro maximalizovaná okna vznikají až za nimi a pro ně pravidla 12.1 platí dál. **Od 0.2.1 jen po dobu startovní sekvence / běhu profilu, potom je počet z profilu jen minimum B – viz 12.14.**
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
- *Umístění:* okno se přesune, jakmile má view (do té doby časovač 40 ms), zaregistruje se jako umístěné profilem (běžná logika ho nikdy nedá na pomocnou plochu) a spuštěné aplikace se maximalizují bez aktivace: minimalizované okno přes `SetWindowPlacement(WPF_RESTORETOMAXIMIZED)` + `SW_SHOWNOACTIVATE`, ostatní `ShowWindowAsync(SW_MAXIMIZE)` až po vrácení fokusu (ověřeno: bez přepnutí pohledu). O minimalizaci cizího okna AutoSpaces nikdy nežádá (mod animace minimalizace ve Windhawku by do procesu nahrál Direct3D, +63 MB). Obnoví-li si aplikace vlastní umístění, maximalizuje se znovu (nejvýš 3×). **Od 0.2.2** se místo maximalizace nastaví stav okna podle pravidla (maximalizovaně / v okně / minimalizovaně, 12.16).
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
- **Princip:** hlášení se nikam neodesílá (jediné síťové spojení aplikace je kontrola aktualizací na GitHubu, 12.15). „Nahlásit problém…“ v nabídce ikony (vždy; nahradilo „Diagnostika pro nahlášení…“ i soubor `unsupported.txt`) otevře dialog (šablona 105, `src/report_dlg.rs`): text, co se stane; **Vytvořit diagnostický soubor** zapíše `%LOCALAPPDATA%\AutoSpaces\report.txt` a ukáže ho v Průzkumníku (`explorer /select`), **Zobrazit** ho otevře v Poznámkovém bloku, **Otevřít hlášení na GitHubu** otevře `ISSUE_URL` (`src/report.rs`, jediné místo: `…/issues/new?template=bug_report.yml`; do adresy se nedávají žádná data uživatele ani systému). Soubor uživatel přetáhne do hlášení sám.
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
- **Pravidla pro okna:** ručně přenesené okno (skončí jinde, než začalo) přestane být spravované – záznam jeho prostoru se zruší, při zmenšení/zavření se nikam nevrací. Upuštění samo nic nepřesouvá ani nemaximalizuje; maximalizované okno zůstane i vedle jiného maximalizovaného (je „ponechané“ do zmenšení). Pak platí obecné pravidlo 12.1: maximalizuje-li ho uživatel na cílové ploše a je tam jiné maximalizované okno, jde na první volnou prázdnou plochu (`find_empty` – ověřeno testem, že prázdné plochy základu B na konci využije dřív, než vytvoří novou), jinak na novou; bez jiného maximalizovaného zůstává. Během nesení se žádná událost okna nezpracuje (uloží se a po upuštění se jednou znovu posoudí), kompakce ani časovač prázdné plochy neběží; úklid (12.1: B, plocha 1 nikdy; plochy profilu od 0.2.1 jen během jeho běhu, 12.14) proběhne jednou po upuštění. Nesení se nespustí během startovní sekvence ani běhu profilu.
- **Nepřesunutelné okno** (není kandidát / systémové / AutoSpaces, proces s vyšší úrovní integrity – UIPI, naučená výjimka – jen čtení, `CanViewMoveDesktops` = false, okno připnuté na všechny plochy nebo mimo zobrazenou plochu): nic se neděje, ukazatel na 1,2 s ukáže „Toto okno nelze přesunout“. Do naučených výjimek se nezapisuje nic.
- **Vizuální způsob** – skrytý klíč `"carryStyle": "auto" | "overlay" | "pin" | "moveSwitch" | "switchMove"` (bez UI, zapisuje se jen když není `auto`); `auto` = nejlepší dostupný: `overlay` → `pin` → `moveSwitch` → `switchMove`; nedostupný zvolený = další v tomto pořadí. Použitý způsob je v logu („carry: … method …“).
  0. `overlay` (od 2026-11-24, výchozí; popis níže v „Overlay ve stylu Task View“): během nesení se nic nepřepíná ani nepřesouvá, jen se posouvá zvýraznění v liště náhledů; při upuštění `MoveViewToDesktop` + `SwitchDesktop` bez animace pod vrstvami, pak vrstvy zmizí; Esc = vrstvy zmizí, nic se nepřepne.
  1. `pin`: okno se po dobu nesení připne na všechny plochy (`IVirtualDesktopPinnedApps::PinView`, `QueryService(B5A399E7-1C87-46B8-88E9-FC5747B171BD, 4CE81583-1E4C-4632-A621-07A53543148F)`, sloty `IsViewPinned` 6, `PinView` 7, `UnpinView` 8), plochy se přepínají animací (`SwitchDesktopWithAnimation`), po upuštění `UnpinView` + `MoveViewToDesktop` na aktuální plochu; Esc = přepnutí zpět, odepnutí, přesun.
  2. `moveSwitch`: `SwitchDesktopAndMoveForegroundView` (slot 10 jen u `win11-24h2`); přesouvá okno v popředí – když nesené okno popředí ztratí, krok se udělá jako `switchMove`.
  3. `switchMove`: `MoveViewToDesktop` na cíl a pak animované přepnutí; funguje všude.
- **Zdroje rozhraní pro připínání:** IID, service ID i pořadí metod (IsAppIdPinned, PinAppID, UnpinAppID, IsViewPinned, PinView, UnpinView) shodně v MScholtes/VirtualDesktop – `VirtualDesktop.cs` (Win10), `VirtualDesktop11-21H2.cs`, `VirtualDesktop11.cs` a `VirtualDesktop11-23H2.cs` v commitu 546b673, aktuální `VirtualDesktop11.cs` (23H2 od 3085) a `VirtualDesktop11-24H2.cs` – a v mrob95/pyvda `com_defns.py` (bez podmínky na build); pro 22621+ navíc Ciantic/VirtualDesktopAccessor `src/interfaces.rs`. Proto je `pinnedApps` u všech šesti variant (čteno jen na webu, nic staženo ani spuštěno). Výstup `IsViewPinned` je BOOL (pyvda) nebo 1bajtový bool (Ciantic) – čte se do vynulovaného 8bajtového bufferu.
- **Ověření před prvním použitím v běhu (jen čtení):** `QueryService` s IID (špatné IID = E_NOINTERFACE) a `IsViewPinned` na nesené okno, o kterém veřejné `GetWindowDesktopId` říká, že je na konkrétní ploše (tedy připnuté není): výsledek musí být false a nic nesmí být zapsáno za 4 bajty, jinak se `pin` pro tento běh vypne (`moveSwitch`/`switchMove`). Odchylka od zadání: vlastní okna AutoSpaces (skryté okno, ukazatel = nástrojové okno, dialogy) nemají application view, takže na nich `IsViewPinned` nejde zavolat; nesené okno je s nezávislým veřejným API porovnatelný ekvivalent. Jako u sebetestu neověřené varianty se před voláním zapíše `pending pin:<varianta> <build>` do `selftest-guard.txt`; pád během kontroly = na tomto buildu se už nezkouší.
- **Okno nikdy nezůstane připnuté:** seznam připnutých oken (hwnd + pid) se zapíše do `state.json` (`"pinned"`) **před** `PinView` a smaže po `UnpinView`. Odepíná se při upuštění, při chybě `PinView` (hned `UnpinView`, nesení pokračuje bez připnutí), při chybě přepnutí, při zavření okna, při ukončení (Ukončit, `WM_CLOSE`/`--quit`, `WM_DESTROY` → `carry_abort`), po restartu Průzkumníka (`set_backend`) a při startu (`release_pins` po `adopt`, také v `--restore`): okno, které ještě žije a `IsViewPinned` vrátí true, se odepne.
- **Ukazatel (OSD):** `src/carry_ui.rs`, okno `WS_POPUP` s `WS_EX_TOPMOST|WS_EX_TOOLWINDOW|WS_EX_NOACTIVATE|WS_EX_TRANSPARENT|WS_EX_LAYERED` (neprůhledné, layered kvůli propouštění kliknutí, `WM_NCHITTEST` → `HTTRANSPARENT`), dole na pracovní oblasti monitoru neseného okna nad hlavním panelem (vodorovně na středu, 40 DIP nad spodním okrajem pracovní oblasti; při panelu nahoře/vlevo/vpravo prostě spodní okraj pracovní oblasti; `core::overlay::osd_pos`, od 2026-11-24, dřív uprostřed); u způsobu `overlay` ho nahrazuje lišta náhledů (zůstává jen „Toto okno nelze přesunout“), velikost podle `GetDpiForMonitor`, barvy podle `SystemUsesLightTheme`, zaoblené (`SetWindowRgn`). Kroužky s čísly ploch, cílová zvýrazněná; u okraje ↻/↺ (rotace) nebo „+“ (nová plocha); nad 16 ploch „k / n“. Nástrojové okno nemá application view, takže ho přepnutí plochy neskryje (je vidět na každé ploše bez připínání) – živě neověřeno. Vlastní proces: watcher ho přeskakuje (`WINEVENT_SKIPOWNPROCESS`) a není kandidát (`WS_EX_TOOLWINDOW`). Zobrazuje se po dobu nesení a 0,8 s po upuštění, pak se okno zničí.
- **Nastavení:** nová záložka „Klávesy“ / „Keys“ (šablona 109): „Přenášet okno zkratkou Win+Ctrl+Shift+←/→“ (`carryEnabled`, výchozí zapnuto), pod ním „Za poslední plochou vytvořit novou plochu (jinak se plochy točí dokola)“ (`carryCreateNew`, výchozí vypnuto, zašedlé při vypnutém přenášení), popis ovládání a stavový řádek (zkratka obsazená / Windows nepodporováno). Klíče se načítají tolerantně.
- **Ladění (jen debug):** `AUTOSPACES_SHOW_CARRY_OSD=<počet>:<cíl>[:new]` nebo `=nomove` ukáže ukazatel na 10 s bez přesunu, `AUTOSPACES_OSD_THEME=dark|light`, `AUTOSPACES_SETTINGS_TAB=<n>`, `AUTOSPACES_NO_HOTKEY=1` (zkratku nechá nainstalované instanci), `AUTOSPACES_NO_NOTICE=1`. `--selftest --select-only` vypíše, co varianta pro přenášení popisuje (nic nevolá).
- **Ověřeno jen testy** (falešný backend s pin/moveSwitch, 30 nových testů); živě vyzkoušeno jen: volnost zkratky, `--selftest --select-only`, snímky ukazatele a nastavení. Vzhled všech tří způsobů, chování hooku, maskování klávesy Win a viditelnost ukazatele během animace vyzkouší uživatel.

#### Overlay ve stylu Task View (`carryStyle: "overlay"`, 2026-11-24)
- **Proč ne přímo Win+Tab:** zobrazení úloh (Task View) je vnitřní XAML rozhraní Průzkumníka bez veřejného API. Dokumentované `IVirtualDesktopManager` má jen `GetWindowDesktopId`, `IsWindowOnCurrentVirtualDesktop`, `MoveWindowToDesktop` (learn.microsoft.com, ověřeno 2026-11-24); otevřít jde zkratkou nebo shell příkazem, ale nejde v něm držet nesené okno ostré nad ním, posouvat zvýraznění ani upustit okno (Task View se aktivuje, tím vezme fokus a překryje i topmost okna). Proto vlastní vrstvy ve stejném duchu.
- **Vzhled:** (1) na každém monitoru vrstva přes pracovní oblast: cílová plocha (na začátku aktuální) živě rozmazaná a ztmavená (`DIM` 0,62), viz úprava níže; (2) nesené okno nad ní ostré na svém místě – je to DWM náhled 1:1 (zdroj = viditelný rám `DWMWA_EXTENDED_FRAME_BOUNDS`), skutečné okno se vůbec nemění (žádný dočasný topmost, žádná změna z-pořadí – není co obnovovat, ani po pádu; odchylka od návrhu „okno dočasně HWND_TOPMOST“, zdůvodnění: nic se nemusí vracet, funguje i u celoobrazovkových a vlastních topmost oken); (3) dole na monitoru neseného okna lišta živých náhledů ploch (pozadí plochy + DWM náhledy jejích oken, nesené okno jen v cílové), popisek = vlastní název plochy z `…\VirtualDesktops\Desktops\{GUID}\Name`, jinak „Plocha N“; `+` za poslední jen s `carryCreateNew`; cíl zvýrazněný rámečkem v barvě zvýraznění, rámeček klouže (Hermite, 200 ms, nový krok během pohybu navazuje rychlostí = slučování), náhledy kolem cíle se nepatrně zmenší (0,93); rotace přes okraj = krátký skok (140 ms) na druhý konec. Vrstvy se objeví za 110 ms a zmizí za 150 ms; při vypnutých animacích Windows (`SPI_GETCLIENTAREAANIMATION`) se rámeček přesouvá hned.
- **Jak:** pro každou plochu a monitor malá kopie („mini“, ~320 px na šířku, `mini_scale`), skryté vlastní okno (`DWMWA_CLOAK`) s tapetou (GDI) a DWM náhledy oken té plochy. Náhledy v liště = náhledy těchto kopií; rozmazání = kopie aktuální plochy zvětšená 16× s posunem o čtvrtiny malého pixelu a se stejnými vahami (`tap_alphas`), čili box blur skládaný DWM (GPU), žádné kreslení na CPU za snímek. Kopie, které začínají mimo vrstvu, se ořezávají po celých malých pixelech (`clip_copy`; jinak DWM kreslí horní část černě – zjištěno prototypem). Pozdější náhled se kreslí nahoře (ověřeno), proto okna od spodu a náhrada neseného okna jako poslední.
- **Prototyp (2026-11-24, jen vykreslení, nic nepřepnuto ani nepřesunuto):** (a) `DwmRegisterThumbnail` veřejně vydá živý náhled okna z neaktivní plochy (cloaked = 2) – ano, i u Chrome, Outlooku, WinUI; UWP pozastavené okno ukazuje úvodní obrazovku; vlastní cloaked okno jako zdroj – ano, i vnořené náhledy a zvětšení přes `rcSource`; (b) `DWMWA_SYSTEMBACKDROP_TYPE` (acrylic/mica, popup i rámové okno, i s `WM_NCACTIVATE`) na oknu, které není aktivní, kreslí jen plnou náhradní barvu, nerozmazává – vrstva aktivní být nesmí (fokus má nesené okno), proto vlastní rozmazání výše; funguje stejně na Win10 (stačí DWM náhledy, Windows 8+); (c) časování na tomto PC (144 Hz, 2560×1440): snímky po `DwmFlush` průměr 6,9 ms, medián 6,9, p95 7,5–8,0, nejdelší 12–14 ms (jeden vynechaný snímek), práce za snímek < 1 ms.
- **Tapety:** per-desktop `HKCU\…\Explorer\VirtualDesktops\Desktops\{GUID}\Wallpaper`, jinak `SPI_GETDESKWALLPAPER`; umístění z `Control Panel\Desktop` (`WallpaperStyle`, `TileWallpaper`; Span se kreslí jako Fill na každý monitor), barva pozadí `Control Panel\Colors\Background`. Dekódování WIC v pracovním vlákně (MTA) přímo na velikost kopie; JPEG přes `IWICBitmapSourceTransform` (škálování při dekódování), ostatní přes Fant scaler; hotové se pošle oknu lišty (`WM_APP+41`, s číslem generace), kopie se překreslí. Do té doby plná barva pozadí.
- **Paměť a velikost (měřeno 2026-11-24):** release s `debug-assertions` (stejná optimalizace): v klidu 2,7 MB private; během nesení 3,6 MB; krátká špička při dekódování tapet ~40 MB < 0,25 s (tapety tohoto PC jsou PNG 4K – PNG se dekóduje celé; JPEG se škáluje už při dekódování), po skončení 2,8 MB. Exe 910 848 B (dřív 854 528 B).
- **Více monitorů:** rozmazání na všech monitorech (každý svou kopií aktuální plochy), lišta jen na monitoru se středem neseného okna; geometrie (`strip`, `to_mini`, záporné souřadnice, panel nahoře) jen jednotkovými testy – tento PC má jeden monitor.
- **Vrstvy:** `WS_POPUP`, `WS_EX_TOOLWINDOW|WS_EX_NOACTIVATE|WS_EX_TRANSPARENT|WS_EX_LAYERED|WS_EX_TOPMOST` (kopie bez topmost, cloaked), `WM_NCHITTEST` → `HTTRANSPARENT`, `WM_MOUSEACTIVATE` → `MA_NOACTIVATE`; vlastní proces → watcher je přeskakuje, nikdy kandidáti. Vznikají při prvním kroku nesení, při zmizení se všechny náhledy odregistrují, okna zničí a bitmapy uvolní.
- **Smyčka:** hlavní smyčka čeká `MsgWaitForMultipleObjectsEx` s časem 0 jen když se něco hýbe nebo mizí, a pak `overlay_ui::frame` (pohyb, průhlednost, `DwmFlush` = vsync); jinak nekonečně (v klidu i během držení bez pohybu nula CPU). Po zmizení se do logu zapíše počet snímků, průměr a nejdelší.
- **Pojistky:** nejdou-li vrstvy vytvořit (nebo selže náhled neseného okna), `carry_overlay_failed` → další způsob (`pin` s kontrolou, jinak `moveSwitch`/`switchMove`), overlay se v tomto běhu už nezkouší; bez DWM nebo ve vzdálené relaci (`SM_REMOTESESSION`) se overlay nenabízí. Ukončení (Ukončit, `WM_CLOSE`/`--quit`, `WM_DESTROY`) → vrstvy hned pryč; restart Průzkumníka → `carry_abort` a zmizení; 30 s bez klávesy zkratky během nesení → jako Esc (nic se nepřepne). Pád procesu vrstvy odnese s sebou a na uživatelově okně nic nezůstane.
- **Testy:** `core/overlay.rs` (pohyb a navázání rychlosti, skok přes okraj, prolínání, váhy rozmazání, geometrie kopií a zdroje náhledu, lišta, zvýraznění, popisky, tapety, zmenšení obrázku, ořez kopií, ukazatel dole) a orchestrátor (overlay nic nepřepíná do upuštění, jedno přepnutí bez animace, Esc nic, nová plocha, náhradní způsob, ukončení/zavřené okno, volba způsobu); dekódování systémové JPEG tapety přes WIC.
- **Ladění (jen debug):** `AUTOSPACES_OVERLAY_DEMO=<složka>` vykreslí vrstvy nad aktuální plochou asi na 4 s (popředí nebo s `AUTOSPACES_OVERLAY_DEMO_SAMPLE=1` vlastní ukázkové okno jako „nesené“), posune zvýraznění, nechá zmizet, ukáže ukazatel ostatních způsobů; snímky BMP a `demo.txt` (časování) do složky; `…_NOSHOTS=1` bez snímků, `…_NOWALL=1` bez tapet. Nic nepřepíná, nepřesouvá, neregistruje zkratku ani hook.
- **Ověřeno jen testy / zkouškou uživatele:** skutečné nesení s přepnutím při upuštění, obnovení fokusu po přepnutí, chování při restartu Průzkumníka, více monitorů, HDR/jiné DPI, Win10.
- **Úprava 2026-11-24 odpoledne (zkouška uživatele: okno neostré, pozadí se nemění):**
  - *Neostrost – příčina:* `DwmQueryThumbnailSourceSize` vrací velikost viditelného rámu (1146×613 u okna 1160×620), ale `rcSource` se počítá od levého horního rohu obdélníku okna (včetně neviditelných okrajů). První verze přepočítávala „velikost zdroje / velikost okna“, takže okno zmenšila asi o 1 % a posunula – vypadalo jako v nižším rozlišení. Teď je `rcSource` = rám v souřadnicích okna bez škálování (`source_rect`), jen u jiné velikosti (DPI virtualizace) poměrně. Ověřeno demem s ukázkovým oknem s 1px čarami a drobným písmem: snímek obrazovky náhledu vs. skutečné okno – uvnitř okna 0 rozdílných pixelů, rozdíly jen v krajních 3 px (okraj a zaoblené rohy se míchají s jiným pozadím). Náhled 1:1 tedy spolehlivě jde; vkládání vrstev pod skutečné okno (bez topmost) proto není potřeba a zůstává výhoda, že se okno uživatele vůbec nemění.
  - *Pozadí podle cílové plochy:* rozmazané pozadí ukazuje vždy cíl (na začátku aktuální plochu); při kroku se vodorovně posune (cíl přijede ze strany, kam se jde, mezera 24 DIP) se stejným pohybem jako rámeček v liště (Hermite 200 ms, navazující rychlost = slučování), při skoku přes okraj (140 ms) se prolne na místě (`backdrop`). Nesené okno v pozadí není (jen nahoře 1:1; v liště dál v náhledu cílové plochy, kreslené přes náhled – `map_into`). Esc vrátí pozadí na výchozí plochu během mizení.
  - *Jak a plynulost:* každá plocha má předem rozmazaný obraz („Pre“, skryté okno v polovičním rozlišení se 16 posunutými kopiemi malé kopie plochy); pozadí je jeden náhled tohoto obrazu na plochu v záběru, takže za snímek se mění 1–2 vlastnosti místo 34 (34 změn za snímek + plné rozlišení vynechávalo snímky, p95 13 ms). Obrazy se ukáží hned na začátku s průhledností 0 (první zobrazení jinak stálo jeden 13ms snímek). Měřeno (release s debug-assertions, 144 Hz): průměr 6,9 ms, medián 7,0, nejdelší 9,7–11,8 ms, 0–2 snímky nad 10 ms z ~155, práce za snímek průměr 0,5 ms, nejvýš 2 ms.
  - *Paměť a velikost:* v klidu beze změny (2,7 MB); během nesení ~4 MB; špička při dekódování PNG tapet ~45 MB < 0,5 s; po skončení 2,8 MB. Obrazy pozadí jsou okna DWM (GPU), ne paměť procesu. Exe 916 480 B.
- **Úprava 2026-11-24 večer (akryl jako Win+Tab, jediný ukazatel):**
  - *Co umí DWM a Windows:* akryl Windows = rozmazané pozadí za oknem + tónování (+ luminosita a jemný šum v XAML `AcrylicBrush`). Win32 aplikace má dvě dokumentované cesty: (a) `DWMWA_SYSTEMBACKDROP_TYPE` (Mica/Acrylic) – kreslí se jen u aktivního okna, u neaktivního plná barva; obejít to dokumentovaně nejde (`WM_NCACTIVATE` ani rámové styly nepomohly), a naše vrstvy aktivní být nesmí; (b) Windows.UI.Composition z Win32: `DWMWA_USE_HOSTBACKDROPBRUSH` (Windows 11, build 22000+) + `ICompositorDesktopInterop::CreateDesktopWindowTarget` + `Compositor.CreateHostBackdropBrush` – „host backdrop“ je už rozmazaný obraz toho, co DWM složí pod oknem, živě, a ověřeno funguje i u okna, které nikdy není aktivní (`WS_EX_NOACTIVATE`). Zvolena (b). Šum a luminositu by šlo jen přes `CompositionEffectBrush` s vlastní implementací `IGraphicsEffect` (COM objekt) – vynecháno, tón je barevná vrstva (tmavý 28,28,30 / α 150; světlý 238,238,240 / α 140). (c) Vlastní rozmazání zachyceného obrazu (D2D) nejde – obsah jiných ploch je k dispozici jen jako DWM náhledy, ne jako pixely.
  - *Bez crate `windows`:* WinRT se volá přes vtable nad windows-sys (`src/glass.rs`); funkce `CreateDispatcherQueueController` (CoreMessaging.dll) a `RoActivateInstance`/`WindowsCreateString` (combase.dll) se načítají až při prvním nesení (`LoadLibraryW`/`GetProcAddress`). Pořadí metod rozhraní přečteno z `C:\Windows\System32\WinMetadata\Windows.UI.winmd` (ICompositor 6–29, ICompositor3.CreateHostBackdropBrush 6, ICompositionTarget.put_Root 7, IContainerVisual.Children 6, IVisualCollection.InsertAtTop 9, ISpriteVisual.put_Brush 7, IVisual.put_Opacity 23, put_Size 36). Compositor a DispatcherQueue vzniknou při prvním nesení a zůstanou (vlákno smí mít jen jeden controller).
  - *Vrstvy teď:* (1) pozadí na každém monitoru v plném rozlišení – přímo v něm DWM náhledy tapety (malý obrázek zvětšený) a oken cílové plochy 1:1, posun / prolnutí jako dosud (náhledy se posouvají po snímku); (2) nad ním akrylové sklo (vlastní okno `WS_EX_NOREDIRECTIONBITMAP`, host backdrop + tón, průhlednost vizuálu při objevení/zmizení); (3) nesené okno 1:1 v samostatném okně na jeho místě; (4) lišta. Bez akrylu (Windows 10, vypnuté „Efekty průhlednosti“ – `EnableTransparency` = 0, chyba kompozice) tmavá vrstva α 0,55 bez rozmazání (tapeta v poloviční velikosti malého obrázku, okna 1:1 – žádné nízké rozlišení oken).
  - *Zjištění DWM:* náhled, jehož cíl začíná nad/vlevo od okna, DWM nevykreslí (proto `clip_thumb`); náhled skrytého (cloaked) okna v plné velikosti vyšel prázdný nebo ukázal obraz za ním (náhledy panelů se proto používají jen zmenšené v liště).
  - *Starý ukazatel s tečkami odstraněn* (okno, kreslení, `Osd`/`Edge`, testy, `AUTOSPACES_SHOW_CARRY_OSD`). Proč se „občas“ objevoval: když se overlay jednou nepodařilo vytvořit (`carry_overlay_failed`), vypnul se pro celý zbytek běhu a další nesení šla způsobem `pin` s tečkami. Teď: selhání platí jen pro to jedno nesení (další zkusí overlay znovu, důvod jde do logu) a jediným ukazatelem pro všechny způsoby je lišta náhledů (u `pin`/`moveSwitch`/`switchMove` bez pozadí a skla). „Toto okno nelze přesunout“ = krátký popisek dole nad hlavním panelem (`carry_ui::hint_show`, 1,2 s). Nový začátek nesení během mizení starých vrstev je vytvoří znovu.
  - *Pojistka snímků:* pohyb se kreslí, dokud jeden snímek nevykreslí konečnou polohu (`settled`), i když bylo vlákno mezitím zaneprázdněné.
  - *Měřeno (release s debug-assertions, 144 Hz):* průměr 6,9–7,1 ms, medián 7,0, 1–4 snímky nad 10 ms ze ~150 (občasný 6–17ms snímek, nejspíš čekání na DWM), práce za snímek průměr 0,7–0,9 ms. Paměť: klid 2,7 MB, během nesení ~3,6 MB, špička při dekódování PNG tapet ~42 MB < 0,5 s, po skončení 2,8 MB (vč. kompozitoru). Exe 921 600 B.
  - *Ladění (debug):* `AUTOSPACES_NO_GLASS=1` (tmavá vrstva místo akrylu), `AUTOSPACES_OVERLAY_DEMO_ROW=1` (jen lišta), `AUTOSPACES_OSD_THEME=light|dark` (barvy lišty, skla a popisku).
- **Náhled při podržení Win+Ctrl+Shift (2026-11-24):** drží-li uživatel všechny tři klávesy (libovolné pořadí, levé i pravé) bez jiné klávesy `PREVIEW_MS` = 150 ms, ukáže se celý overlay pro okno v popředí (sklo, ostré okno, lišta se zvýrazněnou aktuální plochou) – nic se nemění. Šipka naváže nesením z už zobrazených vrstev (stejné okno → vrstvy zůstanou, jen se posune cíl; jiný způsob nebo jiné okno → nové vrstvy). Puštění kterékoli ze tří = vrstvy plynule zmizí. Jiná klávesa (Win+Ctrl+Shift+B, V, S, …) = náhled hned zmizí a až do puštění všech modifikátorů se znovu neukáže; klávesa sama jde do Windows beze změny (Raw Input nic neblokuje ani nezdržuje).
  - *Detekce:* Raw Input (`RegisterRawInputDevices`, klávesnice, `RIDEV_INPUTSINK` na skrytém okně) – kopie událostí asynchronně, i když je v popředí jiná aplikace, bez trvalého `WH_KEYBOARD_LL` (ten by zdržel každou klávesu v systému). Čte se jen druh klávesy (Win / Ctrl / Shift / šipka / jiná) a nahoru/dolů, nic se neukládá ani neloguje; vlastní vložená klávesa (`dwExtraInfo` = MARK) a falešný Shift ovladače (Shift s E0) se přeskakují. Registruje se spolu se zkratkou (jen se zapnutým přenášením a backendem), ruší se při vypnutí a ukončení. Okna spuštěná jako správce (UIPI) a zabezpečená plocha (UAC, zamčení) Raw Input nepošlou – tam náhled nebude, a taková okna nést nejde tak jako tak. Hook po dobu nesení (Esc spolknout, puštění modifikátorů) zůstává beze změny.
  - *Stav:* čistý stavový automat `core::carry::Chord` (testy: podržení → náhled, puštění → zmizení, šipka → nesení, jiná klávesa → okamžité zrušení a „zkažené“ podržení, pořadí, obě strany, autorepeat, prodleva). Časovač náhledu běží jen od stisku třetí klávesy do 150 ms; jemné hodiny (Instant), dřívější WM_TIMER se dočká.
  - *Kdy ne:* `Orchestrator::carry_preview` – jen když by nesení použilo overlay a okno v popředí jde přesunout (stejné kontroly jako nesení: ne plocha/hlavní panel/AutoSpaces/systémová okna, ne správce, ne naučená výjimka, `CanViewMoveDesktops`, na zobrazené ploše), ne celoobrazovková aplikace, ne během startovní sekvence/profilu, ne s vypnutým přenášením, ne ve vzdálené relaci (overlay se tam nenabízí), ne v režimu nepodporováno. „Toto okno nelze přesunout“ se ukazuje dál jen po šipce. Tapety se dekódují ve vlákně, vrstvy se ukážou hned (naměřeno 31 ms do zobrazení v debug buildu).
  - *Ladění (debug):* `AUTOSPACES_PREVIEW_DEMO=<složka>` – 3 s po startu ukáže náhled pro okno v popředí jako při podržení (bez kláves), snímek, zmizí, konec; `AUTOSPACES_RAW_PROBE=<soubor>` (+ `AUTOSPACES_RAW_PROBE_SECS`) – zaregistruje Raw Input i bez zkratky a při konci zapíše jen počet událostí.
- **Úprava po zkoušce 0.2.0 (zásek, prodleva, fokus):**
  - *Zásek animace – příčina:* během nesení se při každé šipce i při každé události okna (`after_orchestrator` → `carry_sync` → `carry_target`/`carry_desktops`, `carry_step`) volalo `GetDesktops` do Průzkumníka (meziprocesové COM na hlavním vlákně uprostřed animace; když je Průzkumník zaneprázdněný, snímek se zdrží). Teď si nesení seznam ploch vezme jednou na začátku (`CarryRun::desks`, nová plocha se připíše) – za celé nesení žádné volání ploch kromě vytvoření nové a přepnutí při upuštění. Snímky dál časuje `DwmFlush` a skutečný čas (Instant), ne `SetTimer`; neměnné náhledy se neaktualizují (skryté plochy jen při změně).
  - *Prodleva:* náhled po 70 ms (dřív 150) a vrstvy se staví hned, jak jsou dole všechny tři klávesy, neviditelně (`Begin::hidden`); po prodlevě jen `reveal` (objevení 70 ms, dřív 110). Šipka před uplynutím prodlevy vrstvy rovnou ukáže. Puštění nebo jiná klávesa před prodlevou je zahodí nevídané. Malé obrázky tapet se drží mezi neseními (cache až 6 obrázků, ~230 KB každý; klíč obsahuje čas souboru, změněný soubor se dekóduje znovu) – při dalším nesení jsou tapety hned.
  - *Fokus po upuštění:* po přepnutí plochy Windows chvíli poté aktivuje vlastní poslední okno cílové plochy (maximalizované okno pak zakrylo přenesené). Po upuštění (i po Esc, pro všechny způsoby) orchestrátor okno aktivuje hned a znovu za 150 a 500 ms (`focus_token`), jen pokud je na zobrazené ploše a popředí má jiné okno; přesune-li ho obecné pravidlo jinam, nechá ho tam. `activate` po `SetForegroundWindow` volá i `BringWindowToTop`. Testy: všechny čtyři způsoby × upuštění/Esc, okno přesunuté jinam.
  - *Neověřeno živě:* skutečné časy na PC uživatele (zkušební instance se ze sandboxu nespouští), chování fokusu se skutečným Průzkumníkem.

### 12.14 Minimum ploch je počet, ne ochrana konkrétních ploch (verze 0.2.1)
- **Hlášení uživatele:** rozložení ploch zapnuté, počet ploch po startu 3, na plochách 2 a 3 se spouštějí aplikace. Po zavření aplikace na ploše 2 (nebo 3) plocha zůstala prázdná a nově maximalizované okno (např. z plochy 1) dostalo novou plochu 4 místo prázdné 2/3; s plochami 4 a 5 zůstávala prázdná 3 jako mezera.
- **Příčina:** plochy profilu byly po prvním použití profilu natrvalo „připnuté“ (`pinned = N`, `is_pinned`): kompakce je nemazala ani jako mezeru a `find_empty` je přeskakovalo při výběru plochy pro maximalizované okno (pravidlo 12.7 „plochy z profilu jsou pevné“). Ochrana byla zavedena kvůli dřívější chybě, kdy kompakce smazala plochy profilu ještě během startu (12.11, „Příčiny chyb ze zkoušky“).
- **Nové pravidlo (zadání uživatele):**
  1. B (počet ploch při startu, s profilem `max(B, desktopCount)`, ruční přidání/odebrání plochy jako dosud) je jen **minimální počet**. Žádná konkrétní plocha kvůli němu nezůstává; prázdné plochy ponechané kvůli minimu jsou vždy **na konci** (invarianty 12.1 beze změny: plocha 1 nikdy, mezera se maže a následující se posunou, pod B se doplní na konec, nad B se prázdné na konci mažou).
  2. **Rezervace ploch profilu platí jen během startovní sekvence a běhu profilu** (`hold || startup_pending || layout.is_some()`): tehdy se plochy 2..N nemažou (kompakce stejně čeká, 12.11) a maximalizované okno je nedostane (jde za ně). Na konci běhu proběhne jedna kompakce – plocha aplikace, která se nespustila, se tím smaže jako mezera (pokud za ní je obsazená plocha) nebo zůstane prázdná na konci (do počtu B). „Použít teď“ se chová stejně (kompakce na konci jeho běhu). `pinned` dál slouží pravidlu „okno aplikace profilu na své ploše se při maximalizaci nepřesouvá“ (`on_profile_desktop`) a okna umístěná profilem (`profile_windows`) zůstávají, kde jsou.
  3. **Maximalizace:** `find_empty` (jediné místo výběru plochy, i pro startovní rozmístění) bere nejnižší prázdnou plochu kromě plochy 1, plochy, ze které okno odchází, a plochy jiného spravovaného okna; teprve když žádná není, vytvoří novou na konci. Přenášení okna (12.13) plochy nevybírá – jde na plochu, kam uživatel krokuje (nová jen „za poslední“ v režimu `carryCreateNew`), takže není co sjednocovat.
  4. **Prohlížená prázdná plocha** se kompakcí nezavírá, dokud ji uživatel neopustí – zavřením by ho aplikace nečekaně přepnula jinam (např. zavře poslední okno a chce na té ploše hned otevřít jiné; prodleva 300 ms na to nestačí). Výjimky (dosavadní pravidlo návratu): odchází-li z ní spravované okno (výstup, `hint`), plocha se smaže a pohled jde s oknem na jeho původní plochu; pomocná plocha spravovaného okna, které zaniklo bez události, se smaže s návratem na původní plochu okna (`remove_at`). Ponechaná plocha nastaví `need_compact`; opuštění plochy změní klíč `VirtualDesktops` v registru → `poll` → kompakce. Odchylka od 12.1 („jinak předchozí plocha v pořadí“): přepnutí na předchozí plochu se už nedělá.
  5. **Prázdná = bez skutečných oken:** beze změny `occupancy` (`real_windows`: okna aplikací i vyloučených a nespravovaných, minimalizovaná ano; systémová okna a okna připnutá na všechny plochy ne; plus plochy živých spravovaných oken). Plocha s okny jiné aplikace se nikdy nemaže.
  6. **Návrat na původní plochu** pracuje s GUID ploch (`ManagedSpace.origin`, `resolve_origin`, náhrobky při smazání původní plochy), takže posun čísel po smazání mezery cíl návratu nerozbije.
- **Příklad (minimum 3):** plochy 1–5 obsazené, zavřou se okna na 3 → plocha 3 se zavře, 4 → 3, 5 → 4. Pak se zavřou okna na 4 i 3 → plocha 4 se zavře, 3 zůstane prázdná. Maximalizace na ploše 1 pak jde na prázdnou 3, ne na novou 4.
- **Testy** (`core/tests.rs`): `minimum_three_freed_middle_desktop_is_closed_and_later_ones_shift`, `minimum_three_two_freed_desktops_close_only_down_to_the_minimum_and_stay_at_the_end`, `maximized_window_takes_the_empty_desktop_kept_for_the_minimum_not_a_new_one`, `profile_desktop_freed_after_the_run_is_reused_and_a_freed_middle_one_is_closed` (nahlášený případ), `return_to_the_origin_survives_the_renumbering_of_desktops`, `startup_with_a_profile_keeps_its_reserved_desktops_until_the_run_ends`; upravené: `viewed_desktop_removal_goes_to_origin_for_managed_exit_else_waits_until_left`, `layout_desktops_are_kept_while_the_profile_runs_and_compacted_at_its_end`, `startup_profile_app_that_never_starts_its_desktop_is_kept_during_the_run_and_closed_after`, `layout_main_window_after_splash_follows_it_no_extra_desktop`.
- **Neověřeno živě** (jen falešný backend): skutečné mazání/posun ploch v Průzkumníku, že změna zobrazené plochy spolehlivě vyvolá `poll` (předpoklad z 12.7: klíč se mění i při přepnutí pohledu), chování se skutečnými aplikacemi profilu.

### 12.15 Aktualizace (kontrola, stažení, instalace; v kódu od 0.2.1, poprvé použitelné s dalším vydáním)
- **Kdy:** první kontrola 10 minut po spuštění (start Windows se nezdržuje), pak jednou za 24 h, dokud aplikace běží; nepovedená kontrola (offline, GitHub nedostupný) se zopakuje za hodinu. Časovač `TIMER_UPDATE` je jednorázový a čeká nejvýš hodinu, pak se přepočítá podle hodin (`core::update::Timing`: `wait_secs`, `done`, `timer_ms`), takže spánek ani posun hodin kontrolu neztratí (hodiny vrácené o víc než den = kontrola hned). Platí jen pro běh, nic se kvůli tomu neukládá. Nastavení `"checkUpdates"` (výchozí `true`, tolerantní načítání, zapisuje se vždy), zaškrtávátko „Kontrolovat aktualizace“ / „Check for updates“ v nové skupině „Aktualizace“ / „Updates“ na záložce Obecné (stránka 106 i dialog 101 o 28/32 DLU vyšší). Položka nabídky ikony „Zkontrolovat aktualizace“ / „Check for updates“ zkontroluje hned (i při vypnutém nastavení) a oznámí i „Máte nejnovější verzi AutoSpaces (X.Y.Z).“ nebo chybu sítě (MessageBox); automatická kontrola chyby jen zapíše do logu činnosti (`update: …`, je-li zapnutý).
- **Síť:** jen WinHTTP (systémová DLL, feature `Win32_Networking_WinHttp`) na samostatném vlákně (`src/update.rs`, `start_check`); výsledek jde hlavnímu STA vláknu zprávou `WM_UPDATE_CHECKED` (`WM_APP + 50`, `Box` v lParam). Dotaz `GET https://api.github.com/repos/premekvibe/AutoSpaces/releases/latest` s hlavičkami `User-Agent: AutoSpaces/<verze>`, `Accept: application/vnd.github+json`, `X-GitHub-Api-Version: 2022-11-28`; žádné cookies (`WINHTTP_DISABLE_COOKIES`), žádný query string, žádné údaje o uživateli ani systému. Proxy podle systému (`WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY`), TLS 1.2/1.3, časové limity 15/30 s, odpověď API nejvýš 1 MB, instalátor nejvýš 64 MB. Parser JSON (`src/json.rs`) nově skládá náhradní páry `😀` a osamocenou náhradu mění na U+FFFD (emoji v textu vydání by jinak shodilo celé čtení).
- **Bezpečnost:**
  1. jen `https` na portu 443, bez `user@` v adrese (`core::update::parse_https`);
  2. přesměrování WinHTTP sám nesleduje (`WINHTTP_OPTION_REDIRECT_POLICY_NEVER`); kód je sleduje nejvýš 5× a jen na povolené hosty `github.com`, `api.github.com`, `*.githubusercontent.com` (dnes vede asset na `release-assets.githubusercontent.com`; `redirect_target`, `allowed_host`);
  3. `html_url` musí ležet pod `https://github.com/premekvibe/AutoSpaces/releases/` (otevírá se v prohlížeči), `browser_download_url` pod `…/releases/download/` a končit `/AutoSpaces-Setup-X.Y.Z.exe` téže verze jako tag;
  4. SHA-256 instalátoru se bere z **textu vydání** (formát níže) a počítá přes BCrypt (`BCRYPT_SHA256_ALG_HANDLE`, feature `Win32_Security_Cryptography`). Chybí-li řádek (nebo jsou pro soubor dva různé), nic se nestahuje a dialog nabídne jen stránku vydání; nesouhlasí-li součet, soubor se smaže, nic se nespouští a dialog to oznámí;
  5. ověřený soubor zůstane otevřený se sdílením jen pro čtení až do spuštění instalátoru (nelze ho mezitím vyměnit);
  6. žádné osobní údaje: jen název a verze aplikace v `User-Agent`.
  Kontrolní součet chrání proti poškození a podvržení na cestě nebo na CDN, ne proti napadenému účtu na GitHubu (text vydání i soubor by pak změnil útočník); instalátor zatím není podepsaný.
- **Formát vydání (povinný pro každé budoucí vydání):** text vydání (body) musí obsahovat řádek
  ``SHA-256 (AutoSpaces-Setup-X.Y.Z.exe): `<64 hex znaků>` ``
  s přesným názvem nahraného assetu (jako u vydání v0.2.0; velikost písmen hexu nevadí, zpětné apostrofy jsou volitelné). Dále: tag `vX.Y.Z` (právě tři čísla; `-beta` a podobné se ignorují), vydání ne draft a ne pre-release (aplikace je přeskočí; `/releases/latest` je stejně nevrací), asset `AutoSpaces-Setup-X.Y.Z.exe` (výstup `tools\build-installer.ps1`). Hash: `Get-FileHash dist\AutoSpaces-Setup-X.Y.Z.exe -Algorithm SHA256`. Vydání bez řádku se jen ohlásí s odkazem na stránku vydání.
- **Oznámení:** balónek `Shell_NotifyIcon` (`NIF_INFO`, `NIIF_USER | NIIF_LARGE_ICON` s ikonou aplikace; ve Windows 10/11 systémové oznámení, zůstává v centru oznámení) „Je k dispozici AutoSpaces X.Y.Z. Klikněte pro instalaci.“ / „AutoSpaces X.Y.Z is available. Click to install.“ (neinstalovaná kopie nebo vydání bez součtu: „… Klikněte pro podrobnosti.“ / „… Click for details.“). `NIN_BALLOONUSERCLICK` otevře dialog, jen když poslední balónek byl o aktualizaci (`BALLOON_UPDATE`; hlášení „nepodporovaná verze“ ho nuluje). Pravidla (`core::update::Notices::verdict`): automaticky se stejná verze ohlásí nejvýš jednou za běh; přeskočená verze („Tuto verzi přeskočit“, uloženo v `%LOCALAPPDATA%\AutoSpaces\update.json` jako `{"skipVersion":"X.Y.Z"}`) se automaticky neohlašuje nikdy, novější ano; ruční kontrola ukáže dialog vždy (i u přeskočené) a počítá se jako ohlášení v běhu.
- **Dialog** (šablona 110, `src/update_dlg.rs`): „Je k dispozici AutoSpaces X.Y.Z. Nainstalovaná verze je …“, odkaz „Co je nového ve verzi X.Y.Z“ (SysLink → `html_url`), poznámka (co instalace udělá; že pro všechny uživatele bude UAC; proč se instalace nenabízí), zaškrtávátko „Kontrolovat aktualizace“ (ukládá `"checkUpdates"`), tlačítka „Nainstalovat“ / „Tuto verzi přeskočit“ / „Později“. Nainstalovat: stahování v témže dialogu na dalším vlákně do `%TEMP%\AutoSpaces-update\` (složka se před stažením a při další automatické kontrole maže), ukazatel průběhu a procenta, „Zrušit“ přeruší (soubor se smaže). Chyba stažení = hláška, lze zkusit znovu. Neinstalovaná kopie nebo vydání bez součtu: tlačítko „Otevřít stránku vydání“.
- **Jak je aplikace nainstalovaná** (`core::update::install_kind`, data čte `update::install_kind`): rozhoduje složka běžícího exe. Přednost má záznam instalátoru – `InstallLocation` klíče `…\Uninstall\{30AB1B41-D258-4DAA-A410-484D902466A8}_is1` v HKCU (= pro uživatele) nebo HKLM (64bitový pohled, = pro všechny), pokud jmenuje přesně tuto složku; jinak pod `%LOCALAPPDATA%\Programs` = pro uživatele, pod Program Files (`ProgramW6432`, `ProgramFiles`, `ProgramFiles(x86)`) = pro všechny; jinak (např. `target\release`) neinstalovaná kopie.
- **Spuštění instalátoru** (`ShellExecuteExW`, sloveso `open`): `/SILENT /SUPPRESSMSGBOXES /NORESTART /CURRENTUSER /UPDATE`, resp. `/ALLUSERS` místo `/CURRENTUSER` (`PrivilegesRequiredOverridesAllowed=dialog` podle dokumentace Inno povoluje i příkazovou řádku). Jazyk instalátoru se zachová sám (`UsePreviousLanguage`, detekce podle UI). **Pro uživatele:** po úspěšném spuštění se AutoSpaces hned ukončí přes `exit_clean` (stejná cesta jako „Ukončit“ – vrátí okna a plochy). **Pro všechny uživatele – odchylka od zadání, ne `runas`:** instalátor spuštěný už elevovaný nemůže aplikaci na konci spustit jako původní uživatel (dokumentace Inno k `runasoriginaluser`), AutoSpaces by pak běžel jako správce. Instalátor si proto o UAC řekne sám; po souhlasu ho jeho `InitializeSetup` → `CloseRunningApp` zavře `WM_CLOSE` (opět `exit_clean`). Odmítne-li uživatel UAC, instalátor skončí a AutoSpaces běží dál beze změny. Nepodaří-li se instalátor spustit vůbec, hláška a AutoSpaces běží dál.
- **Instalátor** (`installer/autospaces.iss`): nová položka `[Run]` bez `postinstall`/`skipifsilent`, s `Check: IsAppUpdate` (tichý režim + parametr `/UPDATE`) a `runasoriginaluser`, spustí AutoSpaces po dokončení tiché aktualizace znovu. Ruční instalace se chová jako dřív (zaškrtávátko na konci; tichá instalace bez `/UPDATE` aplikaci nespouští). `AppMutex`, `CloseApplications=no` a `CloseRunningApp` beze změny.
- **Velikost:** WinHTTP a BCrypt jsou systémové DLL; exe 928 768 → 968 704 B (+39 kB, stále pod 1 MB).
- **Ladění (jen debug):** `AUTOSPACES_UPDATE_NOW=1` přeskočí 10 minut, `AUTOSPACES_UPDATE_JSON=<soubor>` odpoví na kontrolu ze souboru (bez sítě) – např. upravená odpověď s vyšší verzí pro vyzkoušení balónku a dialogu.
- **Testy** (`core/update.rs`, `update.rs`, `json.rs`, `settings.rs`): čtení skutečné (zkrácené) odpovědi v0.2.0, draft / pre-release / zvláštní tag, asset jiné verze nebo mimo repozitář, řádek SHA-256 (formát, jiný soubor, nejednoznačnost), číselné porovnání verzí, hosty a přesměrování, per-user / all-users / neinstalováno podle registru a cesty, příkazová řádka instalátoru, ohlášení jednou za běh + přeskočení + `update.json`, časování 10 min / 24 h / 1 h, BCrypt SHA-256 na známých vektorech, náhradní páry v JSON, `checkUpdates`. Žádný test nejde na síť.
- **Neověřeno:** skutečná aktualizace (balónek a klik i z centra oznámení, stažení přes přesměrování, tichá instalace, opětovné spuštění, cesta přes UAC i jeho odmítnutí) jde vyzkoušet až s dalším vydáním novějším než 0.2.1, které bude mít řádek SHA-256 (a bude mít v instalátoru položku `/UPDATE`). Ručně ověřeno jen: API odpovídá (curl s `User-Agent: AutoSpaces/0.2.1`), asset přesměrovává na `release-assets.githubusercontent.com`, BCrypt SHA-256 souboru `dist\AutoSpaces-Setup-0.2.0.exe` = hodnota v textu vydání v0.2.0, skutečná odpověď API se přečte celá. Předpoklad podle dokumentace Inno, živě neověřený: s `/ALLUSERS` se instalátor elevuje dřív, než spustí `InitializeSetup` (při odmítnutí UAC se AutoSpaces nezavře).

### 12.16 Stav okna u aplikací v rozložení ploch (verze 0.2.2)
- **Zadání uživatele:** u každé aplikace rozložení zvolit, zda se okno na své ploše otevře maximalizovaně (výchozí, chování do 0.2.1), v okně, nebo minimalizovaně. Navržené zaškrtávátko nestačí (stavy jsou tři), proto rozbalovací seznam.
- **Dialog Rozložení ploch:** pod seznamem aplikací „Stav okna vybrané aplikace:“ / „Window state of the selected app:“ s volbami „Maximalizovaně / V okně / Minimalizovaně“ (EN „Maximized / Normal window / Minimized“); bez vybrané položky zašedlé. Stav je i v textu řádku: „Plocha 2 – Outlook (olk.exe) – v okně“ (`LayoutRow` má třetí zástupný znak; texty `ShowMax…`, `ShowMaxRow…` v `i18n.rs`). Seznam je o 18 DLU nižší (94 → 76), dialog má stejnou velikost.
- **settings.json:** pravidlo má `"show": "max" | "normal" | "min"` (`WindowShow`, zapisuje se vždy); chybějící nebo neznámá hodnota = `max`, takže soubory z 0.2.1 se chovají jako dřív. „Převzít současný stav“ převezme stav prvního okna aplikace na dané ploše (minimalizované → `min`, normální → `normal`, maximalizované / celoobrazovkové → `max`).
- **Provedení (`Orchestrator::layout_show`, dřív `layout_maximize`):** spuštěná i už běžící aplikace se po přesunu na svou plochu nastaví do stavu pravidla, bez aktivace a bez přepnutí pohledu (beze změny 12.7: nejdřív se vrátí fokus uživateli, pak změna stavu). Okno, které už ve stavu je, se nechá být. Když si aplikace obnoví vlastní umístění, stav se vnutí znovu nejvýš 3× (jako dřív maximalizace). Win32 (`WindowSystem::show_inactive`):
  - *maximalizovaně* – beze změny (`maximize_inactive`);
  - *v okně* – `SetWindowPlacement` se `showCmd = SW_SHOWNOACTIVATE` a `WPF_ASYNCWINDOWPLACEMENT` bez `WPF_RESTORETOMAXIMIZED` (minimalizované i maximalizované okno se vrátí na normální polohu, asynchronně ve vlákně okna);
  - *minimalizovaně* – klasické aplikace se spouštějí minimalizované (`SW_SHOWMINNOACTIVE`), tam se nedělá nic; jinak `PostMessage(WM_SYSCOMMAND, SC_MINIMIZE)` – okno se minimalizuje samo, jako po kliknutí na tlačítko. Z procesu AutoSpaces tak dál nejde žádné volání minimalizace (12.7: mod animace minimalizace by do procesu nahrál Direct3D). Okno není v popředí (fokus se vrátil uživateli), takže se nic jiného neaktivuje. Aplikace, která se místo minimalizace schová do oznamovací oblasti, se tím stane skrytou a na ploše se nepočítá.
- **Souhra s orchestrátorem:** okno v okně nebo minimalizované nic nespouští a nepočítá se jako maximalizované (`shares_desktop` bere jen viditelná maximalizovaná / celoobrazovková okna) – maximalizace jiného okna vedle něj na téže ploše proto nevytvoří plochu navíc. Maximalizuje-li uživatel okno aplikace profilu na její ploše, zůstává tam (`on_profile_desktop`, beze změny). **Obsazenost:** plocha je obsazená i minimalizovaným oknem – `app_windows` (filtr `is_candidate`) bere viditelná okna a minimalizované okno má `WS_VISIBLE` dál; `occupancy` počítá okna všech aplikací (i mimo profil, i vyloučených). Plocha z rozložení s aplikací v okně nebo minimalizovanou se tedy nezavře, dokud tam okno je; ověřeno testy, i pro minimalizované okno cizí aplikace na běžné ploše (plán 12.1 „obsazená plocha“ to tak definoval už dřív).
- **Testy** (`core/tests.rs`, `core/settings.rs`): `layout_normal_and_minimized_apps_get_their_state_in_the_background`, `layout_window_already_in_its_state_is_left_alone_and_restores_are_not_fought_forever`, `normal_or_minimized_profile_window_triggers_nothing_and_keeps_its_desktop`, `minimized_window_of_any_app_occupies_its_desktop`, `capture_layout_takes_the_window_state_along`, `window_show_of_layout_rules_defaults_to_maximized`.
- **Neověřeno živě:** chování skutečných aplikací (Outlook, TradingView) ve stavu „v okně“ a „minimalizovaně“, zejména aplikací, které po spuštění samy obnovují vlastní umístění nebo na `SC_MINIMIZE` reagují jinak.

### 12.17 Tapety ploch (verze 0.2.2)
- **Rozhodnutí uživatele po zkoušce 0.2.2 (2026-11-27): náhodný výběr ze složky je z produkční verze odstraněn.** Log činnosti potvrdil, že AutoSpaces při přepínání ploch žádnou tapetu nenastavuje (jen „Zamíchat teď“ a přiřazení po OK); problikávání (překreslení celé plochy) při přepnutí mezi plochami s různými tapetami dělá Windows 11 (sestavení 26340). Kód náhodného výběru (výběr obrázků, `wallpapers.json`, reakce na nové plochy, „Zamíchat teď“, oprava dvojitého míchání po „Zamíchat teď“ + OK) je odložený ve větvi `feature/random-wallpapers`. V `main` zůstává jen „Neměnit tapety“ (výchozí) a „Stejný obrázek na všech plochách“: nastaví se jen po OK se změněnou volbou, přes `UpdateWallpaperPathForAllDesktops` (při selhání `SetDesktopWallpaper` jen u ploch, které obrázek ještě nemají), bez podpory ploch dokumentovaně; plochy, které obrázek už evidují (registr, bez ohledu na velikost písmen), se nenastavují znovu. Na nové plochy se nereaguje (při volání pro všechny plochy je předpoklad, že Windows použije obrázek i pro nové – neověřeno). `settings.json` s `"mode": "random"` (a klíči `folder`, `subfolders`, `newDesktops`) se načte jako „Neměnit“ a ostatní klíče se zahodí (test `random_wallpapers_of_the_test_build_read_as_do_not_change`). Záložka Tapety má jen dva přepínače, pole Obrázek a poznámku. Sloty `SetDesktopWallpaper` a `UpdateWallpaperPathForAllDesktops` ve `variants.json` zůstávají (používá je „stejný obrázek“). Exe po odstranění 984 064 B (opět pod 1 MB). Popis níže je stav 0.2.2 před odstraněním; části o náhodném výběru platí jen pro větev.
- **Zadání uživatele:** v aplikaci zvolit (1) jeden obrázek pro všechny plochy, nebo (2) složku, ze které AutoSpaces náhodně volí tapetu jednotlivým plochám.
- **Nastavení:** nová záložka „Tapety“ / „Wallpapers“ (šablona 111 – 110 je dialog aktualizace; stránky se teď berou z tabulky `PAGE_DIALOGS`; stejná výška jako ostatní stránky, 178 DLU), skupina „Tapety ploch“: přepínač „Neměnit tapety (spravují je Windows)“ (výchozí) / „Stejný obrázek na všech plochách“ + pole Obrázek a „Procházet…“ (`GetOpenFileNameW`, filtr .jpg .jpeg .png .bmp) / „Náhodně ze složky (každá plocha jiný obrázek)“ + pole Složka a „Procházet…“ (`SHBrowseForFolderW`, feature `Win32_UI_Shell_Common`), „Včetně podsložek“, „Při vytvoření nové plochy jí dát náhodnou tapetu“ (výchozí zapnuto), tlačítko „Zamíchat teď“ a pod tím poznámka, co která volba na tomto Windows dělá, a že po ukončení tapety zůstanou. Pole nezvolené volby jsou zašedlá. OK s volbou „stejný obrázek“ bez existujícího obrázku nebo „náhodně“ se složkou bez obrázků dialog nezavře (hláška, záložka a pole s fokusem).
- **settings.json:** `"wallpapers": {"mode": "off" | "same" | "random", "image": "", "folder": "", "subfolders": false, "newDesktops": true}`, tolerantní čtení (chybějící klíč = „neměnit“). Po OK se volba použije hned, jen když se změnila (`Wallpapers::changed_from`: režim, u „same“ obrázek, u „random“ složka a podsložky; samotná volba pro nové plochy nic nemění). „Neměnit“ na nic nesahá (jen zapomene záznam rozdaných obrázků). V diagnostickém souboru jsou `image` a `folder` nahrazené „(omitted)“.
- **Jak Windows nastavuje tapety (zjištěno):**
  - Windows 11 má tapetu pro každou plochu (od 21H2). Explorer ji ukládá do `HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\VirtualDesktops\Desktops\{GUID}\Wallpaper`; zápis do registru by se neprojevil (Explorer ho jen zapisuje), nastavuje se nedokumentovaným `IVirtualDesktopManagerInternal::SetDesktopWallpaper(IVirtualDesktop*, HSTRING)` a `UpdateWallpaperPathForAllDesktops(HSTRING)` – v každém zdroji hned za `SetDesktopName`, bez parametru HMONITOR i u variant s ním.
  - Dokumentované `IDesktopWallpaper::SetWallpaper(NULL, path)` (CLSID_DesktopWallpaper `C2CF3110-460E-4FC1-B9D0-8A1C0C9CC4BD`, IID `B92B56A9-8B55-4E14-9A89-0199BBB6F93B`, první metoda = slot 3) nastaví obrázek na všech monitorech. Dokumentace neříká nic o virtuálních plochách; podle zpráv uživatelů (Neowin k sestavení 25197, 2022) mění nastavení ve Windows 11 jen aktuální plochu – **živě neověřeno** (zadání: žádné živé testy, které mění tapety). Proto „na všechny plochy“ jde přednostně přes `UpdateWallpaperPathForAllDesktops`, pak přes `SetDesktopWallpaper` pro každou plochu, a dokumentované volání jen tam, kde sloty nejsou (Windows 10, nebo neznámé sestavení); po každém nastavení se do logu zapíše, na kolika plochách registr ukazuje zvolený obrázek.
  - Windows 10 tapety jednotlivých ploch nemá: jen dokumentované volání pro všechno.
- **Sloty (`variants.json`, nové volitelné sloty `SetDesktopWallpaper`, `UpdateWallpaperPathForAllDesktops`, u každé varianty i pole `wallpaperSource`):** jen u variant, kde se shodují aspoň dva nezávislé zdroje (čteno na webu 2026-11-27, nic staženo ani spuštěno):
  | varianta | SetDesktopName | SetDesktopWallpaper | UpdateWallpaperPathForAllDesktops | zdroje |
  |---|---|---|---|---|
  | `win11-24h2` | 16 | 17 | 18 | MScholtes/VirtualDesktop `VirtualDesktop11-24H2.cs`; Ciantic/VirtualDesktopAccessor `src/interfaces.rs` (`set_name`, `set_wallpaper`, `update_wallpaper_for_all`); mrob95/pyvda `com_defns.py` větev `OVER_26100` (`SetName`, `SetWallpaper`, `SetWallpaperForAllDesktops`) |
  | `win11-23h2-3085` | 15 | 16 | 17 | MScholtes `VirtualDesktop11.cs` commit 4dd34ce (v1.17); Ciantic commit 0276add; pyvda `OVER_22631` |
  | `win11-22h2-2215` | 15 | 16 | 17 | MScholtes `VirtualDesktop11-23H2.cs` commit 546b673; Ciantic commit c918946; pyvda `OVER_22621` |
  | `win11-22h2` | 16 | 17 | 18 | MScholtes `VirtualDesktop11.cs` commit 546b673; pyvda `OVER_22449` |
  | `win11-21h2` | 15 | 16 | 17 | MScholtes `VirtualDesktop11-21H2.cs` commit 546b673; pyvda `OVER_21313` |
  | `win10` | – | – | – | Windows 10 metody nemá (MScholtes `VirtualDesktop.cs`, pyvda větev 9000) |
  Ve všech zdrojích je cesta `HSTRING` a pořadí stejné jako u ostatních slotů variant (které už ověřuje sebetest). `SetDesktopName` se v AutoSpaces dál nevolá.
- **Bezpečnost nedokumentovaných setterů:** sebetest je jen pro čtení a nastavovací metody nevolá (`--selftest --select-only` jen vypíše, které sloty varianta popisuje). Ověřit je bez změny tapety nejde, proto: (1) volají se jen na variantě, která prošla sebetestem (`InternalBackend` jinak nevznikne); (2) `variants.json` s jinými sloty se odmítne – `SetDesktopWallpaper` musí být `SetDesktopName + 1`, `UpdateWallpaperPathForAllDesktops` `SetDesktopName + 2`, oba pod `SwitchDesktopWithAnimation`, druhý jen s prvním (test `wallpaper_slots_from_two_sources_on_windows_11_only`); (3) volání běží v Exploreru, špatný slot by shodil Explorer, ne AutoSpaces – vrátí-li volání chybu „server zmizel“ (RPC), settery se v tomto běhu vypnou a do `selftest-guard.txt` se zapíše `crashed wall:<varianta> <build>`, na tomto sestavení se už nezkoušejí (`Guard::record_crash`). Varianta `win11-24h2` má otevřený rozsah sestavení: budoucí sestavení, které změní pořadí jen za `FindDesktop`, by sebetest neodhalil – pak zafunguje bod (3).
- **Co funguje kde:**
  - *Windows 11 se známou variantou* (24H2+, 23H2, 22H2, 21H2 – mimo 24H2 neověřené, viz 12.5): „stejný obrázek“ přes `UpdateWallpaperPathForAllDesktops`; „náhodně“ = každá plocha svůj obrázek přes `SetDesktopWallpaper`; nová plocha dostane náhodný obrázek (při „stejném obrázku“ ten obrázek).
  - *Windows 11 bez známých slotů* a *Windows 10*: „stejný obrázek“ dokumentovaně; „náhodně“ = **jeden** náhodný obrázek pro všechny plochy (rozhodnutí: volbu neskrývat, poznámka v dialogu to říká; „Zamíchat teď“ vybere jiný); nové plochy nic nepotřebují.
  - *Režim „nepodporováno“*: dialog Nastavení se neotevírá (jako dosud), tapety se nemění.
- **Náhodný výběr (`core/wallpaper.rs`, čisté funkce):** obrázky `.jpg .jpeg .png .bmp` (velikost písmen nerozhoduje) ze složky, volitelně i z podsložek (nejvýš 8 úrovní, nejvýš 5 000 souborů), seřazené. `choose`: plochy se procházejí v pořadí; obrázek se liší od obrázků sousedních ploch, pokud je z čeho vybírat, a od dosavadního obrázku plochy (aby „Zamíchat“ něco změnilo); mezi povolenými vyhrává nejméně použitý (každý obrázek jednou, než se jakýkoli zopakuje), shoda se losuje (xorshift64*, seed přes splitmix64; v aplikaci z hodin a PID, v testech pevný). Rozdané obrázky se pamatují **podle GUID plochy** v `%LOCALAPPDATA%\AutoSpaces\wallpapers.json` (`{"assigned": [{"desktop": "GUID", "wallpaper": "cesta"}]}`, nejvýš 200, zaniklé plochy se zapomenou), takže posun čísel ploch tapetu nezmění (tapetu drží i Windows podle GUID).
- **Nové plochy:** po každé události (`after_orchestrator` → `wallpaper::sync`) se porovná seznam ploch orchestrátoru (`known_desktops`, bez volání do Exploreru) s posledním známým. Plocha vytvořená AutoSpaces (maximalizace, kompakce, profil, nová plocha při přenášení) se v seznamu objeví hned; ruční přidání změní klíč `VirtualDesktops` v registru → `poll` → `absorb`. Nová plocha při „náhodně“ se zapnutou volbou dostane náhodný obrázek odlišný od souseda; při startu AutoSpaces i plochy, které od AutoSpaces obrázek nikdy nedostaly (vznikly, když neběžel). Během přenášení okna se nic nenastavuje (overlay ukazuje tapety), dožene se to po upuštění.
- **„Zamíchat teď“:** Explorer volání COM z obsluhy tlačítka odmítá (0x8001010D, 12.12), proto tlačítko uloží volbu z dialogu (i neuloženou) a pošle hlavnímu oknu `WM_WALL_SHUFFLE` (`WM_APP + 60`); tam se všem plochám rozdají nové obrázky. Během přenášení se zprávě nevyhoví.
- **Overlay přenášení (12.13):** cesty tapet se čtou z registru na začátku každého nesení a klíč cache malých obrázků obsahuje čas souboru, takže nová tapeta se ukáže vždy; po každém nastavení tapety se cache navíc vyprázdní (`overlay_ui::wallpapers_changed`), aby nezůstávaly obrázky, které už nikdo nepotřebuje, a nevadila ani výměna souboru ve stejné sekundě.
- **Ukončení:** AutoSpaces tapety po ukončení **nevrací** (jsou to teď uživatelova nastavení, plochy, které při ukončení zaniknou, tapetu stejně ztratí). Dialog to říká v poznámce.
- **Denní střídání** („Měnit náhodné tapety každý den“) se nedělá – potřebovalo by časovač a uložené datum posledního střídání; lze doplnit později.
- **Velikost:** exe 968 704 → 1 012 736 B (+44 kB za obě funkce; poprvé nad 1 MB). Instalátor `dist\AutoSpaces-Setup-0.2.2.exe` 2 894 699 B.
- **Testy:** `core/wallpaper.rs` (přípony, výpis složky s podsložkami i bez, dost obrázků = žádné opakování, málo obrázků = rovnoměrně a sousedé různí, stejný seed = stejný výběr, rovnoměrnost losu na 4 000 tazích, nová plocha vedle souseda, zamíchání mění obrázek, přiřazení podle GUID nezávislé na pořadí, jeden společný obrázek bez podpory ploch, ukládání a limit záznamů), `core/settings.rs` (`wallpapers_default_off_tolerant_and_roundtrip`, `wallpaper_changes_that_need_applying`), `desktops/variants.rs` (sloty a jejich konzistence), `report.rs` (`wallpaper_paths_are_left_out`).
- **Hlášení po zkoušce 0.2.2 (problik při přepínání ploch v režimu „náhodně“):** uživatel vidí při každém přepnutí „refresh“ celé plochy (ne jinou tapetu). Rozbor kódu: setter se volá jen z `apply` (OK v nastavení se změněnou volbou, „Zamíchat teď“) a ze `sync`, a `sync` končí hned, když se seznam ploch orchestrátoru nezměnil – přepnutí plochy (změna klíče `VirtualDesktops` → `poll`) seznam nemění, takže už 0.2.2 při přepnutí nic nevolala; globální `IDesktopWallpaper::SetWallpaper` se při podpoře tapet ploch nevolá vůbec a do registru tapet AutoSpaces nikdy nezapisuje. Nejpravděpodobnější příčina je proto chování Windows 11 s různými tapetami ploch: Explorer při přepnutí nastaví tapetu cílové plochy a překreslí celou plochu (u tohoto PC JPEG 1,6–3,5 MB, styl Vyplnit). **Neověřeno** – rozhodne záznam činnosti (každé volání setteru zapíše řádek `wallpaper: …`; při přepínání nesmí žádný přibýt) a srovnání s tapetami nastavenými ručně v zobrazení úloh (pravým tlačítkem na plochu → Zvolit pozadí).
  Zpevnění: (1) před každým voláním se porovná obrázek, který Explorer u plochy eviduje (`Desktops\{GUID}\Wallpaper`, bez ohledu na velikost písmen; u jedné tapety `Control Panel\Desktop\WallPaper`), a když souhlasí, nic se nevolá (i volání se stejnou cestou by plochu překreslilo); (2) „stejný obrázek“ nastavuje jen plochy, které ho ještě nemají; (3) logika je v `wallpaper::Host` nad rozhraním `WallSys` a testy s falešným Explorerem počítají volání: přepnutí a jakékoli další události = 0 volání (i po restartu AutoSpaces), nová plocha = 1, „Zamíchat teď“ = N, během přenášení 0 a po upuštění 1, „neměnit“ = 0, bez podpory ploch jen dokumentované volání a stejný obrázek ne dvakrát. Smyčka vlastní zápis → upozornění → další zápis nemůže vzniknout: na změnu registru se settery nevolají.
- **Neověřeno živě** (žádné živé testy, které mění tapety): že sloty skutečně nastaví tapetu (ani na 26340), zda `IDesktopWallpaper::SetWallpaper` ve Windows 11 mění jednu nebo všechny plochy, chování dialogu výběru složky a nové záložky na vysokém DPI, výkon u velkých složek na síťovém disku (výpis běží v hlavním vlákně), celá funkce na Windows 10.
