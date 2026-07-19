//! A snapshot of the compositor's display state.

use serde::{Deserialize, Serialize};

use crate::layout::{DisplayLayout, LogicalDisplay};
use crate::mode::ExtraProps;
use crate::monitor::PhysicalMonitor;

/// `ApplyMonitorsConfig` method values (verified against Mutter 50.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ApplyMethod {
    /// Full validation and CRTC assignment dry-run; changes nothing.
    Verify,
    /// Applies for this session only; never written to `monitors.xml`.
    Temporary,
    /// Applies and triggers Mutter's own confirm flow: GNOME Shell shows the
    /// "Keep these display settings?" dialog and Mutter reverts automatically
    /// after its timeout (20 s) unless the user confirms — only then is
    /// `monitors.xml` written.
    Persistent,
}

impl ApplyMethod {
    pub const fn as_u32(self) -> u32 {
        match self {
            ApplyMethod::Verify => 0,
            ApplyMethod::Temporary => 1,
            ApplyMethod::Persistent => 2,
        }
    }
}

/// Parsed `GetCurrentState` result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisplayState {
    /// Configuration serial. Any `ApplyMonitorsConfig` call must pass the
    /// serial of the state it was derived from; Mutter rejects stale serials.
    pub serial: u32,
    pub monitors: Vec<PhysicalMonitor>,
    pub layout: DisplayLayout,
    #[serde(default)]
    pub supports_changing_layout_mode: bool,
    #[serde(default)]
    pub global_scale_required: bool,
    /// `supports-mirroring` property; absent means supported (GNOME Settings
    /// defaults it to true).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_mirroring: Option<bool>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub extra: ExtraProps,
}

impl DisplayState {
    pub fn monitor(&self, connector: &str) -> Option<&PhysicalMonitor> {
        self.monitors.iter().find(|m| m.connector() == connector)
    }

    pub fn primary(&self) -> Option<&LogicalDisplay> {
        self.layout.primary()
    }

    pub fn disabled_monitors(&self) -> Vec<&PhysicalMonitor> {
        self.layout.disabled_monitors(&self.monitors)
    }

    /// A serializable snapshot sufficient to restore this exact configuration
    /// later (used for rollback, including by the out-of-process watchdog).
    pub fn to_apply_snapshot(&self) -> ApplySnapshot {
        ApplySnapshot {
            captured_serial: self.serial,
            layout: self.layout.clone(),
        }
    }
}

/// Everything needed to re-apply a configuration for rollback.
///
/// The layout references connectors and mode IDs, which stay valid as long as
/// the same monitors remain connected; the restorer must fetch a *fresh*
/// serial before applying (serials advance on every configuration change).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApplySnapshot {
    pub captured_serial: u32,
    pub layout: DisplayLayout,
}
