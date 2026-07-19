//! Compositor-independent domain models and validation logic for display
//! layouts.
//!
//! This crate deliberately has no D-Bus, GTK, or async dependencies. It models
//! the concepts of the `org.gnome.Mutter.DisplayConfig` API — physical
//! monitors, monitor modes, logical displays (which may contain several
//! physical monitors, i.e. mirror groups), layouts, profiles — and implements
//! the validation rules that Mutter enforces server-side, so that the UI can
//! explain problems *before* asking the compositor to verify or apply a
//! configuration.
//!
//! Numeric conventions follow Mutter 50:
//! * layout coordinates are integers,
//! * in the *logical* layout mode a logical display's size is
//!   `round(transformed_mode_size / scale)` (C `roundf` semantics),
//! * scales are `f64` values that must match one of a mode's supported scales.

pub mod geometry;
pub mod identity;
pub mod layout;
pub mod layout_ops;
pub mod mirror;
pub mod mode;
pub mod monitor;
pub mod paths;
pub mod prefs;
pub mod profile;
pub mod snap;
pub mod state;
pub mod store;
pub mod transform;
pub mod validation;

pub use geometry::Rect;
pub use identity::{MatchLevel, MonitorIdentity};
pub use layout::{DisplayLayout, LayoutMode, LogicalDisplay, MonitorAssignment};
pub use mirror::{MirrorCandidate, MirrorError, MirrorMemberMode};
pub use mode::{MonitorMode, format_refresh, format_scale_percent, refresh_eq};
pub use monitor::{ColorMode, PhysicalMonitor, RgbRange};
pub use profile::{DisplayProfile, ProfileProblem, ProfileResolution};
pub use state::{ApplyMethod, ApplySnapshot, DisplayState};
pub use transform::Transform;
pub use validation::{LayoutProblem, normalize, validate};

/// Floating-point tolerance used when comparing scale values.
pub const SCALE_EPS: f64 = 1e-4;

/// Floating-point tolerance used when two refresh rates listed by the
/// compositor are considered "the same rate" (Mutter reports rates like
/// 59.997, 59.94, 59.934 as distinct modes; equality is only used for
/// identifying a mode we selected earlier).
pub const REFRESH_EPS: f64 = 1e-3;

/// Tolerance when re-matching a stored profile refresh rate against the
/// currently advertised modes. Stored rates are rounded to three decimals.
pub const REFRESH_MATCH_EPS: f64 = 5e-2;

pub(crate) fn approx_eq(a: f64, b: f64, eps: f64) -> bool {
    (a - b).abs() <= eps
}
