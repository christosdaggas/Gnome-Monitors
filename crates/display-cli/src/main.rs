//! `monitor-layout-ctl` — diagnostic and proof-of-concept CLI.
//!
//! This is the tool required by the project's PoC phase: it reads the live
//! Mutter state, watches for monitor changes, verifies a partial-mirror
//! configuration without applying it, and can perform a safe *temporary*
//! apply with automatic restoration. It never applies persistently.

use std::io::Write as _;
use std::time::Duration;

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use display_core::layout_ops::{Side, build_mirror_layout};
use display_core::prefs::AppPrefs;
use display_core::state::ApplyMethod;
use display_core::validation::{is_appliable, normalize, validate_state};
use display_core::{DisplayLayout, DisplayState, format_refresh, format_scale_percent};
use futures_util::StreamExt;
use mutter_backend::{BackendError, DisplayBackend, MutterBackend};
use tracing::info;

#[derive(Parser)]
#[command(
    name = "monitor-layout-ctl",
    about = "Diagnostics and safe configuration testing for GNOME displays (Mutter D-Bus)",
    version
)]
struct Cli {
    /// Increase log verbosity (-v info, -vv debug, -vvv trace).
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the current display state.
    State {
        /// Emit the full parsed state as JSON.
        #[arg(long)]
        json: bool,
        /// Also list every mode of every monitor.
        #[arg(long)]
        modes: bool,
    },
    /// Listen for MonitorsChanged and print fresh summaries (hot-plug test).
    Watch {
        /// Stop after this many seconds (0 = run until interrupted).
        #[arg(long, default_value_t = 0)]
        timeout: u64,
    },
    /// Validate a partial-mirror layout with Mutter's verify method.
    /// Applies nothing.
    VerifyMirror {
        /// Connectors that should mirror each other (repeat or comma-separate).
        #[arg(long, value_delimiter = ',', required = true)]
        members: Vec<String>,
        /// Where the mirror group goes relative to the other displays.
        #[arg(long, default_value = "left")]
        side: SideArg,
    },
    /// Safely test-apply a configuration TEMPORARILY, then restore the
    /// original automatically. Nothing is written to disk by the compositor.
    TestApply {
        /// Re-apply the *current* configuration (a visually invisible no-op
        /// that still exercises the full apply path). Default when no
        /// members are given.
        #[arg(long)]
        noop: bool,
        /// Mirror these connectors as the test configuration.
        #[arg(long, value_delimiter = ',')]
        members: Vec<String>,
        /// Where the mirror group goes.
        #[arg(long, default_value = "left")]
        side: SideArg,
        /// Seconds to hold the test configuration before restoring.
        #[arg(long, default_value_t = 8)]
        hold: u64,
    },
    /// Print a diagnostics report (also see --json). Includes monitor
    /// identity by default; --redact replaces EDID serials with stable
    /// pseudonyms and omits friendly names.
    Diagnostics {
        #[arg(long)]
        json: bool,
        /// Replace EDID serials with pseudonyms and omit friendly names.
        #[arg(long)]
        redact: bool,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum SideArg {
    Left,
    Right,
    Above,
    Below,
}

impl From<SideArg> for Side {
    fn from(value: SideArg) -> Side {
        match value {
            SideArg::Left => Side::Left,
            SideArg::Right => Side::Right,
            SideArg::Above => Side::Above,
            SideArg::Below => Side::Below,
        }
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let level = match cli.verbose {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(level)),
        )
        .with_writer(std::io::stderr)
        .init();

    futures_lite::future::block_on(run(cli.command))
}

async fn run(command: Command) -> anyhow::Result<()> {
    let backend = MutterBackend::connect()
        .await
        .context("connecting to org.gnome.Mutter.DisplayConfig on the session bus")?;

    match command {
        Command::State { json, modes } => {
            let state = backend.current_state().await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&state)?);
            } else {
                print_summary(&state, modes);
            }
        }
        Command::Watch { timeout } => {
            let state = backend.current_state().await?;
            print_summary(&state, false);
            println!("\nWatching for monitor changes… (Ctrl+C to stop)");
            let mut events = backend.events().await?;
            let deadline =
                (timeout > 0).then(|| std::time::Instant::now() + Duration::from_secs(timeout));
            enum Wake {
                Event(Option<mutter_backend::BackendEvent>),
                Timeout,
            }
            loop {
                let next = events.next();
                let wake = if let Some(deadline) = deadline {
                    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                    if remaining.is_zero() {
                        break;
                    }
                    futures_lite::future::or(async { Wake::Event(next.await) }, async {
                        async_io::Timer::after(remaining).await;
                        Wake::Timeout
                    })
                    .await
                } else {
                    Wake::Event(next.await)
                };
                match wake {
                    Wake::Event(Some(_)) => {
                        println!("\n=== MonitorsChanged ({}) ===", timestamp());
                        let state = backend.current_state().await?;
                        print_summary(&state, false);
                    }
                    Wake::Event(None) | Wake::Timeout => break,
                }
            }
        }
        Command::VerifyMirror { members, side } => {
            let state = backend.current_state().await?;
            let layout = build_mirror_layout(&state, &members, side.into())?;
            print_layout(&state, &layout, "Proposed layout");
            client_validate(&state, &layout)?;
            match backend
                .apply(state.serial, &layout, ApplyMethod::Verify)
                .await
            {
                Ok(()) => println!("\nMutter verify: ACCEPTED (nothing was applied)"),
                Err(e) => report_rejection("Mutter verify: REJECTED", &e)?,
            }
        }
        Command::TestApply {
            noop,
            members,
            side,
            hold,
        } => {
            test_apply(
                &backend,
                noop || members.is_empty(),
                &members,
                side.into(),
                hold,
            )
            .await?;
        }
        Command::Diagnostics { json, redact } => {
            diagnostics(&backend, json, redact).await?;
        }
    }
    Ok(())
}

