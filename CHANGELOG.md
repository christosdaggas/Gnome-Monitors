# Changelog

## 0.1.0 — 2026-07-19

Initial release. Application ID `com.chrisdaggas.MonitorLayout`; original
app icon (a mirror group beside an extended display).

* Visual layout editor (drag + snap + settle, keyboard nudging, live
  validation) for GNOME Wayland via `org.gnome.Mutter.DisplayConfig`.
* Partial mirroring: any subset of displays as one mirror group, computed
  from real common modes (mixed refresh rates supported), positionable on
  any side of the remaining displays.
* Per-display resolution / refresh (incl. VRR modes) / fractional scaling /
  orientation / enable / primary.
* Numbered identification overlays via GNOME Shell monitor labels
  (automatic while the window is focused).
* Profiles with EDID-identity matching and conservative resolution.
* Safety: verify-before-apply, GNOME's compositor-side keep/revert flow for
  persistent applies, in-app countdown + out-of-process rollback watchdog
  for temporary previews, no direct `monitors.xml` writes ever.
* `monitor-layout-ctl` diagnostics CLI (state / watch / verify-mirror /
  test-apply / diagnostics).
* 87 automated tests including a fake DisplayConfig service exercised over
  peer-to-peer D-Bus.
* RPM packaging with offline (vendored) build script.
