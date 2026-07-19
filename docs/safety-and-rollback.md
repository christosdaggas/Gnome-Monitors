# Safety and Rollback

Changing monitor configurations can black out every display, and on the
audited machine GNOME Shell has segfaulted during mode-sets (killing the
whole Wayland session). The design therefore assumes the worst at every
step.

## Ground rules

1. **Nothing is ever applied without an explicit user action** (the Apply
   or preview buttons, behind a warning dialog).
2. **Every apply is preceded by a verify-mode call** (`method=0`), which
   runs Mutter's complete validation *including the CRTC/plane assignment
   dry-run* with zero side effects.
3. **The application never writes `monitors.xml`.** Only Mutter writes it,
   and only after the user confirms in GNOME Shell's dialog.
4. Before any apply: fresh `GetCurrentState`, layout normalized to (0,0),
   client-side validation (same rules as Mutter 50.2), serial taken from
   that fresh state. Stale serials are detected server-side and surface as
   a friendly "configuration changed, try again" message after a refresh.

## The Apply flow (persistent) — compositor-guarded

Clicking **Apply…** (after the warning dialog) performs verify + apply with
`method=2` (persistent). This is exactly what GNOME Settings does, and it
delegates the safety net to the compositor:

* Mutter arms its own **20-second one-shot revert timer**
  (`DEFAULT_DISPLAY_CONFIGURATION_TIMEOUT`, meta-monitor-manager.c:67) and
  emits `confirm-display-change`.
* **GNOME Shell** shows the *"Keep these display settings?"* dialog
  (Keep Changes / Revert Settings, Esc = revert).
* Keep → Mutter cancels the timer and writes `monitors.xml`. Revert or
  timeout → Mutter itself re-applies the previous configuration.

Because the timer and the revert live **inside the compositor**, this path
survives anything that can happen to this application — including the app
crashing, the terminal closing, or the session manager killing processes.
It is the most robust rollback available on GNOME, which is why it is the
default. The countdown dialog is intentionally *not* duplicated in-app for
this path (GNOME Settings doesn't either; two countdown dialogs would race
each other).

Note for KVM users: don't rely on the KVM's picture to judge a change — the
KVM re-syncs during mode-sets and may show nothing for a few seconds. The
Shell dialog appears on the primary display; Esc always reverts.

## The Preview flow (temporary) — triple-guarded

**Try for N seconds** (default 30, configurable via `confirm_seconds` in
`~/.config/monitor-layout/prefs.json`) applies with `method=1` (temporary),
which Mutter never persists. Three independent guards:

1. **In-app countdown dialog** with Keep Configuration / Revert; timeout or
   Esc reverts by re-applying the snapshot (fresh serial). Keeping promotes
   the layout with a persistent apply — which goes through Mutter's own
   confirm flow above.
2. **The watchdog process** (`monitor-layout-revert-helper`), spawned
   *before* the temporary apply with the rollback snapshot on stdin:
   * parent sends `CONFIRM`/`CANCEL` → helper exits, does nothing;
   * stdin EOF (the UI **crashed**) → helper restores immediately;
   * timeout (UI countdown + 10 s grace) with no message (UI **hung**) →
     helper restores.
   The helper re-reads the current state first and does nothing if the
   configuration already matches the snapshot (so it never double-applies
   or fights the main app), and retries once on serial races.
3. **Session fallback**: temporary configurations vanish on re-login, and a
   `pending-preview.json` marker under `$XDG_STATE_HOME/monitor-layout`
   makes the next app start report an unclean preview.

## Failure handling

* Verify rejects → nothing was changed; the dialog explains why (friendly
  message + raw Mutter error in the expander).
* Temporary apply fails → nothing changed server-side; watchdog is stood
  down; marker removed.
* In-app revert fails (e.g. D-Bus hiccup) → the watchdog is *left armed*
  and its stdin closed, so it performs the restore independently.
* Monitor disconnects mid-preview → `MonitorsChanged` triggers a refresh;
  restore uses connector+mode of the snapshot and surfaces a clear error if
  the monitor is truly gone (re-login remains the final fallback).
* Every apply/revert/watchdog action is logged via `tracing`
  (`RUST_LOG=info` or `-v` on the CLI).

## Why applies crashed GNOME Shell on Mutter 50.2

During testing, monitor reconfigurations (including a no-op re-apply)
segfaulted GNOME Shell — eight identical coredumps in one day, several
predating this project (KVM re-syncs trigger it too). All share one stack:
`meta_window_move_resize_internal` via the deferred window-relocation
queue. This is upstream Mutter issue
[#4891](https://gitlab.gnome.org/GNOME/mutter/-/work_items/4891) — a stale
`target_monitor` on DESKTOP/override-redirect windows NULL-derefs after a
logical monitor disappears — a regression in the 50.x "target monitor"
rework, **fixed by MR !5151 in Mutter 50.3** (Fedora 44:
`sudo dnf update mutter gnome-shell`, then re-login). The application's
safety model made every crash lossless: nothing is written to
`monitors.xml` until the user confirms in the Shell dialog, so the session
always came back with the old configuration. See
`docs/known-limitations.md`.
