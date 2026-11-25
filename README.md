<p align="center"><img src="assets/icon-256.png" width="96" alt="AutoSpaces icon"></p>

# AutoSpaces

**English** · [Česky](README.cs.md)

AutoSpaces is a small tray utility for Windows 10 and 11 that gives maximized windows their own virtual desktop, the way full-screen apps get their own Space on macOS.

Windows has virtual desktops, but you have to create them, move windows onto them and clean them up by hand. AutoSpaces does that for you: maximize a second window and it moves to a fresh desktop; restore or close it and you are back where you started, with the extra desktop gone.

## How it behaves

- **One maximized window per desktop.** A window that is the only maximized one on its desktop stays put. When you maximize another window on the same desktop, that window moves to a desktop of its own and you follow it.
- **It cleans up after itself.** When the window is restored, minimized or closed, you return to the desktop it came from and its desktop is removed.
- **Your desktops stay yours.** The number of desktops you had when AutoSpaces started (or the number set in the desktop layout) is kept as a minimum count. Desktop 1 is never removed. An empty desktop between occupied ones is closed and the following ones move up, so there are no gaps; empty desktops kept for the minimum are always the last ones, and a newly maximized window uses them before a new desktop is created. A desktop you are looking at is not closed until you leave it.
- **Quitting puts everything back.** *Quit* in the tray menu returns moved windows to their original desktops and removes the desktops AutoSpaces added.

## Features

- **Trigger mode:** all maximized windows, or full-screen windows only (for example F11 in a browser).
- **Excluded apps:** pick apps from the running ones that should never be moved.
- **Learned exceptions:** an app whose windows repeatedly cannot be moved is remembered and left alone. The list is visible and editable.
- **Arrange at startup:** windows that are already maximized when AutoSpaces starts are spread over separate desktops, as long as all your windows are on a single desktop.
- **Desktop layout:** choose apps that should open on their own desktops when AutoSpaces starts (desktop 2 – mail, desktop 3 – charts, …). Apps that are already running are moved and maximized instead of being started again. They are launched in the background, without switching your desktop.
- **Carry a window:** hold **Win+Ctrl+Shift** and the screen behind the active window turns to frosted glass, with a strip of live desktop previews at the bottom. Press **←/→** to take the window to the previous or next desktop: it stays sharp and still while the desktops, wallpapers included, slide beneath it. Release the keys to drop it there (it stays in front and keeps the focus), press **Esc** to take it back. Past the last desktop it goes round to desktop 1, or (an option in *Settings → Keys*) a new desktop is created. The same rules as for maximizing apply: a maximized window carried onto a desktop that already has one moves on to a free desktop.
- **Automatic updates:** 10 minutes after start and then once a day, AutoSpaces asks GitHub whether a newer version has been released. If it has, a notification offers it. *Install* downloads the installer, checks it against the SHA-256 checksum published with the release and installs it in the background: AutoSpaces gives the windows and desktops back, updates itself and starts again. *Check for updates* in the tray menu does the same on request, and a version can be skipped.
- **Start with Windows.**
- **Czech and English,** chosen by the Windows display language, with a manual override in Settings.
- **Light on resources:** a single executable under 1 MB, about 3 MB of private memory, event-driven with no polling.

Settings live in `%APPDATA%\AutoSpaces\settings.json`; learned exceptions and state are in `%LOCALAPPDATA%\AutoSpaces`.

**Network:** AutoSpaces connects only to GitHub, and only to check for updates (and, when you choose *Install*, to download the installer). The request carries nothing but the app name and version; no data about you or your computer is sent. Automatic checks can be switched off in *Settings → General → Check for updates*.

## Supported Windows versions

| Windows | Status |
|---|---|
| Windows 11 24H2 and newer (build 26100+) | Tested |
| Windows 11 21H2, 22H2, 23H2 | Included, not yet tested on a real system |
| Windows 10 1809–22H2 | Included, not yet tested on a real system |

Windows offers no public API for moving another app's window between virtual desktops, so AutoSpaces uses the internal interface of the Windows shell. That interface differs between Windows versions and can change with an update. To stay safe, AutoSpaces checks the interface with read-only calls against the registry at startup and only then uses it. If the check fails, AutoSpaces moves nothing, tells you that this Windows version is not supported, and offers *Report a problem…* in the tray menu.

## Installation

Run `AutoSpaces-Setup-<version>.exe`. By default it installs for the current user without administrator rights; installing for all users is optional. The installer is not code-signed yet, so Windows SmartScreen shows a warning: choose *More info → Run anyway*.

After installation AutoSpaces sits in the notification area. Right-click the icon for *Enabled*, *Settings…*, *Learned exceptions*, *Check for updates*, *About…* and *Quit*.

**Tip:** to make a click on a running app's taskbar icon take you to its desktop, open Windows *Settings > System > Multitasking > Desktops* and set *On the taskbar, show all the open windows* to *On all desktops*. Shift + click then starts a new instance.

## Building from source

Requirements: Rust (MSVC toolchain) and the Windows SDK (for `rc.exe`); Inno Setup 6 for the installer.

```
cargo build --release
cargo test
powershell -ExecutionPolicy Bypass -File tools\build-installer.ps1
```

The executable is written to `target\release\autospaces.exe`, the installer to `dist\`. The only dependency is the `windows-sys` crate.

Useful switches: `--selftest --select-only` (read-only check of the desktop interface), `--restore` (give windows back and tidy the desktops), `--quit` (ask a running instance to exit cleanly).

## Known limitations

- A launching app's splash screen can flash briefly on your current desktop before it is moved; Windows always creates new windows on the desktop you are looking at.
- Windows reports a minimize about 100 ms late, so returning from a minimized window has a short delay.
- Some apps cannot be moved between desktops at all; they end up among the learned exceptions.

## Author

Přemysl Vavroušek · [www.premysl-vavrousek.cz](https://www.premysl-vavrousek.cz)

If AutoSpaces is useful to you, the *About…* dialog lists ways to support its development.

## Licence

AutoSpaces is free software under the [GNU General Public License v3.0 or later](LICENSE). The name, the icon and the author's payment details are not covered by it; see [NOTICE.md](NOTICE.md).

## Reporting a problem

Please open an issue at [github.com/premekvibe/AutoSpaces/issues](https://github.com/premekvibe/AutoSpaces/issues). The tray icon menu → *Report a problem…* creates a diagnostic file (`report.txt`, without window titles or your name) that you can drag into the report.
