<p align="center"><img src="assets/icon-256.png" width="96" alt="Ikona AutoSpaces"></p>

# AutoSpaces

[English](README.md) · **Česky**

AutoSpaces je malá aplikace pro Windows 10 a 11, která běží v oznamovací oblasti a dává maximalizovaným oknům vlastní virtuální plochu, podobně jako macOS dává celoobrazovkovým aplikacím vlastní Space.

Windows virtuální plochy mají, ale vytvářet je, přesouvat na ně okna a uklízet je musíte ručně. AutoSpaces to dělá za vás: maximalizujete druhé okno a přesune se na novou plochu; zmenšíte ho nebo zavřete a jste zpátky tam, odkud jste přišli, a plocha navíc zmizí.

## Jak se chová

- **Jedno maximalizované okno na plochu.** Okno, které je na své ploše jediné maximalizované, zůstává na místě. Když na téže ploše maximalizujete další okno, přesune se na vlastní plochu a vy s ním.
- **Uklízí po sobě.** Po zmenšení, minimalizaci nebo zavření okna se vrátíte na plochu, ze které přišlo, a jeho plocha se odstraní.
- **Vaše plochy zůstávají vaše.** Počet ploch, které jste měli při spuštění AutoSpaces (nebo který je nastavený v rozložení ploch), se drží jako minimální počet. Plocha 1 se nikdy neodstraní. Prázdná plocha mezi obsazenými se zavře a následující se posunou, aby nevznikaly mezery; prázdné plochy ponechané kvůli minimu jsou vždy na konci a nově maximalizované okno je použije dřív, než vznikne nová plocha. Plocha, na kterou se právě díváte, se nezavře, dokud ji neopustíte.
- **Ukončení vše vrátí.** *Ukončit* v nabídce ikony vrátí přesunutá okna na původní plochy a odstraní plochy, které AutoSpaces přidal.

## Funkce

- **Režim spouštění:** všechna maximalizovaná okna, nebo jen celoobrazovkový režim (například F11 v prohlížeči).
- **Vyloučené aplikace:** z běžících aplikací vyberete ty, které se nemají přesouvat nikdy.
- **Naučené výjimky:** aplikaci, jejíž okna se opakovaně nepodaří přesunout, si AutoSpaces zapamatuje a nechá ji být. Seznam je vidět a jde upravit.
- **Rozmístění při startu:** okna, která jsou při spuštění AutoSpaces už maximalizovaná, se rozdělí na samostatné plochy, pokud máte všechna okna na jediné ploše.
- **Rozložení ploch:** zvolíte aplikace, které se mají při startu AutoSpaces otevřít na vlastních plochách (plocha 2 – pošta, plocha 3 – grafy, …). Každá aplikace se otevře maximalizovaně (výchozí), v okně nebo minimalizovaně, podle toho, co jí v seznamu zvolíte. Aplikace, která už běží, se nespouští znovu, jen se přesune a nastaví do zvoleného stavu. Spouštění probíhá na pozadí, bez přepnutí vaší plochy. *Převzít současný stav* převezme i stav oken.
- **Přenášení okna:** podržte **Win+Ctrl+Shift** a pozadí za aktivním oknem se změní v matné sklo, dole se objeví pás živých náhledů ploch. Šipkou **←/→** přenesete okno na předchozí nebo další plochu: zůstává ostré a na místě, zatímco plochy i s tapetami pod ním ujíždějí. Puštěním kláves okno na ploše necháte (zůstane v popředí a s fokusem), klávesou **Esc** ho vrátíte zpět. Za poslední plochou se pokračuje plochou 1, nebo (volba v *Nastavení → Klávesy*) vznikne nová plocha. Platí stejná pravidla jako při maximalizaci: maximalizované okno přenesené na plochu, kde už jedno maximalizované je, se přesune na volnou plochu.
- **Tapety:** v *Nastavení → Tapety* necháte tapety na Windows (výchozí), dáte jeden obrázek na všechny plochy, nebo necháte AutoSpaces vybrat každé ploše náhodný obrázek ze složky (.jpg, .jpeg, .png, .bmp; sousední plochy dostanou podle možností různé obrázky, výběr se pamatuje pro každou plochu). Náhodnou tapetu mohou dostávat i nové plochy a *Zamíchat teď* rozdá nové. Obrázek pro každou plochu zvlášť vyžaduje Windows 11; ve Windows 10 (a v sestaveních Windows 11, která AutoSpaces nezná) dá náhodný výběr všem plochám jeden společný obrázek. Po ukončení AutoSpaces původní tapety nevrací.
- **Automatické aktualizace:** 10 minut po spuštění a pak jednou denně se AutoSpaces zeptá GitHubu, zda nevyšla novější verze. Pokud ano, nabídne ji oznámením. *Nainstalovat* stáhne instalátor, ověří ho podle kontrolního součtu SHA-256 zveřejněného u vydání a nainstaluje ho na pozadí: AutoSpaces vrátí okna a plochy, aktualizuje se a znovu spustí. *Zkontrolovat aktualizace* v nabídce ikony udělá totéž na požádání; verzi lze také přeskočit.
- **Spouštění po startu Windows.**
- **Čeština a angličtina** podle jazyka Windows, s možností ručního přepnutí v nastavení.
- **Nenáročnost:** jediný soubor kolem 1 MB, asi 3 MB vlastní paměti, reaguje na události a na nic se nedotazuje ve smyčce.

