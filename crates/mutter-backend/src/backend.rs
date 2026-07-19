//! The backend abstraction: real Mutter, or an in-memory mock for tests.

use std::pin::Pin;
use std::sync::{Arc, Mutex};

use display_core::DisplayState;
use display_core::layout::DisplayLayout;
use display_core::state::ApplyMethod;
use display_core::validation::{self, LayoutProblem};
use futures_util::{Stream, StreamExt};
use tracing::{debug, info, warn};

use crate::error::{BackendError, RejectionKind};
use crate::parse::parse_state;
use crate::proxy::DisplayConfigProxy;
use crate::serialize::{wire_logical_monitors, wire_properties};

/// Events emitted by a backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendEvent {
    /// The compositor's monitor state changed (hot-plug, mode change, any
    /// applied configuration — including our own). Fetch a fresh state; any
    /// previously held serial is now stale.
    MonitorsChanged,
}

/// Stream of backend events.
pub type EventStream = Pin<Box<dyn Stream<Item = BackendEvent> + Send>>;

/// Compositor abstraction used by the UI, CLI, and tests.
pub trait DisplayBackend {
    fn current_state(
        &self,
    ) -> impl std::future::Future<Output = Result<DisplayState, BackendError>>;

    /// Sends the layout with the given method. `serial` must come from the
    /// `DisplayState` the layout was derived from.
    fn apply(
        &self,
        serial: u32,
        layout: &DisplayLayout,
        method: ApplyMethod,
    ) -> impl std::future::Future<Output = Result<(), BackendError>>;

    fn events(&self) -> impl std::future::Future<Output = Result<EventStream, BackendError>>;
}

/// The real backend, speaking to `org.gnome.Mutter.DisplayConfig`.
#[derive(Debug, Clone)]
pub struct MutterBackend {
    proxy: DisplayConfigProxy<'static>,
    supports_changing_layout_mode: Arc<Mutex<bool>>,
}

impl MutterBackend {
    /// Connects to the session bus.
    pub async fn connect() -> Result<Self, BackendError> {
        let connection = zbus::Connection::session().await?;
        Self::with_connection(&connection).await
    }

    /// Uses an existing connection (integration tests point this at a
    /// private bus running a fake DisplayConfig service).
    pub async fn with_connection(connection: &zbus::Connection) -> Result<Self, BackendError> {
        let proxy = DisplayConfigProxy::builder(connection)
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await?;
        Ok(Self {
            proxy,
            supports_changing_layout_mode: Arc::new(Mutex::new(false)),
        })
    }

    pub fn proxy(&self) -> &DisplayConfigProxy<'static> {
        &self.proxy
    }
}

impl DisplayBackend for MutterBackend {
    async fn current_state(&self) -> Result<DisplayState, BackendError> {
        let raw = self.proxy.get_current_state().await?;
        let state = parse_state(&raw)?;
        if let Ok(mut flag) = self.supports_changing_layout_mode.lock() {
            *flag = state.supports_changing_layout_mode;
        }
        debug!(
            serial = state.serial,
            monitors = state.monitors.len(),
            logical = state.layout.logical_displays.len(),
            "fetched current state"
        );
        Ok(state)
    }

    async fn apply(
        &self,
        serial: u32,
        layout: &DisplayLayout,
        method: ApplyMethod,
    ) -> Result<(), BackendError> {
        let logical_monitors = wire_logical_monitors(layout)?;
        let supports_layout_mode = self
            .supports_changing_layout_mode
            .lock()
            .map(|f| *f)
            .unwrap_or(false);
        let properties = wire_properties(layout.layout_mode, supports_layout_mode)?;
        info!(
            serial,
            ?method,
            groups = logical_monitors.len(),
            "ApplyMonitorsConfig"
        );
        self.proxy
            .apply_monitors_config(serial, method.as_u32(), logical_monitors, properties)
            .await
            .map_err(BackendError::from_apply_error)?;
        Ok(())
    }

    async fn events(&self) -> Result<EventStream, BackendError> {
        let stream = self.proxy.receive_monitors_changed().await?;
        Ok(Box::pin(stream.map(|_| BackendEvent::MonitorsChanged)))
    }
}

/// In-memory backend for tests and fixtures.
///
/// Validates like Mutter (serial check + the display-core rule set, which
/// mirrors Mutter 50's verify), mutates its state on temporary/persistent
/// applies, and emits [`BackendEvent::MonitorsChanged`].
#[derive(Debug, Clone)]
pub struct MockBackend {
    state: Arc<Mutex<DisplayState>>,
    sender: async_channel::Sender<BackendEvent>,
    receiver: async_channel::Receiver<BackendEvent>,
    /// Applies recorded as (serial, method) for assertions.
    pub applied: Arc<Mutex<Vec<(u32, ApplyMethod)>>>,
}

