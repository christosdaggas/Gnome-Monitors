# Testing

## Automated suite

`cargo test --workspace` — 92 tests, all passing at release time. This
includes the hardening added after external review: policy-aware validation
(global scale, physical-mode integer scales, mirroring support, leased
monitors, color-mode capability), transactional mirror merges with a
single-primary invariant, and profile protection for newly connected
monitors.

### Unit tests (display-core, 65)

* **Geometry**: overlap vs edge-touch, Mutter 50 adjacency (corner contact
  is not adjacent), bounding boxes.
* **Transforms**: wire values 0–7 (incl. the 6/7 flipped ordering per the
  interface XML), dimension swapping, serde names.
* **Modes**: refresh-rate comparisons (59.94 ≠ 59.95 ≠ 60.00), scale
  support + closest-scale snapping, GNOME-style formatting (refresh, scale
  percent truncation, aspect ratios).
* **Identity**: full-EDID vs model vs connector matching, identical models
  distinguished by serial, connector fallback without EDID, synthetic-serial
  detection (KVM EDIDs).
* **Monitors**: KVM heuristic (flags the Lontium device, spares real
  panels), best-mode selection, resolution ordering.
* **Validation**: valid partial-mirror layout, gaps, overlaps, missing /
  multiple primary, unknown connector / mode, unsupported scale, duplicate
  monitor across groups, mirror resolution mismatch, empty layout,
  rotation-aware sizes, normalization (offset and negative origins).
* **Mirroring**: 4K mirror with mixed refresh (LG 59.997 + KVM 30.000),
  no-common-resolution explanation, disjoint scale sets, three-way mirror.
* **Layout ops**: `build_mirror_layout` for all four sides with validity +
  primary preservation; `merge_into_mirror` / `split_from_mirror`
  round-trip, middle-monitor merge adjacency, clean failure on unknown
  monitors.
* **Snapping**: edge snap + guides, threshold behavior, gap closing,
  overlap resolution, nearest-side settling with multiple monitors.
* **Profiles**: capture → resolve after a connector swap (the real
  DP-5/6 → DP-7/8 scenario), missing monitor with same-model hint,
  duplicate-EDID disambiguation via connector hint then ambiguity report,
  EDID-less connector fallback, refresh tolerance matching, serde
  round-trip, unknown-field tolerance (forward migration).
* **Prefs / store**: alias persistence and precedence, KVM override,
  atomic JSON round-trip, corrupt-file errors.

### Backend tests (mutter-backend, 10 + fixtures)

* Parsing a full raw state (properties, color modes, current-mode
  resolution into assignments) and both failure modes (missing current
  mode, invalid transform).
* **Unknown D-Bus properties are preserved, never fatal** (fixture 13).
* Serialization of a partial-mirror layout to the exact wire tuples
  (optional keys only when set; `layout-mode` only when supported).
* Error classification for every observed Mutter message.
* All 13 fixture topologies validate and round-trip through JSON.

### Integration tests (fake DisplayConfig service, 8)

A fake `org.gnome.Mutter.DisplayConfig` served over a **peer-to-peer zbus
connection** (no session bus needed) with Mutter 50.2's verbatim error
strings, exercised through the *real* `MutterBackend`:

state retrieval/parsing · verify has no side effects · temporary apply
updates state + emits `MonitorsChanged` (received through the backend's
event stream) · persistent apply recorded · stale serial → classified
rejection · unknown connector → classified rejection · missing primary →
classified rejection · full rollback round-trip with a fresh serial.

### Watchdog tests (display-revert-helper, 4)

Process-level: CONFIRM and CANCEL exit promptly without touching D-Bus,
garbage control documents fail fast, unknown lines are ignored. (The
restore path itself is the same backend code covered by the fake-service
rollback test, and was additionally exercised live.)

## Live verification on the target machine (2026-07-19)

Read-only and verify-mode checks ran freely; **applies happened only with
explicit user consent** after a Shell-crash incident (see below).

* `monitor-layout-ctl state` — parses the real three-monitor topology,
  detects the KVM, HDR, VRR.
* `monitor-layout-ctl watch` — received `MonitorsChanged` for real changes.
* `verify-mirror` for left/below placements — Mutter accepted our wire
  format.
* Verify probes of invalid layouts — Mutter rejected with the documented
  errors (gap, overlap, two primaries, no primary).
* `test-apply --noop` — capture → verify → temporary apply → hold →
  restore → verified restoration. The apply/restore worked, **and** the
  mode-set triggered a GNOME Shell segfault moments later (a pre-existing
  platform bug on this machine — identical crashes in the journal hours
  before this project ran; details in `docs/safety-and-rollback.md`).
  Consequence: automated live applies are disabled; the manual matrix below
  is user-driven.

## Manual test matrix

Status: ☑ done during development · ☐ pending user runs (each involves a
mode-set, i.e. screens may blink and — on this machine — the Shell may
crash; nothing is persisted without the Shell confirmation).

* ☑ App shows all three displays with correct names/modes/primary
* ☑ Hot-refresh via MonitorsChanged (external gdctl change reflected)
* ☑ Mirror group creation/split in the editor, all four placements,
  validation banner for gaps/overlaps (editor-level, no apply)
* ☑ Identify overlays on all three screens (Shell labels)
* ☑ Profile save / load / rename / duplicate / delete
* ☑ App killed during preview → watchdog restored (validated at the
  process level + fake service; live equivalent is the noop test-apply)
* ☐ Apply: KVM+LG mirrored, LG2 extended (left/right/above/below)
* ☐ Primary on mirror group vs on the independent display
* ☐ Resolution / refresh / scale / rotation changes via Apply
* ☐ KVM disconnect/reconnect during editing and during preview
* ☐ Suspend/resume, lock/unlock, logout/login, reboot persistence
* ☐ Preview timeout auto-revert on live hardware

## Quality gates (all green at release)

`cargo fmt --all -- --check` · `cargo clippy --workspace --all-targets
--all-features` (zero warnings, with `unwrap_used`/`expect_used`/`todo`
denied) · `cargo test --workspace` · `cargo build --release --workspace` ·
`desktop-file-validate` · `appstreamcli validate` (pass; one pedantic
note).