Nastavení je v `%APPDATA%\AutoSpaces\settings.json`; naučené výjimky a stav v `%LOCALAPPDATA%\AutoSpaces`.

**Síť:** AutoSpaces se připojuje jen ke GitHubu, a to jen kvůli kontrole aktualizací (a po volbě *Nainstalovat* kvůli stažení instalátoru). Dotaz obsahuje jen název a verzi aplikace; žádné údaje o vás ani o počítači se neodesílají. Automatickou kontrolu lze vypnout v *Nastavení → Obecné → Kontrolovat aktualizace*.

## Podporované verze Windows

| Windows | Stav |
|---|---|
| Windows 11 24H2 a novější (sestavení 26100+) | Vyzkoušeno |
| Windows 11 21H2, 22H2, 23H2 | Zahrnuto, zatím nevyzkoušeno na skutečném systému |
| Windows 10 1809–22H2 | Zahrnuto, zatím nevyzkoušeno na skutečném systému |

Windows nenabízejí veřejné rozhraní pro přesun okna cizí aplikace mezi virtuálními plochami, proto AutoSpaces používá vnitřní rozhraní systému. To se mezi verzemi Windows liší a může se změnit aktualizací. Kvůli bezpečnosti si ho AutoSpaces při startu ověří voláními, která jen čtou, a porovná výsledek s registrem; teprve pak ho začne používat. Když ověření neprojde, AutoSpaces nic nepřesouvá, oznámí, že tato verze Windows není podporována, a v nabídce ikony nabídne *Nahlásit problém…*.

## Instalace

Spusťte `AutoSpaces-Setup-<verze>.exe`. Ve výchozím nastavení se instaluje pro aktuálního uživatele bez práv správce; instalace pro všechny uživatele je volitelná. Instalátor zatím není podepsaný, takže Windows SmartScreen zobrazí varování: zvolte *Další informace → Přesto spustit*.

Po instalaci najdete AutoSpaces v oznamovací oblasti. Pravým tlačítkem na ikonu otevřete nabídku: *Zapnuto*, *Nastavení…*, *Naučené výjimky*, *Zkontrolovat aktualizace*, *O aplikaci…* a *Ukončit*.

**Tip:** aby vás klik na ikonu běžící aplikace na hlavním panelu přenesl na její plochu, otevřete ve Windows *Nastavení > Systém > Multitasking > Plochy* a volbu *Na hlavním panelu zobrazovat všechna otevřená okna* nastavte na *Na všech plochách*. Shift + klik pak spustí novou instanci.

## Sestavení ze zdrojových kódů

Potřebujete Rust (MSVC) a Windows SDK (kvůli `rc.exe`); pro instalátor Inno Setup 6.

```
cargo build --release
cargo test
powershell -ExecutionPolicy Bypass -File tools\build-installer.ps1
```

Aplikace vznikne v `target\release\autospaces.exe`, instalátor ve složce `dist\`. Jedinou závislostí je crate `windows-sys`.

Užitečné přepínače: `--selftest --select-only` (kontrola rozhraní ploch jen čtením), `--restore` (vrátí okna a srovná plochy), `--quit` (požádá běžící instanci o korektní ukončení).

## Známá omezení

- Úvodní obrazovka spouštěné aplikace může krátce probliknout na vaší ploše, než se přesune; Windows vytvářejí nová okna vždy na ploše, na kterou se díváte.
- Windows hlásí minimalizaci asi o 100 ms později, takže návrat z minimalizovaného okna má krátké zpoždění.
- Některé aplikace mezi plochami přesunout nejde vůbec; skončí mezi naučenými výjimkami.

## Autor

Přemysl Vavroušek · [www.premysl-vavrousek.cz](https://www.premysl-vavrousek.cz)

Pokud vám AutoSpaces pomáhá, v dialogu *O aplikaci…* najdete možnosti, jak podpořit jeho vývoj.

## Licence

AutoSpaces je svobodný software pod licencí [GNU General Public License verze 3.0 nebo novější](LICENSE). Na název, ikonu a platební údaje autora se licence nevztahuje; viz [NOTICE.md](NOTICE.md).

## Nahlášení problému

Založte prosím hlášení na [github.com/premekvibe/AutoSpaces/issues](https://github.com/premekvibe/AutoSpaces/issues). Nabídka ikony → *Nahlásit problém…* vytvoří diagnostický soubor (`report.txt`, bez titulků oken a bez vašeho jména), který můžete přetáhnout do hlášení.