fn client_validate(state: &DisplayState, layout: &DisplayLayout) -> anyhow::Result<()> {
    let problems = validate_state(state, layout);
    if problems.is_empty() {
        println!("Client-side validation: OK");
    } else if is_appliable(&problems) {
        println!("Client-side validation: OK after normalization ({problems:?})");
    } else {
        for problem in &problems {
            println!("Client-side validation problem: {problem}");
        }
        bail!("the proposed layout is invalid; not sending to Mutter");
    }
    Ok(())
}

fn report_rejection(prefix: &str, error: &BackendError) -> anyhow::Result<()> {
    println!("\n{prefix}");
    println!("  {error}");
    println!("  Technical details: {}", error.technical_details());
    bail!("configuration rejected");
}

/// The safe temporary-apply / rollback proof of concept.
async fn test_apply(
    backend: &MutterBackend,
    noop: bool,
    members: &[String],
    side: Side,
    hold: u64,
) -> anyhow::Result<()> {
    // 1. Fresh state + rollback snapshot.
    let original = backend.current_state().await?;
    let snapshot = original.to_apply_snapshot();
    println!(
        "Captured original configuration (serial {}).",
        original.serial
    );

    // 2. Persist the snapshot so a crash leaves a recovery file behind.
    let marker = display_core::paths::state_dir()
        .map(|d| d.join("poc-pending-revert.json"))
        .context("cannot determine XDG state directory")?;
    display_core::store::write_json_atomic(&marker, &snapshot)?;
    println!("Rollback snapshot written to {}.", marker.display());

    // 3. Build the target configuration.
    let mut target = if noop {
        println!("\nTest configuration: re-applying the CURRENT layout (no visible change).");
        original.layout.clone()
    } else {
        println!("\nTest configuration: mirror {members:?} ({side:?} of the rest).");
        build_mirror_layout(&original, members, side)?
    };
    normalize(&mut target, &original.monitors);
    print_layout(&original, &target, "Target layout");

    // 4. Validate client-side, then with Mutter's verify method.
    client_validate(&original, &target)?;
    if let Err(e) = backend
        .apply(original.serial, &target, ApplyMethod::Verify)
        .await
    {
        report_rejection("Mutter verify: REJECTED — aborting before any change", &e)?;
    }
    println!("Mutter verify: ACCEPTED");

    // 5. Apply temporarily. The compositor never touches monitors.xml for
    //    TEMPORARY, so even a total failure is recoverable by re-login.
    println!("\nApplying TEMPORARILY…");
    if let Err(e) = backend
        .apply(original.serial, &target, ApplyMethod::Temporary)
        .await
    {
        report_rejection("Temporary apply failed (nothing changed)", &e)?;
    }

    // 6. Hold, then restore no matter what happened in between.
    print!("Holding for {hold} s: ");
    for i in (1..=hold).rev() {
        print!("{i}… ");
        std::io::stdout().flush().ok();
        async_io::Timer::after(Duration::from_secs(1)).await;
    }
    println!();

    let restore_result = restore(backend, &snapshot.layout).await;
    match restore_result {
        Ok(()) => {
            let now = backend.current_state().await?;
            let mut expected = snapshot.layout.clone();
            normalize(&mut expected, &now.monitors);
            let mut actual = now.layout.clone();
            normalize(&mut actual, &now.monitors);
            if layouts_equivalent(&expected, &actual) {
                println!(
                    "Original configuration restored and verified (serial {}).",
                    now.serial
                );
                std::fs::remove_file(&marker).ok();
            } else {
                bail!(
                    "restoration applied but the resulting layout differs; rollback snapshot kept at {}",
                    marker.display()
                );
            }
        }
        Err(e) => {
            println!("Restoration failed: {e}");
            println!("Rollback snapshot kept at {}.", marker.display());
            println!("A logout/login will also restore the persistent configuration.");
            return Err(e);
        }
    }
    Ok(())
}

