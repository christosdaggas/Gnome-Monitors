//! Integration tests against a fake DisplayConfig service.
//!
//! The fake service implements the subset of `org.gnome.Mutter.DisplayConfig`
//! this application uses, over a private peer-to-peer zbus connection (no
//! session bus or dbus-daemon required). It reproduces Mutter 50.2's error
//! strings so the error-translation layer is exercised end-to-end.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use display_core::Transform;
use display_core::layout::{DisplayLayout, LayoutMode, LogicalDisplay, MonitorAssignment};
use display_core::state::ApplyMethod;
use futures_util::StreamExt;
use mutter_backend::backend::{BackendEvent, DisplayBackend, MutterBackend};
use mutter_backend::error::{BackendError, RejectionKind};
use mutter_backend::proxy::{RawLogicalMonitor, RawMode, RawMonitor, RawState, WireLogicalMonitor};
use zbus::object_server::SignalEmitter;
use zvariant::{OwnedValue, Value};

fn ov<'a>(v: impl Into<Value<'a>>) -> OwnedValue {
    v.into().try_to_owned().unwrap()
}

/// Shared mutable state of the fake compositor.
#[derive(Debug)]
struct FakeState {
    serial: u32,
    /// (connector, current mode id) pairs of the applied config.
    applied: Vec<(String, String)>,
    layout: Vec<RawLogicalMonitor>,
    calls: Vec<(u32, u32)>, // (serial, method)
}

struct FakeDisplayConfig {
    state: Arc<Mutex<FakeState>>,
}

fn mk_mode(id: &str, w: i32, h: i32, hz: f64, current: bool) -> RawMode {
    let mut props = HashMap::new();
    if current {
        props.insert("is-current".to_owned(), ov(true));
    }
    if id.contains("59.997") {
        props.insert("is-preferred".to_owned(), ov(true));
    }
    (id.to_owned(), w, h, hz, 2.0, vec![1.0, 1.5, 2.0], props)
}

fn mk_monitor(connector: &str, serial: &str, current_mode: &str) -> RawMonitor {
    let mut props = HashMap::new();
    props.insert("display-name".to_owned(), ov("Fake 27\""));
    props.insert("is-builtin".to_owned(), ov(false));
    (
        (
            connector.to_owned(),
            "GSM".to_owned(),
            "LG HDR 4K".to_owned(),
            serial.to_owned(),
        ),
        vec![
            mk_mode(
                "3840x2160@59.997",
                3840,
                2160,
                59.997,
                current_mode == "3840x2160@59.997",
            ),
            mk_mode(
                "3840x2160@30.000",
                3840,
                2160,
                30.0,
                current_mode == "3840x2160@30.000",
            ),
            mk_mode(
                "1920x1080@60.000",
                1920,
                1080,
                60.0,
                current_mode == "1920x1080@60.000",
            ),
        ],
        props,
    )
}

const CONNECTORS: [(&str, &str); 3] = [("DP-7", "0xa"), ("DP-8", "0xb"), ("HDMI-1", "0xc")];

impl FakeDisplayConfig {
    fn build_state(&self) -> RawState {
        let state = self.state.lock().unwrap();
        let monitors: Vec<RawMonitor> = CONNECTORS
            .iter()
            .map(|(connector, serial)| {
                let current = state
                    .applied
                    .iter()
                    .find(|(c, _)| c == connector)
                    .map_or("", |(_, id)| id.as_str());
                mk_monitor(connector, serial, current)
            })
            .collect();
        let mut props = HashMap::new();
        props.insert("layout-mode".to_owned(), ov(1u32));
        props.insert("supports-changing-layout-mode".to_owned(), ov(true));
        (state.serial, monitors, state.layout.clone(), props)
    }
}

#[zbus::interface(name = "org.gnome.Mutter.DisplayConfig")]
impl FakeDisplayConfig {
    fn get_current_state(&self) -> RawState {
        self.build_state()
    }

