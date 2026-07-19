# System Audit

Read-only audit of the machine this application is being developed on and for.
Performed 2026-07-19. No system configuration was changed during the audit; the
only D-Bus mutations attempted were `ApplyMonitorsConfig` calls with method
`0` (**verify**), which validate a configuration without applying it.

## Summary

| Item | Value |
|---|---|
| OS | Fedora Linux 44 (Workstation Edition), stable |
| Kernel | 7.1.3-201.fc44.x86_64 |
| Architecture | x86_64 |
| Host | `Linserver`, Micro Computer (HK) Tech Limited "AI Series" mini-PC (desktop chassis) |
| Container | None (bare metal; `systemd-detect-virt` → `none`) |
| Desktop | GNOME, **Wayland** session |
| GNOME Shell | 50.3 (`gnome-shell-50.3-1.fc44`) |
| Mutter | **50.2** (`mutter-50.2-2.fc44`) |
| GTK | 4.22.4 (`gtk4-4.22.4-1.fc44`, devel installed) |
| libadwaita | 1.9.2 (`libadwaita-1.9.2-1.fc44`, devel installed) |
| glib | 2.88.2 |
| Display manager | GDM 50.1 (session service `gdm-password`, session 3, seat0, `Type=wayland`) |
| GPU | AMD Radeon 880M/890M "Strix" iGPU (`1002:150e`), `amdgpu` driver |
| Rust | rustc/cargo 1.96.1 (Fedora packages) |
| gdctl | present (`/usr/bin/gdctl`, Python, part of Mutter 50) |

Environment of the graphical session: `XDG_SESSION_TYPE=wayland`,
`XDG_CURRENT_DESKTOP=GNOME`, `DESKTOP_SESSION=gnome`,
`WAYLAND_DISPLAY=wayland-0`, `DISPLAY=:0` (Xwayland), `XDG_RUNTIME_DIR=/run/user/1000`.

## Graphics hardware

* Single GPU: `c7:00.0 Display controller: AMD [AMD/ATI] Strix [Radeon 880M / 890M] [1002:150e] (rev e4)`, kernel driver `amdgpu` (loaded, with `drm_display_helper`, `cec`, etc.).
* DRM nodes: `/dev/dri/card1` (+ `renderD128`), `by-path pci-0000:c7:00.0`.
* No hybrid graphics, no NVIDIA, no virtual GPU.

DRM connector status at audit time:

| Connector | Status |
|---|---|
| card1-DP-1 … DP-6 | disconnected |
| **card1-DP-7** | connected, enabled |
| **card1-DP-8** | connected, enabled |
| **card1-HDMI-A-1** | connected, enabled |
| card1-Writeback-1 | writeback (ignore) |

## Connected displays

Mutter is the authoritative source (`GetCurrentState`); EDID read from
`/sys/class/drm/*/edid` correlates exactly.

### DP-7 — LG 27" 4K (physical monitor)

* Mutter monitor spec: connector `DP-7`, vendor `GSM`, product `LG HDR 4K`, serial `0x0004ee0e`
* EDID: PNP `GSM`, product 0x7707, serial32 323086 (=0x4EE0E), size 60×34 cm
* `display-name`: `LG Electronics 27"`
* 56 modes, 24 unique resolutions; preferred + current: `3840x2160@59.997` (also 59.968, 30.000 at 4K)
* Supported scales at 4K: 1.0 … 4.0 incl. fractional (1.25, 1.5, 1.75 …); preferred scale 2.0
* Properties: `is-underscanning: false`, `is-builtin: false`, `is-for-lease: false`, `color-mode: 0`, `supported-color-modes: [0, 2, 1]` (default, sdr-native, **bt2100** → HDR-capable), `rgb-range: 1` (auto)
* Currently: active, **primary**, position (1920, 0), scale 2.0, transform normal

### DP-8 — LG 27" 4K (physical monitor, same model as DP-7)

* Mutter monitor spec: connector `DP-8`, vendor `GSM`, product `LG HDR 4K`, serial `0x0003e924`
* EDID serial32 256292 (=0x3E924) — same model as DP-7, distinguished **only by serial**
* Same mode/scale/color capabilities as DP-7; current mode `3840x2160@59.997`
* Currently: active, position (0, 0), scale 2.0

### HDMI-1 — KVM / remote-management device (emulated display)

* Mutter monitor spec: connector `HDMI-1`, vendor `LTM`, product `Lontium semi`, serial `0x88888800`
* `display-name`: `LTM 5"` — EDID claims a 12×7 cm, 5-inch panel
* **This is the KVM.** Evidence:
  * "LTM" = Lontium Semiconductor, a maker of HDMI capture/converter chips used in KVM-over-IP devices.
  * Physically implausible identity: a "5-inch" 4K panel with repeated-digit serial `0x88888800`.
  * Mode table is a capture-device profile: 4K only at ≤ 30 Hz (30/29.97/25/24/23.976), everything else ≤ 60 Hz — typical of an HDMI-to-USB/network capture pipeline, not a real 4K monitor.
  * `~/.config/monitors.xml` history contains an earlier KVM on the same HDMI connector identifying as `AOC GLKVM` (GL.iNet KVM), serial `AHLL19A000036`.
* 53 modes; preferred + current `3840x2160@30.000`
* Properties: like the LGs, plus `min-refresh-rate: 24` and `supported-color-modes: [0, 2]` (no bt2100 → no HDR)
* Currently: active, position (3840, 0), scale 2.0

### Current logical layout (all extended, one row)