async fn restore(backend: &MutterBackend, layout: &DisplayLayout) -> anyhow::Result<()> {
    println!("Restoring original configuration…");
    // The serial advanced when we applied; fetch a fresh one.
    let fresh = backend.current_state().await?;
    backend
        .apply(fresh.serial, layout, ApplyMethod::Temporary)
        .await
        .context("re-applying the original configuration")?;
    Ok(())
}

/// Order-insensitive layout comparison (Mutter may reorder logical monitors).
fn layouts_equivalent(a: &DisplayLayout, b: &DisplayLayout) -> bool {
    if a.layout_mode != b.layout_mode || a.logical_displays.len() != b.logical_displays.len() {
        return false;
    }
    let key = |l: &display_core::LogicalDisplay| {
        let mut monitors: Vec<(String, String)> = l
            .monitors
            .iter()
            .map(|m| (m.connector.clone(), m.mode_id.clone()))
            .collect();
        monitors.sort();
        (
            l.x,
            l.y,
            format!("{:.4}", l.scale),
            l.transform,
            l.primary,
            monitors,
        )
    };
    let mut ka: Vec<_> = a.logical_displays.iter().map(key).collect();
    let mut kb: Vec<_> = b.logical_displays.iter().map(key).collect();
    ka.sort();
    kb.sort();
    ka == kb
}

fn timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("t+{}s", now.as_secs() % 100_000)
}