    async fn apply_monitors_config(
        &self,
        serial: u32,
        method: u32,
        logical_monitors: Vec<WireLogicalMonitor>,
        _properties: HashMap<String, OwnedValue>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        let emit = self.apply_inner(serial, method, logical_monitors)?;
        if emit {
            Self::monitors_changed(&emitter)
                .await
                .map_err(|e| zbus::fdo::Error::Failed(format!("could not emit signal: {e}")))?;
        }
        Ok(())
    }

    #[zbus(signal)]
    async fn monitors_changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}

impl FakeDisplayConfig {
    fn apply_inner(
        &self,
        serial: u32,
        method: u32,
        logical_monitors: Vec<WireLogicalMonitor>,
    ) -> zbus::fdo::Result<bool> {
        let mut state = self.state.lock().unwrap();
        if serial != state.serial {
            return Err(zbus::fdo::Error::AccessDenied(
                "The requested configuration is based on stale information".into(),
            ));
        }
        if logical_monitors.is_empty() {
            return Err(zbus::fdo::Error::Failed(
                "Monitors config incomplete".into(),
            ));
        }
        // Minimal Mutter-like validation with verbatim error strings.
        let mut primaries = 0;
        for (_, _, _, _, primary, monitors) in &logical_monitors {
            if *primary {
                primaries += 1;
            }
            for (connector, mode_id, _) in monitors {
                if !CONNECTORS.iter().any(|(c, _)| c == connector) {
                    return Err(zbus::fdo::Error::InvalidArgs(format!(
                        "Invalid connector '{connector}' specified"
                    )));
                }
                if !["3840x2160@59.997", "3840x2160@30.000", "1920x1080@60.000"]
                    .contains(&mode_id.as_str())
                {
                    return Err(zbus::fdo::Error::InvalidArgs(format!(
                        "Invalid mode '{mode_id}' specified"
                    )));
                }
            }
        }
        if primaries == 0 {
            return Err(zbus::fdo::Error::InvalidArgs(
                "Config is missing primary logical".into(),
            ));
        }
        if primaries > 1 {
            return Err(zbus::fdo::Error::InvalidArgs(
                "Config contains multiple primary logical monitors".into(),
            ));
        }

        state.calls.push((serial, method));
        if method == 0 {
            return Ok(false); // verify: no side effects
        }

        state.applied = logical_monitors
            .iter()
            .flat_map(|(.., monitors)| monitors.iter().map(|(c, m, _)| (c.clone(), m.clone())))
            .collect();
        state.layout = logical_monitors
            .iter()
            .map(|(x, y, scale, transform, primary, monitors)| {
                (
                    *x,
                    *y,
                    *scale,
                    *transform,
                    *primary,
                    monitors
                        .iter()
                        .map(|(c, _, _)| {
                            let serial = CONNECTORS
                                .iter()
                                .find(|(cc, _)| cc == c)
                                .map_or("", |(_, s)| s);
                            (
                                c.clone(),
                                "GSM".to_owned(),
                                "LG HDR 4K".to_owned(),
                                serial.to_owned(),
                            )
                        })
                        .collect(),
                    HashMap::new(),
                )
            })
            .collect();
        state.serial += 1;
        Ok(true)
    }
}

