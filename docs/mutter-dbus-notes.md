# Mutter DisplayConfig D-Bus Notes (verified against Mutter 50.2)

Everything below was verified against **this machine's** Mutter 50.2:
live `busctl` introspection, verify-mode probes, the bundled `gdctl`
(`/usr/bin/gdctl`, part of Mutter 50), and the upstream sources at tag `50.2`
(`data/dbus-interfaces/org.gnome.Mutter.DisplayConfig.xml`,
`src/backends/meta-monitor-manager.c`, `meta-monitor-config-manager.c`,
`meta-monitor-config-utils.c`, `meta-monitor.c`, `mtk/mtk-rectangle.c`).
The interface XML is byte-identical between tags 49.0 and 50.2.

Bus name `org.gnome.Mutter.DisplayConfig`, object path
`/org/gnome/Mutter/DisplayConfig`, interface `org.gnome.Mutter.DisplayConfig`.

## GetCurrentState

```
GetCurrentState() → (u serial,
                     a((ssss)a(siiddada{sv})a{sv}) monitors,
                     a(iiduba(ssss)a{sv})          logical_monitors,
                     a{sv}                          properties)
```

### Monitor: `((ssss) modes properties)`

* `(ssss)` = connector, vendor, product, serial ("monitor spec"). This
  4-tuple is the monitor's identity everywhere in the API.
* Mode `(siiddad a{sv})` = id, width px, height px, refresh (d),
  preferred scale (d), supported scales (ad), properties.
  * **Mode IDs** are generated as `"%dx%d[i]@%.3f[+vrr]"`
    (`generate_mode_id`, meta-monitor.c:739): `1920x1080@60.000`,
    interlaced `1920x1080i@59.940`, VRR `2560x1440@143.998+vrr`. Treat as
    opaque strings; select modes by passing the ID verbatim back.
  * Mode properties: `is-current (b)`, `is-preferred (b)`,
    `is-interlaced (b)`, `refresh-rate-mode (s)` = `"variable"`/`"fixed"`
    (absent ⇒ fixed; VRR modes appear as separate entries).
* Monitor properties (all optional; absence = documented default):
  `width-mm (i)`, `height-mm (i)` (documented; **not emitted by this
  Mutter 50.2 build** — treat as optional and fall back to sysfs EDID),
  `is-underscanning (b)` (absence ⇒ underscanning unsupported),
  `max-screen-size (ii)`, `is-builtin (b)`, `display-name (s)`,
  `privacy-screen-state (bb)` (enabled, hw-locked; absence ⇒ unsupported),
  `min-refresh-rate (i)` (VRR minimum), `is-for-lease (b)`,
  `color-mode (u)`, `supported-color-modes (au)`, `rgb-range (u)`.

### Logical monitor: `(iiduba(ssss)a{sv})`

x, y, scale (d), transform (u), primary (b), monitor specs, properties
(none documented). **More than one monitor spec ⇒ these monitors mirror
each other** (clone group). No mode IDs here — each member's mode is the one
flagged `is-current` in the monitor list.

### Top-level properties

* `layout-mode (u)`: 1 = logical (logical monitor size = mode size ÷ scale),
  2 = physical (size = mode size). Absent ⇒ logical, unchangeable.
  (Note: Mutter ≤ 46 XML documented the values reversed; implementation was
  always 1 = logical.)
* `supports-changing-layout-mode (b)`: send `layout-mode` in apply only if
  true.
* `global-scale-required (b)`: all logical monitors must share one scale.
* `supports-mirroring (b)`: parsed by GNOME Settings with default *true*.
* `legacy-ui-scaling-factor` was removed in Mutter 47 — do not rely on it.

### Enum values

| Enum | Values |
|---|---|
| transform | 0 normal, 1 90°, 2 180°, 3 270°, 4 flipped, 5 flipped-90, 6 flipped-180, 7 flipped-270 (wl_output order; **the gdctl shipped with Mutter 50 labels 6/7 swapped — the XML doc + `MtkMonitorTransform` are authoritative**) |
| layout-mode | 1 logical, 2 physical |
| color-mode | 0 default, 1 BT.2100 (HDR), 2 SDR-native |
| rgb-range | 1 auto, 2 full, 3 limited (0 = internal "unknown", never sent) |
| method | 0 verify, 1 temporary, 2 persistent |

## ApplyMonitorsConfig

```
ApplyMonitorsConfig(u serial, u method,
                    a(iiduba(ssa{sv})) logical_monitors,
                    a{sv} properties)
```

* Logical monitor: x, y, scale (d), transform (u), primary (b), monitors.
* Monitor assignment `(ssa{sv})` = connector, mode ID, properties. Accepted
  per-monitor properties in 50.2: `underscanning (b)` (rejected when the
  monitor doesn't support it), `color-mode (u)`, `rgb-range (u)` (the latter
  two are **not validated** in the D-Bus path — send only supported values).
* Top-level properties: `layout-mode (u)` (only when
  `supports-changing-layout-mode`), `monitors-for-lease (a(ssss))` (each
  spec must name a *connected* monitor that is *omitted* from the config).
* **Disabling a monitor = omitting it.** There is no explicit disable field.
  Disabling *all* monitors is impossible ("Monitors config incomplete").
* Width/height of logical monitors are **derived server-side**:
  transform-swap the mode size, then (logical mode) `roundf(size / scale)`.

### Method semantics

All three methods run identical validation *plus a full CRTC/plane
assignment dry-run*. Differences:

