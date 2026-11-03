# V[ertical]SCROLL WM

<!-- logo placeholder -->
<!-- <img src="assets/logo.png" alt="VScroll WM logo" width="180"> -->

[![Status](https://img.shields.io/badge/status-development%20%2F%20testing-orange)](https://github.com/jack-dozen/vscroll-wm)
[![Platform](https://img.shields.io/badge/platform-Windows-blue)](https://github.com/jack-dozen/vscroll-wm)
[![License](https://img.shields.io/badge/license-MIT-green)](LICENSE)

> A vertical scrolling tiling window manager for Windows, inspired by Windows Snap and niri.

---

## What

**vscroll-wm** is a tiling window manager that combines the familiar snapping behavior of Windows with the scrolling workspace idea from [niri](https://github.com/YaLTeR/niri).

- Many shortcuts feel the same as Windows Snap (Win + Arrow, etc.)
- Unlike niri (which scrolls **horizontally**), this one lives up to its name: the workspace scrolls **vertically**

<!-- Demo video placeholder -->
<!-- ![Demo](assets/demo.gif) -->
<!-- or embed a YouTube / Loom link later -->

---

## Why

I really like niri’s tiling + scrolling concept, but I was too lazy to relearn a completely new set of muscle memory after years of using Windows Snap.

So I asked Claude to help build a window manager that keeps the Windows-style shortcuts while giving the vertical scrolling workspace feel.

---

## How

### Installation

1. Download the latest `.exe` from the [Releases](https://github.com/jack-dozen/vscroll-wm/releases) page (when available).
2. Put `vscroll-wm.exe` wherever you want.
3. (Optional) Copy `vscroll-wm.conf.example` → `vscroll-wm.conf` next to the exe and edit it.
4. Run the exe. It will sit in the system tray.

You can also point to a custom config with the environment variable:

```
VSCROLL_CONFIG=C:\path\to\your\vscroll-wm.conf
```

### Configuration

Config file: `vscroll-wm.conf` (next to the exe, or via `VSCROLL_CONFIG`).

Reload config anytime from the tray menu → **Reload configuration**.

| Key | Default | Description |
|-----|---------|-------------|
| `gap` | `16` | Gap between windows and screen edges (pixels, 0–200) |
| `anim_ms` | `160` | Animation duration in ms. `0` = off. Auto-disabled if Windows animation effects are turned off |
| `focus_border` | *(none)* | Focus border color on Windows 11 (`#RRGGBB` or `off`) |
| `overview_key` | `tab` | Key used with Win for overview (`tab`, `space`, `grave`, single letter/number, or `off`). `Win+Alt+Space` is always available |
| `overview_size` | `70` | Overview panel size as % of work area (30–100) |
| `overview_hint` | `on` | Show key hints at the bottom of the overview |
| `overview_thumbnails` | `on` | Show window thumbnails in overview |
| `overview_label` | `title` | Label style: `title` / `app` / `reverse` |
| `alias.<exe>` | — | Display name for overview when using `app` label mode (e.g. `alias.code = VS Code`) |
| `overview_color_*` | see example | Colors for overview (`selected`, `active`, `view`, `background`, `box`) |

There is also an ignore file (`vscroll-wm.ignore`) to keep certain windows floating:

```
# Examples
exe:vlc.exe
title:Picture in Picture
class:CabinetWClass
```

### Keyboard Shortcuts

| Shortcut | Action |
|----------|--------|
| `Win` + `←` `→` `↑` `↓` | Snap / move active window |
| `Win` + `Shift` + Arrow | Move focus |
| `Win` + `Enter` | Maximize / restore active window |
| `Win` + `Alt` + Arrow | Swap with neighboring window |
| `Win` + `Alt` + `F` | Toggle float / tile |
| `Win` + `Alt` + `I` | Print window info (class / exe / title) to console |
| `Win` + `Tab` *(or configured key)* | Open overview (strip map) |
| `Win` + `Alt` + `Space` | Overview (always available) |
| `Win` + `Q` | Close active window |
| `Win` + `M` | Minimize active window |
| `Win` + `Ctrl` + `Esc` | Quit and restore all windows |

While overview is open:
- Arrow keys / Tab / Shift+Tab → move selection
- Enter → confirm
- Esc → cancel
- Release Win after moving selection → jump to that window

---

## Hope

I hope someone forks, clones, copies, or just steals the idea and makes a better version of this.

Right now it only supports **Windows**. The long-term wish is to support more operating systems.

### Planned features
- Shortcut cheatsheet overlay
- Consume window horizontally

### Known issues
*(to be filled)*

---

## License

