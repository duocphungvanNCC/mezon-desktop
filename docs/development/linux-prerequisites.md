# Linux Prerequisites

This project uses GPUI, a native GPU-accelerated UI framework that depends on
WebKitGTK 4.1, GTK4, and Linux windowing system libraries. These must be
installed before the project can compile on Linux.

## Supported Distributions

The instructions below target **Ubuntu 24.04+** and Debian-based derivatives
(Pop!_OS, Linux Mint, etc.). Package names may differ on other distributions
(see notes at the end).

## Install System Libraries

```sh
sudo apt update
sudo apt install \
  build-essential \
  pkg-config \
  libwebkit2gtk-4.1-dev \
  libsoup-3.0-dev \
  libgtk-4-dev \
  libglib2.0-dev \
  libxkbcommon-dev \
  libwayland-dev \
  libx11-dev \
  libxcb-xfixes0-dev \
  libxcb-xkb-dev \
  libxcb-randr0-dev \
  libxcb-cursor-dev
```

`libwebkit2gtk-4.1-dev` pulls in `libjavascriptcoregtk-4.1-dev` automatically
as a dependency. These two packages are required by the vendored `gpui_linux`
crate for window creation and rendering.

## Verify Installation

Confirm the WebKit libraries are discoverable by `pkg-config`:

```sh
pkg-config --modversion webkit2gtk-4.1
pkg-config --modversion javascriptcoregtk-4.1
```

Both commands should print a version number (e.g. `2.46.x`). If either reports
"not found", the corresponding `-dev` package is missing or `pkg-config` cannot
find its `.pc` file.

## Build

After installing the system libraries, build the project:

```sh
cargo clean
cargo build
```

If `cargo build` fails with `pkg-config` errors (e.g. "not found"), try
running the build from a **native Linux terminal** (GNOME Terminal, Konsole,
etc.) instead of VS Code's integrated terminal. The integrated terminal may
inherit a different environment or miss `PKG_CONFIG_PATH` entries from your
shell profile.

## Troubleshooting

### `webkit2gtk-4.1.pc` / `javascriptcoregtk-4.1.pc` not found

This is the most common build failure on a fresh Linux setup. The full error
looks like:

```
Package webkit2gtk-4.1 was not found in the pkg-config search path.
Perhaps you should add the directory containing webkit2gtk-4.1.pc
```

**Cause:** The WebKitGTK development headers are not installed, or
`pkg-config` cannot locate them.

**Fix:**

1. Install `libwebkit2gtk-4.1-dev` (see command above).
2. If already installed, check that the `.pc` file exists:

   ```sh
   find /usr -name 'webkit2gtk-4.1.pc' 2>/dev/null
   ```

   Typically it lives under `/usr/lib/x86_64-linux-gnu/pkgconfig/`.
3. If found but `pkg-config` still fails, set the path explicitly:

   ```sh
   export PKG_CONFIG_PATH=/usr/lib/x86_64-linux-gnu/pkgconfig:$PKG_CONFIG_PATH
   ```

4. Run `cargo clean && cargo build` again.

### `Package 'webkit2gtk-4.1' required by 'virtual:world' not found`

Same root cause and fix as above. On Ubuntu 24.04+, `libwebkit2gtk-4.1-dev` is
the correct package. The older `libwebkit2gtk-4.0-dev` is not available by
default and is not used by this project.

## Other Distributions

- **Fedora / RHEL:** Use `dnf install webkit2gtk4.1-devel glib2-devel gtk4-devel libsoup3-devel`
  and Wayland/X11 development groups.
- **Arch Linux:** Use `pacman -S webkit2gtk-4.1 glib2 gtk4 libsoup3 libxkbcommon`
  and the `wayland` / `xorg` group packages.
- **openSUSE:** Use `zypper install webkit2gtk-4_1-devel glib2-devel gtk4-devel libsoup3-devel`.

Package discovery for vendored crates (`gpui_linux`, `gpui_wgpu`) may require
additional X11 or Wayland development libraries depending on your desktop
environment. The `apt` command above covers the full set for Debian-based
distributions.