* **0 verify** — frees the assignment and returns. No side effects at all.
* **1 temporary** — applies; pushes the previous config onto Mutter's
  in-memory history (max 3); **never writes `monitors.xml`**.
* **2 persistent** — applies like temporary, then arms a **20 s**
  compositor-side timer (`DEFAULT_DISPLAY_CONFIGURATION_TIMEOUT`,
  meta-monitor-manager.c:67) and emits `confirm-display-change`; GNOME Shell
  shows the *"Keep these display settings?"* dialog.
  * Keep → `meta_monitor_manager_confirm_configuration(TRUE)` → the config
    is saved to `~/.config/monitors.xml` (async). Nothing is re-applied.
  * Revert or timeout → the previous config is re-applied with method
    *temporary*; nothing is saved.

**Consequence:** a client that applies with `persistent` gets Mutter's own
proven confirm/auto-revert workflow for free — this is exactly what GNOME
Settings does (it never uses method 1 and has no countdown of its own).

### Validation rules and exact error strings (50.2)

In handler order; all errors arrive as D-Bus errors whose message text is
the quoted string (`AccessDenied` for the first three, `InvalidArgs` /
`Failed` otherwise):

1. `serial != current` → **"The requested configuration is based on stale
   information"**. The serial increments on every monitor re-read (hot-plug,
   any applied config).
2. monitors.xml `<policy><dbus>` disabled → **"Monitor configuration via
   D-Bus is disabled"**.
3. Bad `layout-mode` value → **"Invalid layout mode specified"**.
4. Unknown connector → **"Invalid connector '%s' specified"**; unknown mode
   ID → **"Invalid mode '%s' specified"**; underscanning on unsupporting
   monitor → **"Underscanning requested but unsupported"**.
5. Empty logical monitor → **"Empty logical monitor"** / **"Logical monitor
   is empty"**.
6. Scale must be within `FLT_EPSILON` of a supported scale (validated
   against the **first** member's mode, re-checked for **every** member in
   the applicability pass) → **"Scale %g not valid for resolution %dx%d"**,
   **"Scale not supported by backend"**. Send scales exactly as advertised.
7. Negative position → **"Invalid logical monitor position (%d, %d)"**.
8. **Mirror constraint**: every member's mode must have the same width AND
   height as the first member's (refresh rates and VRR **may differ**) →
   **"Monitors modes in logical monitor not equal"**.
9. Logical mode: `mode / scale` must be exactly integral →
   **"Scaled logical monitor size is fractional"**. Physical mode: integer
   scales only → **"A fractional scale with physical layout mode not
   allowed"**.
10. Overlap (strict interior; edge-touching fine) →
    **"Logical monitors overlap"**.
11. More than one primary → **"Config contains multiple primary logical
    monitors"**; none → **"Config is missing primary logical"**.
12. Adjacency: flood-fill over `mtk_rectangle_is_adjacent_to` — rectangles
    must share a border of **positive length** (corner contact does NOT
    count); all logical monitors must form one connected component →
    **"Logical monitors not adjacent"**.
13. Bounding box must start at exactly (0, 0) →
    **"Logical monitors positions are offset"**.
14. Applicability re-check against live hardware: **"Specified monitor not
    found"**, **"Specified monitor mode not available"** (spec matching
    tolerance: refresh `< 0.001` Hz, refresh-rate-mode exact),
    lid-closed built-in panel → **"Refusing to activate a closed laptop
    panel"**.
15. CRTC/plane assignment: **"Configured monitor '%s %s' not found"**,
    **"Invalid mode %dx%d (%.3f) for monitor '%s %s'"**, **"No available
    CRTC for monitor '%s %s' not found"** (sic), **"No available primary
    plane found for CRTC %u (%s)"**.

### Supported scales (what Mutter advertises per mode)

`meta_monitor_calculate_supported_scales`: candidate scales are reduced
fractions n/d with d ≤ 4, value in [1, 4], accepted only when they divide
the mode exactly in both axes and leave ≥ 600×600 = 360 000 logical pixels
(`MINIMUM_LOGICAL_AREA`). Fallback list: `{1.0}`. Physical layout mode:
integers only.

## MonitorsChanged

`MonitorsChanged()` — no arguments. Emitted for hot-plug and for every
applied configuration (including our own). On receipt: re-fetch
`GetCurrentState`; any held serial is stale.

## Other interface members (not used by this application)

`GetResources` / `ApplyConfiguration` (legacy CRTC-level API),
`ChangeBacklight` (deprecated) / `SetBacklight(u serial, s connector, i
value)` + `Backlight (uaa{sv})` property (**separate backlight serial** — do
not pass the display-config serial), `GetCrtcGamma` / `SetCrtcGamma`,
`SetOutputCTM`, properties `PowerSaveMode (i, rw)`,
`PanelOrientationManaged (b)`, `ApplyMonitorsConfigAllowed (b)`,
`NightLightSupported (b)`, `HasExternalMonitor (b)`.

## GNOME Shell interface used for identification

`org.gnome.Shell` at `/org/gnome/Shell`:
`ShowMonitorLabels(a{sv})` — dict connector name → int32 label number
(wrapped in a tuple), `HideMonitorLabels()`. This is what the GNOME Settings
Displays panel calls; verified present on the installed Shell 50.3.
GNOME Settings numbers monitors 1..N (built-in first), shows labels only
while its window is active, and hides them when mirroring everything.
