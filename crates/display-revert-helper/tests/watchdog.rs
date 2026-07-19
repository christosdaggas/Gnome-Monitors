//! Process-level tests for the revert watchdog.
//!
//! The CONFIRM/CANCEL paths never touch D-Bus, so they are tested
//! unconditionally. The restore path needs a session bus and is therefore
//! exercised in `mutter-backend`'s fake-service tests (same code path via
//! `MutterBackend`) and, on the target machine, by the manual test matrix.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn helper() -> Command {
    Command::new(env!("CARGO_BIN_EXE_monitor-layout-revert-helper"))
}

const CONTROL: &str = r#"{"timeout_seconds": 30, "snapshot": {"captured_serial": 1, "layout": {"layout_mode": "logical", "logical_displays": [{"x": 0, "y": 0, "scale": 2.0, "transform": "normal", "primary": true, "monitors": [{"connector": "DP-7", "mode_id": "3840x2160@59.997"}]}]}}}"#;

#[test]
fn confirm_exits_cleanly_without_restoring() {
    let mut child = helper()
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let stdin = child.stdin.as_mut().unwrap();
        writeln!(stdin, "{CONTROL}").unwrap();
        writeln!(stdin, "CONFIRM").unwrap();
    }
    let start = Instant::now();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "helper must exit promptly on CONFIRM"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("parent confirmed"), "stderr: {stderr}");
}

#[test]
fn cancel_exits_cleanly_without_restoring() {
    let mut child = helper()
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let stdin = child.stdin.as_mut().unwrap();
        writeln!(stdin, "{CONTROL}").unwrap();
        writeln!(stdin, "CANCEL").unwrap();
    }
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("parent reverted"));
}

#[test]
fn garbage_control_document_fails_fast() {
    let mut child = helper()
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let stdin = child.stdin.as_mut().unwrap();
        writeln!(stdin, "this is not json").unwrap();
    }
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success(), "invalid control must be an error");
}

#[test]
fn unknown_lines_are_ignored_until_confirm() {
    let mut child = helper()
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let stdin = child.stdin.as_mut().unwrap();
        writeln!(stdin, "{CONTROL}").unwrap();
        writeln!(stdin, "HEARTBEAT?").unwrap();
        writeln!(stdin, "CONFIRM").unwrap();
    }
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
}
