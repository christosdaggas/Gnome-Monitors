<img src="docs/img/icon.svg" width="96" align="right" alt="Monitor Layout icon"/>

# Monitor Layout

A native GNOME (Wayland) app for arranging displays — with **partial
mirroring**: mirror two screens as a group while a third stays extended.
Ideal for KVM setups where a remote console should mirror the main monitor.

![Monitor Layout](docs/img/main-window.png)

## Features

- **Visual editor** — drag displays with edge snapping; invalid layouts are
  explained before you apply.
- **Partial mirroring** — group any subset of displays; place the group
  left, right, above, or below the rest.
- **Per-display settings** — resolution, refresh rate, scale (incl.
  fractional), orientation, primary, enable/disable.
- **Identify** — numbered overlay on each screen.
- **Profiles** — save named layouts, matched by monitor identity so they
  survive cable swaps.
- **Safe applies** — GNOME's own keep-or-revert confirmation, plus a
  watchdog that restores the layout if the app crashes. Nothing is saved
  until you confirm.

## Install

Download the RPM from the
[latest release](https://github.com/christosdaggas/Gnome-Monitors/releases/latest),
then:

```bash
sudo dnf install ./monitor-layout-1.0.0-1.fc44.x86_64.rpm
```

Or build it yourself:

```bash
sudo dnf install rust cargo gtk4-devel libadwaita-devel
cargo build --release
./target/release/monitor-layout
```

Uninstall with `sudo dnf remove monitor-layout`.

## Usage

1. **Identify** your screens (button in the header).
2. **Drag** displays on the canvas to arrange them.
3. **Mirror two, keep one extended**: select a display → sidebar →
   *Mirror With* → pick the second display. Drag the group to any side.
4. **Apply** — confirm *Keep Changes* within 20 s, or it reverts.
5. **Profiles** → *Save Layout as Profile* to store an arrangement.

## Requirements

Fedora 44 / GNOME 50 on Wayland (Mutter 50.2+). Needs no root and no special
permissions.

> **Note:** Mutter 50.2 has a bug that crashes the session on display
> changes; it is fixed in **Mutter 50.3** — update before applying layouts.

## Docs & source

More detail lives in [`docs/`](docs/): architecture, the safety model, the
Mutter D-Bus reference, testing, and known limitations.

## License

GPL-3.0-or-later. © Christos A. Daggas.
