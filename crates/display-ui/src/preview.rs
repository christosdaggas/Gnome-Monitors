//! Temporary-preview flow with an out-of-process rollback watchdog.
//!
//! Unlike the main Apply flow (persistent method + Mutter's own compositor
//! revert), previewing applies the layout with the TEMPORARY method and
//! guards it three ways:
//!
//! 1. an in-app countdown dialog (configurable seconds) that reverts on
//!    timeout or on "Revert";
//! 2. the `monitor-layout-revert-helper` watchdog process, which restores
//!    the previous configuration if this app crashes or hangs;
//! 3. Mutter never writes temporary configurations to disk, so a re-login
//!    always returns to the saved configuration.

use std::cell::RefCell;
use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::rc::Rc;

use adw::prelude::*;
use display_core::state::{ApplyMethod, ApplySnapshot};
use gtk::glib;
use mutter_backend::DisplayBackend;
use serde::Serialize;
use tracing::{info, warn};

use crate::{UiCtx, refresh};

#[derive(Serialize)]
struct Control<'a> {
    timeout_seconds: u64,
    snapshot: &'a ApplySnapshot,
    /// The layout the preview is about to apply; the watchdog only restores
    /// when the screen still shows this (or already shows the snapshot).
    expected: &'a display_core::DisplayLayout,
}

pub struct PreviewSession {
    helper: Option<Child>,
    snapshot: ApplySnapshot,
    finished: bool,
}

thread_local! {
    static ACTIVE: RefCell<Option<PreviewSession>> = const { RefCell::new(None) };
}

fn helper_path() -> Option<std::path::PathBuf> {
    // Next to our own binary (both in target/debug and when installed).
    let sibling = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("monitor-layout-revert-helper")));
    if let Some(path) = sibling.filter(|p| p.exists()) {
        return Some(path);
    }
    // Fall back to PATH.
    Some(std::path::PathBuf::from("monitor-layout-revert-helper"))
}

fn spawn_helper(
    snapshot: &ApplySnapshot,
    expected: &display_core::DisplayLayout,
    timeout_seconds: u64,
) -> Option<Child> {
    let path = helper_path()?;
    let mut child = match Command::new(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            warn!("could not start revert helper {}: {e}", path.display());
            return None;
        }
    };
    let control = Control {
        timeout_seconds,
        snapshot,
        expected,
    };
    let ok = child
        .stdin
        .as_mut()
        .and_then(|stdin| {
            serde_json::to_string(&control)
                .ok()
                .and_then(|json| writeln!(stdin, "{json}").ok())
        })
        .is_some();
    if !ok {
        warn!("could not hand snapshot to revert helper");
        let _ = child.kill();
        return None;
    }
    info!("revert watchdog armed ({timeout_seconds} s)");
    Some(child)
}

fn signal_helper(session: &mut PreviewSession, message: &str) {
    if let Some(child) = session.helper.as_mut() {
        if let Some(stdin) = child.stdin.as_mut() {
            let _ = writeln!(stdin, "{message}");
            let _ = stdin.flush();
        }
        let _ = child.wait();
    }
    session.helper = None;
}

fn marker_path() -> Option<std::path::PathBuf> {
    display_core::paths::state_dir().map(|d| d.join("pending-preview.json"))
}

/// Starts the preview: verify → temporary apply → countdown dialog.
pub async fn start(ctx: &Rc<UiCtx>) {
    if ctx.state.applying.get() {
        return;
    }
    let backend = ctx.state.backend.borrow().clone();
    let Some(backend) = backend else { return };

    // Genuinely fresh state (also aborts on topology changes / new problems).
    let Some((serial, layout)) = crate::fresh_state_for_send(ctx, &backend).await else {
        return;
    };
    let snapshot = {
        let current = ctx.state.current.borrow();
        let Some(state) = current.as_ref() else {
            return;
        };
        state.to_apply_snapshot()
    };
    let confirm_seconds = u64::from(ctx.state.prefs.borrow().confirm_seconds.max(5));

    ctx.state.applying.set(true);
    ctx.update_status();

    // Verify first — never touch anything Mutter would reject.
    if let Err(e) = backend.apply(serial, &layout, ApplyMethod::Verify).await {
        ctx.state.applying.set(false);
        ctx.update_status();
        crate::error_dialog(ctx, "The Configuration Cannot Be Previewed", &e);
        return;
    }

    // Fail closed: without the marker AND an armed watchdog, no preview.
    let marker_ok = marker_path()
        .map(|marker| display_core::store::write_json_atomic(&marker, &snapshot).is_ok())
        .unwrap_or(false);
    let helper = spawn_helper(&snapshot, &layout, confirm_seconds + 10);
    let unguarded_override = std::env::var_os("MONITOR_LAYOUT_UNGUARDED_PREVIEW").is_some();
    if (!marker_ok || helper.is_none()) && !unguarded_override {
        ctx.state.applying.set(false);
        ctx.update_status();
        let dialog = adw::AlertDialog::builder()
            .heading("Preview Unavailable")
            .body(
                "The rollback watchdog (monitor-layout-revert-helper) could not be \
                 started, so a crash during the preview could leave the temporary \
                 configuration active. Nothing was changed.\n\nUse Apply instead — it \
                 is protected by GNOME's own automatic revert — or reinstall the \
                 application so the helper is available.",
            )
            .build();
        dialog.add_response("ok", "_OK");
        dialog.set_default_response(Some("ok"));
        dialog.present(Some(&ctx.window));
        return;
    }

    match backend.apply(serial, &layout, ApplyMethod::Temporary).await {
        Ok(()) => {
            info!("temporary preview applied");
            ACTIVE.with(|active| {
                *active.borrow_mut() = Some(PreviewSession {
                    helper,
                    snapshot,
                    finished: false,
                });
            });
            countdown_dialog(ctx, confirm_seconds);
        }
        Err(e) => {
            ACTIVE.with(|active| {
                let mut session = PreviewSession {
                    helper,
                    snapshot,
                    finished: true,
                };
                signal_helper(&mut session, "CANCEL");
                *active.borrow_mut() = None;
            });
            if let Some(marker) = marker_path() {
                let _ = std::fs::remove_file(marker);
            }
            ctx.state.applying.set(false);
            ctx.update_status();
            crate::error_dialog(ctx, "The Preview Could Not Be Applied", &e);
        }
    }
}

