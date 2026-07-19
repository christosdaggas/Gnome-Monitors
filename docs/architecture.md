# Architecture

Cargo workspace with five crates, separated so that domain logic, D-Bus
code, and UI never mix:

```
crates/
├── display-core          pure domain models + validation (no D-Bus, no GTK)
├── mutter-backend        typed org.gnome.Mutter.DisplayConfig backend (zbus)
├── display-cli           monitor-layout-ctl: diagnostics / PoC CLI
├── display-revert-helper monitor-layout-revert-helper: rollback watchdog
└── display-ui            monitor-layout: GTK 4 / libadwaita application
```

## display-core

Compositor-independent and fully unit-tested. Key modules:

| Module | Contents |
|---|---|
| `identity` | `MonitorIdentity` (connector + EDID triple), `stable_key()`, match levels, synthetic-serial heuristic |
| `mode` | `MonitorMode` (id, size, refresh, scales, VRR flag, unknown-property bag), formatting helpers |
| `monitor` | `PhysicalMonitor` + capabilities (HDR color modes, RGB range, VRR, underscanning), KVM heuristic |
| `layout` | `LogicalDisplay` (≥1 member = mirror group), `MonitorAssignment`, `DisplayLayout`, logical-size math (`round(size/scale)`) |
| `state` | `DisplayState` (parsed `GetCurrentState`), `ApplySnapshot`, `ApplyMethod` |
| `geometry` | `Rect` with Mutter 50's exact overlap/adjacency semantics (corner contact is *not* adjacent) |
| `validation` | The Mutter 50.2 rule set client-side: unknown monitor/mode, mirror resolution equality, scale support, overlap, edge-connectivity, primary count, origin normalization (auto-fixable) |
| `mirror` | Common-mirror-mode calculation: resolution intersection, best mode per member, common scales, "why not" explanations |
| `snap` | Canvas geometry: edge snapping with guides, `settle_rect` (closes gaps, resolves overlaps, guarantees adjacency) |
| `layout_ops` | High-level edits: `build_mirror_layout` (mirror group left/right/above/below the rest), `merge_into_mirror`, `split_from_mirror` |
| `profile` | `DisplayProfile` with EDID-identity matching (connector only as tie-breaker), conservative resolution with per-problem reports |
| `prefs` | Friendly names, KVM overrides, confirm-countdown seconds |
| `paths`/`store` | XDG paths, atomic JSON persistence |

## mutter-backend

* `proxy` — zbus `#[proxy]` for the verified 50.2 signatures; raw tuple type
  aliases (`RawState`, `WireLogicalMonitor`, …).
* `parse` — raw → domain. Tolerant: optional properties defaulted, unknown
  ones preserved stringified. Logical-monitor members get their mode from
  the member's `is-current` mode; current `color-mode`/`rgb-range`/
  `underscanning` are carried into assignments so re-applying a snapshot
  preserves them.
* `serialize` — domain → wire. Optional keys sent only when known-supported.
* `error` — classification of Mutter's rejection strings into
  `RejectionKind` + user-friendly messages (raw text kept for the details
  expander).
* `backend` — `DisplayBackend` trait (`current_state` / `apply` / `events`)
  with two implementations: `MutterBackend` (real, also connectable to any
  `zbus::Connection` — used by the fake-service integration tests) and
  `MockBackend` (in-memory, validates like Mutter, emits events, hot-plug
  simulation).
* `fixtures` — the named test topologies (single, dual-extended,
  dual-mirrored, mirror-plus-extended, triple-extended, kvm-no-serial,
  identical-twins, mixed-1080p-4k, mixed-refresh-rates, fractional-scaling,
  rotated, unknown-properties, no-common-mirror-mode).
* `shell_labels` — `org.gnome.Shell` ShowMonitorLabels/HideMonitorLabels.

## display-ui

GTK 4 + libadwaita, built in code (no templates), single window:

* `ui_state` — `AppState`: current `DisplayState`, edited `DisplayLayout`,
  selection, prefs, guard flags; dirty = normalized inequality (the GNOME
  Settings `config_equal` pattern).
* `canvas` — `DrawingArea` with cairo/pango drawing of monitor cards
  (position-numbered, name, mode, scale, ★ primary, ⧉ mirrored, KVM tag),
  drag with live snap guides, drop-settling via `snap::settle_rect`,
  keyboard nudging (arrows / Shift+arrows), auto-fit view transform.
* `panel` — the sidebar for the selected logical display: member list with
  split buttons, "Mirror With" combo (`layout_ops::merge_into_mirror`),
  primary/enabled switches, resolution/refresh/scale/orientation combos
  (only values the compositor advertises), per-monitor Details expander
  (identity, physical size, capabilities, friendly-name editor, KVM
  override). Structural edits rebuild the panel from an idle handler —
  the GNOME Settings anti-loop pattern.
* `profiles` — profiles menu + save/manage dialogs; loading resolves via
  EDID identity and puts the layout in the *editor* (apply stays explicit).
* `preview` — temporary-apply preview with in-app countdown + the
  out-of-process watchdog (below).
* `main` — window assembly, apply flow, MonitorsChanged subscription
  (refresh preserves unsaved edits and re-validates them), Shell label
  wiring (labels shown while the window is focused).

### Event/state flow

```
Mutter ──MonitorsChanged──▶ refresh(): GetCurrentState (fresh serial)
                            ├─ edits clean → editor tracks new state
                            └─ edits dirty → editor kept, revalidated,
                               banner explains problems (e.g. monitor gone)
canvas/panel edits ─▶ edited layout ─▶ validate() ─▶ banner + Apply gating
Apply… ─▶ warning dialog ─▶ verify(0) ─▶ persistent(2) ─▶ Mutter arms 20 s
        revert + GNOME Shell shows "Keep these settings?"        (see below)
Try N s ─▶ verify(0) ─▶ watchdog armed ─▶ temporary(1) ─▶ in-app countdown
```

## display-revert-helper

A ~150-line process with no GTK dependency. The UI spawns it *before* a
temporary apply and passes `{timeout_seconds, snapshot}` on stdin. It
restores the snapshot (fresh serial, temporary method, stale-serial retry,
no-op if the layout already matches) when stdin hits EOF (parent died) or
the timeout expires without a `CONFIRM`/`CANCEL` line. See
`docs/safety-and-rollback.md`.

## display-cli

`monitor-layout-ctl state|watch|verify-mirror|test-apply|diagnostics` — the
proof-of-concept tool required by the project plan, kept as a diagnostics
utility. `state --json` and `diagnostics --json` produce sanitized
machine-readable reports.
