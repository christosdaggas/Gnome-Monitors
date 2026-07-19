# Known Limitations

## Platform

* **Mutter 50.2 crashes on monitor reconfiguration** (fixed in 50.3).
  Identified as upstream Mutter issue
  [#4891](https://gitlab.gnome.org/GNOME/mutter/-/work_items/4891): windows
  of type DESKTOP / override-redirect keep a stale `target_monitor` pointer
  through a reconfiguration, and the deferred window-relocation queue then
  NULL-dereferences in `meta_workspace_get_onmonitor_region()` —
  `meta_window_move_resize_internal` in the coredump. Any layout change
  triggers it (GNOME Settings, KVM re-syncs, this app alike); on Wayland
  the Shell crash ends the session. **Fix: MR
  [!5151](https://gitlab.gnome.org/GNOME/mutter/-/merge_requests/5151),
  shipped in Mutter 50.3** — on Fedora 44: `sudo dnf update mutter
  gnome-shell`, then log out and back in. The app's safety model means a
  crash never persists a bad layout: `monitors.xml` is only written after
  the user confirms in the Shell dialog.
* **Wayland-only.** The X11 code paths of Mutter were removed in GNOME 50;
  this app targets the Wayland session only.
* **GNOME-only.** The backend is Mutter's DisplayConfig; other compositors
  (wlroots, KDE) use different protocols. The core crate is
  compositor-independent by design, so other backends are possible later.
* **Version sensitivity.** Wire format and semantics verified against
  Mutter 50.2 (identical to 49.x). Older Mutters (≤ 47) lack `color-mode`/
  `rgb-range`; the app only sends what the server advertised, but has not
  been tested against anything older than 49.

## Features

* **HDR (BT.2100), SDR-native color mode, and RGB range are read-only** in
  v0.1 (shown in Details). Mutter 50.2 applies these values without
  validating them; exposing writable controls safely needs testable HDR
  hardware end-to-end. Underscanning is likewise supported by the code but
  hidden because no connected monitor supports it.
* **VRR** appears as separate `…+vrr` modes in the refresh-rate list rather
  than a dedicated toggle.
* **No live thumbnails** in monitor cards. Wayland rightly forbids silent
  capture; doing this properly requires an XDG Desktop Portal ScreenCast
  session + PipeWire stream per monitor with a visible "sharing" indicator.
  Deferred; the cards show name/number/mode/status instead.
* **No automatic profile application on hot-plug** (by design for v0.1 —
  predictability first; profiles are applied manually).
* **Monitor labels**: GNOME Shell restricts `ShowMonitorLabels` to an
  allowlist of D-Bus senders (essentially GNOME Settings), so third-party
  apps receive `AccessDenied`. Identify therefore tries the Shell API and
  automatically falls back to its own identification screens — one brief
  fullscreen window per monitor with a large number (Mutter backs
  fullscreen surfaces with black, so a floating transparent badge is not
  possible). The automatic show-on-focus behaviour only uses the Shell
  path and is effectively inactive where the allowlist applies.
* **Backlight, night light, gamma/CTM** are out of scope (different APIs;
  the connected monitors expose no backlight anyway).
* **Canvas accessibility is partial.** The arrangement canvas exposes a
  live accessible description and full keyboard control (Tab/Shift+Tab
  cycles displays, arrows move the selection), and every setting is
  reachable through the standard sidebar widgets — but the monitor cards
  are canvas drawings, not individual accessible objects, so screen
  readers cannot enumerate them directly. Planned for v0.2.
* **No translations yet.** All strings are English; gettext integration is
  planned before the UI grows further.
* **Icon provenance.** The application icon was supplied by the user and
  originates from SVG Repo (per its file header). SVG Repo hosts icons
  under various licenses — verify the specific icon's license before wider
  distribution.
* **Diagnostics content.** `monitor-layout-ctl diagnostics` includes
  monitor EDID identity (needed for support); `--redact` replaces serials
  with stable pseudonyms, drops unknown properties, and omits friendly
  names.

## Mechanics worth knowing

* Mutter requires the layout to be gap-free (edge-connected with positive
  shared borders — corner contact does not count), non-overlapping, origin
  at (0,0), exactly one primary. The editor enforces/repairs this
  continuously (drop-settling, normalization) and explains residual
  problems in the banner.
* Mirror groups need equal mode *resolutions* on every member (refresh may
  differ). The Resolution list for a group only offers sizes every member
  supports; scales only those valid for every member's chosen mode.
* Mode IDs are snapshot-scoped strings; profiles therefore store
  width/height/refresh and re-match with a 0.05 Hz tolerance against the
  live mode list.
* The KVM heuristic (vendor/serial/size plausibility) can mislabel unusual
  hardware — the Details expander has a manual override switch, and the
  label is cosmetic only.
* Two monitors with byte-identical EDIDs (vendor+product+serial) are
  disambiguated by connector hint; if that also fails, profile application
  reports ambiguity instead of guessing.