fn countdown_dialog(ctx: &Rc<UiCtx>, seconds: u64) {
    let dialog = adw::AlertDialog::builder()
        .heading("Keep This Configuration?")
        .body(format!(
            "Reverting to the previous configuration in {seconds} seconds."
        ))
        .build();
    dialog.add_response("revert", "_Revert");
    dialog.add_response("keep", "_Keep Configuration");
    dialog.set_response_appearance("keep", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("revert"));
    dialog.set_close_response("revert");

    // Tick every second; auto-close (= revert) at zero.
    let remaining = Rc::new(std::cell::Cell::new(seconds));
    let tick: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
    let source = {
        let dialog = dialog.clone();
        let remaining = Rc::clone(&remaining);
        glib::timeout_add_seconds_local(1, move || {
            let left = remaining.get().saturating_sub(1);
            remaining.set(left);
            if left == 0 {
                dialog.close();
                return glib::ControlFlow::Break;
            }
            dialog.set_body(&format!(
                "Reverting to the previous configuration in {left} seconds."
            ));
            glib::ControlFlow::Continue
        })
    };
    *tick.borrow_mut() = Some(source);

    let ctx2 = Rc::clone(ctx);
    dialog.connect_response(None, move |_, response| {
        if let Some(source) = tick.borrow_mut().take() {
            source.remove();
        }
        let keep = response == "keep";
        let ctx2 = Rc::clone(&ctx2);
        glib::spawn_future_local(async move {
            finish(&ctx2, keep).await;
        });
    });
    dialog.present(Some(&ctx.window));
}

async fn finish(ctx: &Rc<UiCtx>, keep: bool) {
    let Some(mut session) = ACTIVE.with(|active| active.borrow_mut().take()) else {
        return;
    };
    if session.finished {
        return;
    }
    session.finished = true;

    let backend = ctx.state.backend.borrow().clone();
    if keep {
        // The parent handles persistence: tell the watchdog to stand down,
        // then promote the (already live) layout to persistent so GNOME's
        // standard confirmation stores it in monitors.xml.
        signal_helper(&mut session, "CONFIRM");
        if let Some(backend) = backend {
            let promoted = {
                match backend.current_state().await {
                    Ok(state) => Some((state.serial, state.layout.clone())),
                    Err(e) => {
                        warn!("could not re-read state to persist preview: {e}");
                        None
                    }
                }
            };
            if let Some((serial, layout)) = promoted {
                match backend
                    .apply(serial, &layout, ApplyMethod::Persistent)
                    .await
                {
                    Ok(()) => ctx.toast(
                        "Kept — confirm “Keep Changes” in the system dialog to save it permanently",
                    ),
                    Err(e) => {
                        ctx.toast(&format!("Kept for this session, but saving it failed: {e}"))
                    }
                }
            }
        }
    } else {
        // Revert ourselves; the watchdog is told to stand down either way
        // (it would only act if we crashed before getting here).
        if let Some(backend) = backend {
            let result = async {
                let state = backend.current_state().await?;
                backend
                    .apply(
                        state.serial,
                        &session.snapshot.layout,
                        ApplyMethod::Temporary,
                    )
                    .await
            }
            .await;
            match result {
                Ok(()) => ctx.toast("The previous display configuration was restored"),
                Err(e) => {
                    warn!("in-app revert failed, leaving it to the watchdog: {e}");
                    // Do NOT stand the watchdog down — let it try.
                    if let Some(child) = session.helper.take() {
                        drop(child); // closing stdin (EOF) triggers the helper
                    }
                    ctx.toast("Revert failed here — the watchdog is restoring the layout");
                    ctx.state.applying.set(false);
                    refresh(ctx, false).await;
                    return;
                }
            }
        }
        signal_helper(&mut session, "CANCEL");
    }

    if let Some(marker) = marker_path() {
        let _ = std::fs::remove_file(marker);
    }
    ctx.state.applying.set(false);
    refresh(ctx, false).await;
}

/// Startup check: a leftover marker means a previous preview did not finish
/// cleanly (the watchdog should have restored the configuration).
pub fn startup_check(ctx: &Rc<UiCtx>) {
    if let Some(marker) = marker_path()
        && marker.exists()
    {
        ctx.toast("A previous preview ended unexpectedly; the saved configuration was restored");
        let _ = std::fs::remove_file(marker);
    }
}