/// Boots the fake service + a backend connected to it over a socket pair.
async fn setup() -> (MutterBackend, Arc<Mutex<FakeState>>) {
    let initial_layout: Vec<RawLogicalMonitor> = vec![
        (
            0,
            0,
            2.0,
            0,
            true,
            vec![(
                "DP-7".into(),
                "GSM".into(),
                "LG HDR 4K".into(),
                "0xa".into(),
            )],
            HashMap::new(),
        ),
        (
            1920,
            0,
            2.0,
            0,
            false,
            vec![(
                "DP-8".into(),
                "GSM".into(),
                "LG HDR 4K".into(),
                "0xb".into(),
            )],
            HashMap::new(),
        ),
        (
            3840,
            0,
            2.0,
            0,
            false,
            vec![(
                "HDMI-1".into(),
                "GSM".into(),
                "LG HDR 4K".into(),
                "0xc".into(),
            )],
            HashMap::new(),
        ),
    ];
    let state = Arc::new(Mutex::new(FakeState {
        serial: 1,
        applied: vec![
            ("DP-7".into(), "3840x2160@59.997".into()),
            ("DP-8".into(), "3840x2160@59.997".into()),
            ("HDMI-1".into(), "3840x2160@30.000".into()),
        ],
        layout: initial_layout,
        calls: Vec::new(),
    }));

    let (client_stream, server_stream) = tokio_free_socketpair();

    let service = FakeDisplayConfig {
        state: Arc::clone(&state),
    };
    // A p2p server's build() blocks until the client completes the
    // handshake, so both ends must be built concurrently.
    let server_fut = async {
        zbus::connection::Builder::unix_stream(server_stream)
            .server(zbus::Guid::generate())
            .unwrap()
            .p2p()
            .serve_at("/org/gnome/Mutter/DisplayConfig", service)
            .unwrap()
            .build()
            .await
            .unwrap()
    };
    let client_fut = async {
        zbus::connection::Builder::unix_stream(client_stream)
            .p2p()
            .build()
            .await
            .unwrap()
    };
    let (server, client) = futures_lite::future::zip(server_fut, client_fut).await;
    // Keep the server connection alive for the test duration.
    std::mem::forget(server);

    let backend = MutterBackend::with_connection(&client).await.unwrap();
    std::mem::forget(client);
    (backend, state)
}

fn tokio_free_socketpair() -> (
    std::os::unix::net::UnixStream,
    std::os::unix::net::UnixStream,
) {
    std::os::unix::net::UnixStream::pair().unwrap()
}

fn simple_layout(primary_first: bool) -> DisplayLayout {
    let assignment = |connector: &str, mode: &str| MonitorAssignment {
        connector: connector.into(),
        mode_id: mode.into(),
        color_mode: None,
        rgb_range: None,
        underscanning: None,
    };
    DisplayLayout {
        layout_mode: LayoutMode::Logical,
        logical_displays: vec![
            LogicalDisplay {
                x: 0,
                y: 0,
                scale: 2.0,
                transform: Transform::Normal,
                primary: primary_first,
                monitors: vec![
                    assignment("DP-7", "3840x2160@59.997"),
                    assignment("HDMI-1", "3840x2160@30.000"),
                ],
            },
            LogicalDisplay {
                x: 1920,
                y: 0,
                scale: 2.0,
                transform: Transform::Normal,
                primary: !primary_first,
                monitors: vec![assignment("DP-8", "3840x2160@59.997")],
            },
        ],
    }
}

#[test]
fn state_retrieval_and_parsing() {
    futures_lite::future::block_on(async {
        let (backend, _) = setup().await;
        let state = backend.current_state().await.unwrap();
        assert_eq!(state.serial, 1);
        assert_eq!(state.monitors.len(), 3);
        assert_eq!(state.layout.logical_displays.len(), 3);
        assert!(state.supports_changing_layout_mode);
        let dp7 = state.monitor("DP-7").unwrap();
        assert_eq!(dp7.current_mode().unwrap().id, "3840x2160@59.997");
    });
}

#[test]
fn verify_does_not_change_state() {
    futures_lite::future::block_on(async {
        let (backend, fake) = setup().await;
        let state = backend.current_state().await.unwrap();
        backend
            .apply(state.serial, &simple_layout(true), ApplyMethod::Verify)
            .await
            .unwrap();
        let fake = fake.lock().unwrap();
        assert_eq!(fake.calls, vec![(1, 0)]);
        assert_eq!(fake.serial, 1, "verify must not bump the serial");
    });
}

