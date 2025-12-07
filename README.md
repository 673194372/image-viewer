# Image Viewer

A lightweight, fast image viewer for Wayland, built with GTK4 and Rust.

[中文文档](README_zh.md)

## Features

- **Fast rendering** - Hardware-accelerated with Cairo
- **Overlay mode** - Pin images on top using wlr-layer-shell (Wayland only)
- **Intuitive controls** - Scroll to zoom, drag to pan, double-click to toggle overlay
- **Minimal UI** - Custom titlebar with essential controls only
- **Image operations** - Rotate, copy to clipboard, fit-to-window

## Screenshots

![Normal Mode](assets/screenshot.png)

## Installation

### Runtime Dependencies

- GTK4 (>= 4.14)
- gdk-pixbuf2
- cairo
- pango
- wayland (for overlay mode)
- gtk4-layer-shell

**Arch Linux:**
```bash
sudo pacman -S gtk4 cairo pango wayland gtk4-layer-shell
```

**Ubuntu/Debian:**
```bash
sudo apt install libgtk-4-1 libcairo2 libpango-1.0-0 libwayland-client0
# gtk4-layer-shell may need to be built from source
```

**Fedora:**
```bash
sudo dnf install gtk4 cairo pango wayland gtk4-layer-shell
```

### Build Dependencies

- Rust (>= 1.75)
- Cargo
- pkg-config
- GTK4 development files
- Cairo development files
- gtk4-layer-shell development files

**Arch Linux:**
```bash
sudo pacman -S rust pkg-config gtk4 cairo gtk4-layer-shell
```

**Ubuntu/Debian:**
```bash
sudo apt install rustc cargo pkg-config libgtk-4-dev libcairo2-dev
# gtk4-layer-shell-dev may need to be built from source
```

**Fedora:**
```bash
sudo dnf install rust cargo pkg-config gtk4-devel cairo-devel gtk4-layer-shell-devel
```

### Building

```bash
git clone https://github.com/jswysnemc/image-viewer.git
cd image-viewer
cargo build --release
```

The binary will be at `target/release/image-viewer`.

### Install (optional)

```bash
# Binary
sudo cp target/release/image-viewer /usr/local/bin/

# Desktop entry
sudo cp image-viewer.desktop /usr/share/applications/

# Icon
sudo cp assets/image-viewer.svg /usr/share/icons/hicolor/scalable/apps/
```

## Usage

```bash
# Open an image
image-viewer /path/to/image.png

# Or use file manager integration
```

### Controls

| Action | Normal Mode | Overlay Mode |
|--------|-------------|--------------|
| Zoom | Scroll wheel | Scroll wheel |
| Pan | Left-click drag | Left-click drag (moves window) |
| Enter overlay | Double-click | - |
| Exit overlay | - | Double-click |
| Close | Close button / - | Right-click |

### Overlay Mode

Double-click an image to enter overlay mode. The image will be pinned on top of all windows using the Wayland layer-shell protocol. This is useful for reference images while working.

## License

MIT License - see [LICENSE](LICENSE)

## Contributing

Contributions are welcome! Please feel free to submit issues and pull requests.
