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