```
(0,0) 1920×1080L        (1920,0) 1920×1080L      (3840,0) 1920×1080L
┌──────────────┐        ┌──────────────┐         ┌──────────────┐
│ DP-8 (LG #2) │        │ DP-7 (LG #1) │         │ HDMI-1 (KVM) │
│ 4K@60 s2.0   │        │ 4K@60 s2.0 ★ │         │ 4K@30 s2.0   │
└──────────────┘        └──────────────┘         └──────────────┘
                              ★ = primary
```

State serial at audit time: `1`. Global properties: `layout-mode: 1` (logical),
`supports-changing-layout-mode: true`. `global-scale-required` is **absent**
(per-logical-monitor scales allowed).

### Common mirror resolutions (DP-7 ∩ HDMI-1)

21 common resolutions, including 3840×2160 (KVM side limited to 30 Hz there),
2560×1440@59.95, 1920×1080@60. A 4K mirror of DP-7+KVM runs DP-7 at 59.997 Hz
and the KVM at 30.000 Hz — **verified acceptable to Mutter** (see below).

## Live D-Bus interface (`org.gnome.Mutter.DisplayConfig`, Mutter 50.2)

`busctl --user introspect org.gnome.Mutter.DisplayConfig /org/gnome/Mutter/DisplayConfig`:

```
.ApplyConfiguration    uba(uiiiuaua{sv})a(ua{sv})                            (legacy, unused)
.ApplyMonitorsConfig   uua(iiduba(ssa{sv}))a{sv}                             ← modern API
.GetCurrentState       → ua((ssss)a(siiddada{sv})a{sv})a(iiduba(ssss)a{sv})a{sv}
.GetResources          (legacy, unused)
.SetBacklight          usi        .ChangeBacklight (deprecated)
.GetCrtcGamma / .SetCrtcGamma / .SetOutputCTM                                (legacy CRTC-level)
Properties: ApplyMonitorsConfigAllowed=true, Backlight (uaa{sv}),
            HasExternalMonitor=true, NightLightSupported=true,
            PanelOrientationManaged=false, PowerSaveMode=0 (writable)
Signal: MonitorsChanged ()
```

Enumerations confirmed from the installed `gdctl` (ships with Mutter 50.2, uses
this exact API):

* `ConfigMethod`: 0 = verify, 1 = temporary, 2 = persistent
* `LayoutMode`: 1 = logical, 2 = physical
* `Transform`: 0 normal, 1 90°, 2 180°, 3 270°, 4 flipped, 5 flipped-90, 6 flipped-270, 7 flipped-180
* `ColorMode`: 0 default, 1 bt2100, 2 sdr-native
* `RgbRange`: 1 auto, 2 full, 3 limited
* `ApplyMonitorsConfig` top-level properties: `layout-mode: u` (only if
  `supports-changing-layout-mode`), `monitors-for-lease: a(ssss)`
* Per-monitor-assignment properties in apply: `color-mode: u`, `rgb-range: u`

The GNOME Shell interface `org.gnome.Shell` additionally exposes
`ShowMonitorLabels(a{sv})` / `HideMonitorLabels()` — the supported identify-
overlay mechanism used by GNOME Settings.

## Feasibility probes (verify-only, nothing applied)

All probes used `ApplyMonitorsConfig` with method 0 (verify) via `gdctl set --verify`:

| Probe | Result |
|---|---|
| **Partial mirror**: one logical monitor = DP-7 `3840x2160@59.997` + HDMI-1 `3840x2160@30.000` (mixed refresh), DP-8 extended right | **Accepted** |
| Mirror group below independent monitor (vertical layout) | Accepted |
| Mirror group at fractional scale 1.5 | Accepted |
| Mirror at 1920×1080@60 next to 4K extended monitor | Accepted |
| Mirror group rotated 90° | Accepted |
| DP-8 omitted from config (= disabled) | Accepted |
| Gap between logical monitors (x=2000 instead of 1920) | Rejected: `Logical monitors not adjacent` |
| Overlapping logical monitors | Rejected: `Logical monitors not adjacent` |
| Two primary logical monitors | Rejected: `Config contains multiple primary logical monitors` |
| No primary logical monitor | Rejected: `Config is missing primary logical monitor` |

**Conclusion: the target topology (KVM + LG #1 mirrored as one logical monitor,
LG #2 extended, either side/above/below, selectable primary) is fully supported
by the installed Mutter 50.2.** Mirroring is represented as *multiple monitors
inside one logical monitor*, exactly as the API models it; members may run
different refresh rates as long as the mode resolution matches.

## Connector-name instability (why EDID identity is required)

`~/.config/monitors.xml` history (inspected read-only) shows the same two LG
panels (serials 0x0004ee0e / 0x0003e924) previously appearing on **DP-5/DP-6**,
and both LGs have swapped connectors between configurations. The KVM has
appeared as two different devices on HDMI-1 (`AOC GLKVM`, now `LTM Lontium
semi`), plus a historical ASUS `ROG PG248Q` on the same connector. Saved
profiles must therefore match on vendor+product+serial (EDID identity), never
on connector name alone.

## Toolchain

* rustc / cargo 1.96.1 (Fedora), gcc 16.1.1, pkg-config 2.5.1
* `gtk4-devel-4.22.4`, `libadwaita-devel-1.9.2`, `glib2-devel-2.88.2` installed
* crates.io reachable from cargo (zbus 5.18.0 current at audit time)

## Raw captures

Raw `GetCurrentState` output and a JSON rendering are kept in the session
scratchpad (`audit/get-current-state.raw`, `audit/state.json`) and reproduced
in condensed form in `docs/mutter-dbus-notes.md`.