impl MockBackend {
    pub fn new(state: DisplayState) -> Self {
        let (sender, receiver) = async_channel::unbounded();
        Self {
            state: Arc::new(Mutex::new(state)),
            sender,
            receiver,
            applied: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, DisplayState> {
        match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// Replaces the connected monitors (hot-plug simulation): bumps the
    /// serial, drops vanished monitors from the layout, and emits a change
    /// event.
    pub fn hotplug(&self, monitors: Vec<display_core::PhysicalMonitor>) {
        {
            let mut state = self.lock_state();
            state.monitors = monitors;
            let connectors: Vec<String> = state
                .monitors
                .iter()
                .map(|m| m.connector().to_owned())
                .collect();
            state.layout.logical_displays.retain_mut(|logical| {
                logical
                    .monitors
                    .retain(|a| connectors.contains(&a.connector));
                !logical.monitors.is_empty()
            });
            if state.layout.primary().is_none()
                && let Some(first) = state.layout.logical_displays.first_mut()
            {
                first.primary = true;
            }
            state.serial += 1;
        }
        let _ = self.sender.try_send(BackendEvent::MonitorsChanged);
    }

    /// Direct access for test assertions.
    pub fn snapshot(&self) -> DisplayState {
        self.lock_state().clone()
    }
}

fn problems_to_error(problems: &[LayoutProblem]) -> Option<BackendError> {
    let fatal = problems.iter().find(|p| !p.is_auto_fixable())?;
    let kind = match fatal {
        LayoutProblem::NotAdjacent => RejectionKind::NotAdjacent,
        LayoutProblem::Overlap { .. } => RejectionKind::Overlap,
        LayoutProblem::MultiplePrimary => RejectionKind::MultiplePrimary,
        LayoutProblem::MissingPrimary | LayoutProblem::NoActiveDisplays => {
            RejectionKind::MissingPrimary
        }
        LayoutProblem::UnsupportedScale { .. } | LayoutProblem::InvalidScale { .. } => {
            RejectionKind::InvalidScale
        }
        LayoutProblem::UnknownMode { .. } => RejectionKind::InvalidMode,
        LayoutProblem::MirrorResolutionMismatch { .. } => RejectionKind::MirrorModesUnequal,
        LayoutProblem::UnknownConnector { .. } => RejectionKind::UnknownMonitor,
        LayoutProblem::NotNormalized { .. } => RejectionKind::BadPosition,
        _ => RejectionKind::Other,
    };
    Some(BackendError::Rejected {
        kind,
        message: fatal.to_string(),
    })
}

impl DisplayBackend for MockBackend {
    async fn current_state(&self) -> Result<DisplayState, BackendError> {
        Ok(self.lock_state().clone())
    }

    async fn apply(
        &self,
        serial: u32,
        layout: &DisplayLayout,
        method: ApplyMethod,
    ) -> Result<(), BackendError> {
        let mut state = self.lock_state();
        if serial != state.serial {
            return Err(BackendError::Rejected {
                kind: RejectionKind::StaleSerial,
                message: "The requested configuration is based on stale information".into(),
            });
        }
        let problems = validation::validate_with_policy(
            &state.monitors,
            layout,
            validation::ValidationPolicy::from_state(&state),
        );
        if let Some(error) = problems_to_error(&problems) {
            warn!(%error, "mock backend rejected configuration");
            return Err(error);
        }
        if let Ok(mut applied) = self.applied.lock() {
            applied.push((serial, method));
        }
        if method == ApplyMethod::Verify {
            return Ok(());
        }

        let mut layout = layout.clone();
        validation::normalize(&mut layout, &state.monitors);
        // Update per-monitor current-mode flags like the compositor would.
        for monitor in &mut state.monitors {
            let assigned_mode = layout
                .logical_displays
                .iter()
                .flat_map(|l| l.monitors.iter())
                .find(|a| a.connector == monitor.identity.connector)
                .map(|a| a.mode_id.clone());
            for mode in &mut monitor.modes {
                mode.is_current = assigned_mode.as_deref() == Some(mode.id.as_str());
            }
        }
        state.layout = layout;
        state.serial += 1;
        drop(state);
        let _ = self.sender.try_send(BackendEvent::MonitorsChanged);
        Ok(())
    }

    async fn events(&self) -> Result<EventStream, BackendError> {
        let receiver = self.receiver.clone();
        Ok(Box::pin(futures_util::stream::unfold(
            receiver,
            |receiver| async move { receiver.recv().await.ok().map(|event| (event, receiver)) },
        )))
    }
}