#[test]
fn temporary_apply_updates_state_and_emits_signal() {
    futures_lite::future::block_on(async {
        let (backend, fake) = setup().await;
        let mut events = backend.events().await.unwrap();
        let state = backend.current_state().await.unwrap();

        backend
            .apply(state.serial, &simple_layout(true), ApplyMethod::Temporary)
            .await
            .unwrap();
        assert_eq!(events.next().await, Some(BackendEvent::MonitorsChanged));

        let new_state = backend.current_state().await.unwrap();
        assert_eq!(new_state.serial, 2);
        assert_eq!(new_state.layout.logical_displays.len(), 2);
        let (_, group) = new_state.layout.group_of("DP-7").unwrap();
        assert!(group.is_mirror_group());
        assert!(group.contains_connector("HDMI-1"));
        assert!(group.primary);
        assert_eq!(fake.lock().unwrap().calls, vec![(1, 1)]);
    });
}

#[test]
fn persistent_apply_is_recorded() {
    futures_lite::future::block_on(async {
        let (backend, fake) = setup().await;
        let state = backend.current_state().await.unwrap();
        backend
            .apply(state.serial, &simple_layout(false), ApplyMethod::Persistent)
            .await
            .unwrap();
        assert_eq!(fake.lock().unwrap().calls, vec![(1, 2)]);
    });
}

#[test]
fn stale_serial_is_classified() {
    futures_lite::future::block_on(async {
        let (backend, _) = setup().await;
        let state = backend.current_state().await.unwrap();
        backend
            .apply(state.serial, &simple_layout(true), ApplyMethod::Temporary)
            .await
            .unwrap();
        // Re-use the old serial: must be rejected as stale.
        let err = backend
            .apply(state.serial, &simple_layout(true), ApplyMethod::Temporary)
            .await
            .unwrap_err();
        match err {
            BackendError::Rejected { kind, .. } => assert_eq!(kind, RejectionKind::StaleSerial),
            other => panic!("expected stale-serial rejection, got {other:?}"),
        }
    });
}

#[test]
fn unknown_connector_is_classified() {
    futures_lite::future::block_on(async {
        let (backend, _) = setup().await;
        let state = backend.current_state().await.unwrap();
        let mut layout = simple_layout(true);
        layout.logical_displays[1].monitors[0].connector = "DP-99".into();
        let err = backend
            .apply(state.serial, &layout, ApplyMethod::Verify)
            .await
            .unwrap_err();
        match err {
            BackendError::Rejected { kind, message } => {
                assert_eq!(kind, RejectionKind::UnknownMonitor);
                assert!(message.contains("DP-99"));
            }
            other => panic!("expected rejection, got {other:?}"),
        }
    });
}

#[test]
fn missing_primary_is_classified() {
    futures_lite::future::block_on(async {
        let (backend, _) = setup().await;
        let state = backend.current_state().await.unwrap();
        let mut layout = simple_layout(true);
        for logical in &mut layout.logical_displays {
            logical.primary = false;
        }
        let err = backend
            .apply(state.serial, &layout, ApplyMethod::Verify)
            .await
            .unwrap_err();
        match err {
            BackendError::Rejected { kind, .. } => {
                assert_eq!(kind, RejectionKind::MissingPrimary);
            }
            other => panic!("expected rejection, got {other:?}"),
        }
    });
}

#[test]
fn rollback_roundtrip_temporary() {
    futures_lite::future::block_on(async {
        let (backend, _) = setup().await;
        let original = backend.current_state().await.unwrap();
        let snapshot = original.to_apply_snapshot();

        backend
            .apply(
                original.serial,
                &simple_layout(true),
                ApplyMethod::Temporary,
            )
            .await
            .unwrap();
        // Restore with a *fresh* serial, exactly like the watchdog does.
        let fresh = backend.current_state().await.unwrap();
        backend
            .apply(fresh.serial, &snapshot.layout, ApplyMethod::Temporary)
            .await
            .unwrap();

        let restored = backend.current_state().await.unwrap();
        assert_eq!(restored.layout.logical_displays.len(), 3);
        assert!(
            !restored
                .layout
                .logical_displays
                .iter()
                .any(|l| l.is_mirror_group())
        );
    });
}
