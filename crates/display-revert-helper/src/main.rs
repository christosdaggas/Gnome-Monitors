//! `monitor-layout-revert-helper` — an out-of-process rollback watchdog.
//!
//! The main application spawns this helper *before* applying a temporary
//! display configuration and feeds it a control document on stdin:
//!
//! ```json
//! {"timeout_seconds": 40, "snapshot": { …ApplySnapshot… }}
//! ```
//!
//! Then:
//! * a `CONFIRM` or `CANCEL` line on stdin → the parent handled the outcome;
//!   the helper exits without touching anything;
//! * stdin reaching EOF → the parent died while a temporary configuration
//!   was live; the helper restores the snapshot immediately;
//! * `timeout_seconds` elapsing with no message → the parent hung; the
//!   helper restores the snapshot.
//!
//! Restoration is applied with the TEMPORARY method (never persistent) using
//! a fresh configuration serial, retrying once if the serial races. The
//! helper is deliberately tiny and independent from GTK so that a UI crash
//! cannot take the rollback path down with it.

use std::io::BufRead;
use std::time::{Duration, Instant};

use anyhow::Context;
use display_core::state::{ApplyMethod, ApplySnapshot};
use mutter_backend::{BackendError, DisplayBackend, MutterBackend, RejectionKind};
use serde::Deserialize;
use tracing::{error, info, warn};

#[derive(Debug, Deserialize)]
struct Control {
    timeout_seconds: u64,
    snapshot: ApplySnapshot,
    /// The layout the preview applied. When present, restoration only
    /// happens while the compositor still shows this layout — if a third
    /// party changed the configuration meanwhile, the watchdog must not
    /// overwrite that newer configuration.
    #[serde(default)]
    expected: Option<display_core::DisplayLayout>,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let stdin = std::io::stdin();
    let mut first_line = String::new();
    stdin
        .lock()
        .read_line(&mut first_line)
        .context("reading control document from stdin")?;
    let control: Control = serde_json::from_str(&first_line).context("parsing control document")?;
    info!(
        timeout = control.timeout_seconds,
        logical_displays = control.snapshot.layout.logical_displays.len(),
        "watchdog armed"
    );

    let (sender, receiver) = std::sync::mpsc::channel::<Option<String>>();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            match line {
                Ok(line) => {
                    if sender.send(Some(line)).is_err() {
                        return;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = sender.send(None); // EOF
    });

    let deadline = Instant::now() + Duration::from_secs(control.timeout_seconds);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(remaining) {
            Ok(Some(line)) => match line.trim() {
                "CONFIRM" => {
                    info!("parent confirmed the new configuration; exiting");
                    return Ok(());
                }
                "CANCEL" => {
                    info!("parent reverted on its own; exiting");
                    return Ok(());
                }
                other => warn!(?other, "ignoring unknown control line"),
            },
            Ok(None) => {
                warn!("parent process disappeared; restoring previous configuration");
                return restore(&control.snapshot, control.expected.as_ref());
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                warn!("confirmation timeout expired; restoring previous configuration");
                return restore(&control.snapshot, control.expected.as_ref());
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                warn!("stdin reader ended; restoring previous configuration");
                return restore(&control.snapshot, control.expected.as_ref());
            }
        }
    }
}

fn restore(
    snapshot: &ApplySnapshot,
    expected: Option<&display_core::DisplayLayout>,
) -> anyhow::Result<()> {
    futures_lite::future::block_on(async {
        let backend = MutterBackend::connect()
            .await
            .context("connecting to Mutter")?;
        for attempt in 1..=3u32 {
            let state = backend
                .current_state()
                .await
                .context("fetching fresh state")?;
            // If someone else already restored it (e.g. the main app raced
            // us, or Mutter's own revert fired), do nothing.
            if state.layout == snapshot.layout {
                info!("configuration already matches the snapshot; nothing to do");
                return Ok(());
            }
            // If the configuration is neither the snapshot nor the preview we
            // were guarding, a third party changed it meanwhile — never
            // overwrite a newer configuration.
            if let Some(expected) = expected
                && state.layout != *expected
            {
                warn!(
                    "configuration was changed by someone else during the preview; leaving it untouched"
                );
                return Ok(());
            }
            match backend
                .apply(state.serial, &snapshot.layout, ApplyMethod::Temporary)
                .await
            {
                Ok(()) => {
                    info!("previous display configuration restored");
                    return Ok(());
                }
                Err(BackendError::Rejected {
                    kind: RejectionKind::StaleSerial,
                    ..
                }) if attempt < 3 => {
                    warn!(attempt, "serial raced; retrying restore");
                }
                Err(e) => {
                    error!(error = %e, "restoration failed");
                    return Err(e.into());
                }
            }
        }
        anyhow::bail!("restoration kept racing with configuration changes");
    })
}