fn print_summary(state: &DisplayState, with_modes: bool) {
    let prefs = load_prefs();
    println!(
        "Serial: {}   layout-mode: {:?}",
        state.serial, state.layout.layout_mode
    );
    println!("\nPhysical monitors ({}):", state.monitors.len());
    for monitor in &state.monitors {
        let identity = &monitor.identity;
        let kvm = if prefs.is_kvm(monitor) {
            "  [likely KVM]"
        } else {
            ""
        };
        let hdr = if monitor.supports_hdr() { "  HDR" } else { "" };
        let vrr = if monitor.supports_vrr() { "  VRR" } else { "" };
        println!(
            "  {:8} {} — {} {} (serial {}){kvm}{hdr}{vrr}",
            identity.connector,
            prefs.display_name(monitor),
            identity.vendor,
            identity.product,
            if identity.serial.is_empty() {
                "<none>"
            } else {
                &identity.serial
            },
        );
        match monitor.current_mode() {
            Some(mode) => println!(
                "           current: {}×{} @ {}  (preferred: {})",
                mode.width,
                mode.height,
                format_refresh(mode.refresh_hz),
                monitor
                    .preferred_mode()
                    .map_or("-".into(), |m| m.id.clone()),
            ),
            None => println!("           disabled"),
        }
        if with_modes {
            for mode in &monitor.modes {
                println!(
                    "           mode {:22} scales {:?}{}{}",
                    mode.id,
                    mode.supported_scales,
                    if mode.is_current { "  [current]" } else { "" },
                    if mode.is_preferred {
                        "  [preferred]"
                    } else {
                        ""
                    },
                );
            }
        }
    }
    print_layout(state, &state.layout, "\nLogical displays");
    let disabled = state.disabled_monitors();
    if !disabled.is_empty() {
        println!(
            "\nDisabled monitors: {}",
            disabled
                .iter()
                .map(|m| m.connector())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
}

fn print_layout(state: &DisplayState, layout: &DisplayLayout, title: &str) {
    println!("{title} ({}):", layout.logical_displays.len());
    for (i, logical) in layout.logical_displays.iter().enumerate() {
        let size = logical
            .logical_size(&state.monitors, layout.layout_mode)
            .map_or("?".into(), |(w, h)| format!("{w}×{h}"));
        let members: Vec<String> = logical
            .monitors
            .iter()
            .map(|a| format!("{} [{}]", a.connector, a.mode_id))
            .collect();
        println!(
            "  #{} at ({}, {})  {}  scale {} ({})  transform {}  {}{}{}",
            i + 1,
            logical.x,
            logical.y,
            size,
            logical.scale,
            format_scale_percent(logical.scale),
            logical.transform,
            if logical.primary { "PRIMARY  " } else { "" },
            if logical.is_mirror_group() {
                "MIRROR: "
            } else {
                ""
            },
            members.join(" + "),
        );
    }
}

fn load_prefs() -> AppPrefs {
    display_core::paths::prefs_file()
        .and_then(|p| display_core::store::read_json(&p).ok().flatten())
        .unwrap_or_default()
}

/// Stable pseudonym for a sensitive value (FNV-1a; good enough to correlate
/// reports without revealing the value).
fn pseudonym(tag: &str, value: &str) -> String {
    if value.is_empty() {
        return String::new();
    }
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in value.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{tag}-{:08x}", hash as u32)
}

async fn diagnostics(backend: &MutterBackend, json: bool, redact: bool) -> anyhow::Result<()> {
    let mut state = backend.current_state().await?;
    if redact {
        for monitor in &mut state.monitors {
            monitor.identity.serial = pseudonym("sn", &monitor.identity.serial);
            // Unknown properties may embed identifying strings; drop them.
            monitor.extra.clear();
        }
        // Friendly names live in prefs and are keyed by the real serial, so
        // redacted reports simply fall back to compositor display names.
    }
    let state = state;
    let os = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let pretty = os
        .lines()
        .find_map(|l| l.strip_prefix("PRETTY_NAME="))
        .map(|v| v.trim_matches('"').to_owned())
        .unwrap_or_else(|| "unknown".into());
    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".into());
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "unknown".into());

    if json {
        let report = serde_json::json!({
            "app_version": env!("CARGO_PKG_VERSION"),
            "os": pretty,
            "session_type": session,
            "desktop": desktop,
            "state": state,
        });
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("monitor-layout-ctl {}", env!("CARGO_PKG_VERSION"));
        println!("OS: {pretty}");
        println!("Session: {session} ({desktop})");
        println!();
        print_summary(&state, false);
    }
    info!("diagnostics complete");
    Ok(())
}
