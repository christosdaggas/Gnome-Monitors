# Research and Feasibility

## Which API is used, and why it is correct for GNOME Wayland

The application uses **`org.gnome.Mutter.DisplayConfig`** on the session bus
(object `/org/gnome/Mutter/DisplayConfig`): `GetCurrentState`,
`ApplyMonitorsConfig`, and the `MonitorsChanged` signal. This is the same
interface GNOME Settings' Displays panel uses (verified in the
gnome-control-center 50.3 sources — since GNOME 50 the D-Bus code lives
directly in `panels/display/cc-display-config.c`), and it is the only
supported configuration mechanism on a GNOME Wayland session:

* `xrandr` speaks the X11 RandR protocol. Under Wayland it can only see
  Xwayland's emulated view and cannot configure outputs. Not used.
* `wlr-output-management` is a wlroots protocol; Mutter does not expose it.
  Not used.
* `~/.config/monitors.xml` is Mutter's *persistence file*, written by Mutter
  itself after a confirmed configuration change. Editing it directly bypasses
  validation and takes effect unpredictably. It is inspected read-only for
  research (its history proved connector-name instability) but is never
  written by this application.

The interface definition was fetched at the exact installed version (tag
50.2; byte-identical to 49.0) and cross-checked against live introspection
of the running Mutter. Full reference: `docs/mutter-dbus-notes.md`.

## Partial mirroring feasibility — verified

The D-Bus model makes mirroring structural: a *logical monitor* carries a
list of monitor assignments, and **more than one assignment means those
monitors clone each other**. Partial mirroring is therefore "one logical
monitor with two members + one logical monitor with one member".

This was verified on the installed Mutter 50.2 with verify-mode
`ApplyMonitorsConfig` calls (method 0, changes nothing) before any code was
written, and later re-verified through this application's own backend:

| Configuration | Result |
|---|---|
| Mirror group (DP-7 4K@59.997 + KVM 4K@30.000) beside extended DP-8 | Accepted |
| The same with mirror group above/below/left/right | Accepted |
| Mirror group at fractional scale 1.5, at 1080p, rotated 90° | Accepted |
| Mixed refresh rates inside the group | Accepted (server compares only mode width/height — `meta-monitor-config-manager.c:1828`) |

GNOME Settings itself never exercises partial mirroring (its clone model is
all-or-nothing, `cc_display_config_set_cloning`), which is precisely the gap
this application fills — using the same wire capability, not a private API.

Two physical monitors belong to one logical monitor by listing both
assignments in one `(iiduba(ssa{sv}))` entry; the third monitor stays a
separate logical monitor with its own position/scale/transform. Mirror-group
constraints (all verified against the 50.2 source):

* every member's mode must have the **same width × height** (refresh may
  differ);
* the group's scale must be supported by **every** member's chosen mode;
* the group has one position, scale, transform, and primary flag.

## What can and cannot be controlled

**Controllable via `ApplyMonitorsConfig` on Mutter 50.2** (all implemented):
position, primary, per-monitor mode (resolution + refresh, including VRR
modes as distinct `…+vrr` entries), per-logical-monitor scale (fractional
allowed in the logical layout mode), transform (8 values), enabled state
(disable = omit), mirror grouping, `underscanning` (only for monitors that
advertise support), `color-mode` (HDR BT.2100 / SDR-native, monitors
advertising `supported-color-modes`), `rgb-range`, `monitors-for-lease`,
layout mode (logical/physical).

**Not controllable through this interface**: brightness/backlight of
ordinary desktop monitors (separate `SetBacklight` API with its own serial;
the audited monitors expose no backlight), night light (gsettings), gamma /
CTM (legacy CRTC API, not part of the monitor-level model), privacy screen
(gsettings + hardware), panel orientation on tablets (managed by Mutter).

