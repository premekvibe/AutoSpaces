# Licence prostředků (assets)

## Logo Bitcoin / Lightning v dialogu „O aplikaci“

Nejde o soubor v assets: logo (oranžový kruh s bílým bleskem) kreslí kód přímo přes GDI
(`draw_logo` v `src/about_dlg.rs`), takže nic nezvětšuje exe.

- Zdroj: vlastní kresba autora projektu (jednoduché geometrické tvary: kruh a sedmiúhelník
  blesku), žádný převzatý obrázek ani písmo.
- Licence: stejná jako zbytek projektu; kresbu lze volně použít (CC0).
- Barva kruhu #F7931A je jen barva a není chráněna.
- Nepoužito: oficiální logo Bitcoinu (bitcoin.org / Wikimedia Commons) ani logo Lightning Labs;
  licenci u zdroje se nepodařilo v rámci této práce ověřit, proto je nahrazuje vlastní kresba.

## Ostatní

- `autospaces.ico`, `icon-*.png`, `icon-source.jpg`: ikona aplikace (viz `tools/make-icon.ps1`).
- `donate-qr.png`, `donate-revolut-source.jpg` (a z nich vytvořené `donate-*.bmp`): platební QR
  kódy autora aplikace. Názvy a barvy bank a služeb v nich slouží jen k označení způsobu platby.
