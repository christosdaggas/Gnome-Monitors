//! zbus proxy and raw wire types for `org.gnome.Mutter.DisplayConfig`.
//!
//! Signatures match Mutter 50.2 exactly (verified by live `busctl`
//! introspection on the target system):
//!
//! ```text
//! GetCurrentState() -> ua((ssss)a(siiddada{sv})a{sv})a(iiduba(ssss)a{sv})a{sv}
//! ApplyMonitorsConfig(u, u, a(iiduba(ssa{sv})), a{sv})
//! MonitorsChanged()
//! ```

use std::collections::HashMap;

use zvariant::OwnedValue;

/// `(connector, vendor, product, serial)`.
pub type RawMonitorSpec = (String, String, String, String);

/// `(id, width, height, refresh, preferred_scale, supported_scales, properties)`.
pub type RawMode = (
    String,
    i32,
    i32,
    f64,
    f64,
    Vec<f64>,
    HashMap<String, OwnedValue>,
);

/// `(spec, modes, properties)`.
pub type RawMonitor = (RawMonitorSpec, Vec<RawMode>, HashMap<String, OwnedValue>);

/// `(x, y, scale, transform, primary, monitor_specs, properties)`.
pub type RawLogicalMonitor = (
    i32,
    i32,
    f64,
    u32,
    bool,
    Vec<RawMonitorSpec>,
    HashMap<String, OwnedValue>,
);

/// Full `GetCurrentState` reply.
pub type RawState = (
    u32,
    Vec<RawMonitor>,
    Vec<RawLogicalMonitor>,
    HashMap<String, OwnedValue>,
);

/// Monitor assignment inside an `ApplyMonitorsConfig` logical monitor:
/// `(connector, mode_id, properties)` where properties may carry
/// `underscanning: b`, `color-mode: u`, `rgb-range: u`.
pub type WireMonitorAssignment = (String, String, HashMap<String, OwnedValue>);

/// `(x, y, scale, transform, primary, monitors)`.
pub type WireLogicalMonitor = (i32, i32, f64, u32, bool, Vec<WireMonitorAssignment>);

#[zbus::proxy(
    interface = "org.gnome.Mutter.DisplayConfig",
    default_service = "org.gnome.Mutter.DisplayConfig",
    default_path = "/org/gnome/Mutter/DisplayConfig"
)]
pub trait DisplayConfig {
    fn get_current_state(&self) -> zbus::Result<RawState>;

    fn apply_monitors_config(
        &self,
        serial: u32,
        method: u32,
        logical_monitors: Vec<WireLogicalMonitor>,
        properties: HashMap<String, OwnedValue>,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    fn monitors_changed(&self) -> zbus::Result<()>;

    #[zbus(property)]
    fn apply_monitors_config_allowed(&self) -> zbus::Result<bool>;
}