**Deliberately not exposed in the UI** ("no fake controls"): HDR color mode
and RGB range are surfaced read-only in the Details expander for v0.1 —
the KVM lacks BT.2100 support and the API applies these values unvalidated
(`color-mode`/`rgb-range` are parsed but not checked server-side in 50.2),
so blind toggles would be unsafe; underscanning is hidden because no
connected monitor supports it. VRR appears via mode selection (`+vrr`
modes) rather than a separate switch. These can be promoted to controls
once testable hardware exists.

## Physical vs logical monitors

Physical monitors are what `GetCurrentState` lists: connector + EDID
identity + mode list + capability properties. Logical monitors are regions
of the coordinate space that host one or more physical monitors. All layout
operations (position, adjacency, primary, scale, transform) act on logical
monitors; mode selection acts on physical monitors. The UI mirrors this
split: cards on the canvas are logical displays; the sidebar shows the
member monitors of the selected card.

## Version-sensitivity risks and compatibility strategy

Risks: the interface is not formally stable; properties appear per version
(`rgb-range` in 49, `color-mode`/`is-for-lease` in 48, the 48-only
Luminance API was removed in 49), enum *documentation* has historically
lagged the implementation (layout-mode doc was reversed before 47), and the
bundled gdctl 50 labels transforms 6/7 differently from the interface XML.

Strategy:

1. **Parse defensively.** Every `a{sv}` property is optional; documented
   "absence means X" defaults are applied; unknown properties are preserved
   and shown in diagnostics instead of causing failures (tested).
2. **Send conservatively.** Optional apply-side properties (`color-mode`,
   `rgb-range`, `underscanning`, `layout-mode`) are sent only when the
   current state showed the server understands them.
3. **Let the server decide.** Client-side validation mirrors Mutter 50.2's
   rules for good error messages, but every apply is preceded by a
   verify-mode call, and Mutter's verdict is authoritative.
4. **Exact error-string mapping is best-effort.** Messages are classified
   with substring matching; unknown messages fall back to a generic
   rejection with the raw text in the technical-details expander.
5. **Serial discipline.** Every apply uses the serial of a freshly fetched
   state; `MonitorsChanged` invalidates any held state.

## Rust stack (verified against crates.io / docs.rs, July 2026)

gtk4 0.11.4 (`gnome_50` feature = GTK 4.22 + gio 2.88 — exactly the Fedora
44 platform), libadwaita 0.9.2 (`v1_9`), zbus 5.18 / zvariant 5.13 (default
async-io executor runs on its own thread, so zbus futures can be awaited
from GLib's main loop via `glib::spawn_future_local` — no tokio), rustc
1.96.1 (MSRV needed: 1.92). All widgets used (`ToolbarView`,
`OverlaySplitView`, `AlertDialog`, `SwitchRow`, `ComboRow`, `EntryRow`,
`ExpanderRow`, `Banner`, `ToastOverlay`, `StatusPage`) exist in libadwaita
0.9 at or below the `v1_5` gate, well within the installed 1.9.2.

## Identification overlays and previews

Monitor identification first tries `org.gnome.Shell.ShowMonitorLabels`
(the GNOME Settings mechanism; call site cc-display-panel.c:325–371).
Live testing revealed GNOME Shell guards this method with a D-Bus sender
allowlist — third-party callers get `org.freedesktop.DBus.Error.AccessDenied`
— so the app detects the denial once and falls back to its own overlay:
one fullscreen window per monitor (`fullscreen_on_monitor`, the only
per-monitor placement Wayland allows an app) styled as a brief
identification screen with the position-based display number. No private
Shell APIs, no `Eval`, no bus-name impersonation.

Live thumbnails inside monitor cards were evaluated and deferred: they would
require an XDG Desktop Portal ScreenCast session + PipeWire per monitor,
with explicit user permission and a persistent "screen sharing" indicator —
disproportionate for v0.1. The cards instead show name, number, mode, scale,
and status badges. The portal route is documented for a future release
(`docs/known-limitations.md`).
