# vscroll-wm (`vscroll-layout`)

[![Platform](https://img.shields.io/badge/platform-cross--platform-blue)](https://github.com/jack-dozen/vscroll-wm)
[![License](https://img.shields.io/badge/license-GPL--3.0-blue)](LICENSE)

> Pure Rust, zero-dependency vertical scrolling tiling layout engine.

## Overview

`vscroll-layout` is the core layout engine extracted from **vscroll-wm** (now powering **noren-wm**). It contains pure layout logic with **no Win32 or UI dependencies**, making it easy to test, embed, and reuse.

### Core Model

- **Vertical scrolling strip with two lanes** (`Full`, `Left`, `Right` spans).
- Independent per-window heights (`1/3`, `1/2`, `Full`, or custom pixel steps).
- Automatic viewport scrolling that follows focus.
- Operations: snap (height / side), maximize, focus navigation, swap/move, join/split, and drag-and-drop **snap layout presets** (`snap`).
- Multi-workspace management and layout serialization (`Workspaces`, `SavedLayout`, `SavedStrip`).

## Usage

Add as a Git dependency in your `Cargo.toml`:

```toml
[dependencies]
vscroll-layout = { git = "https://github.com/jack-dozen/vscroll-wm.git" }
```

Or alias it as `layout`:

```toml
[dependencies]
layout = { package = "vscroll-layout", git = "https://github.com/jack-dozen/vscroll-wm.git" }
```

## Development

```bash
cargo test
```

## License

GPL-3.0 — see [LICENSE](LICENSE).
