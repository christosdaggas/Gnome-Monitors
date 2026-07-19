//! Logical displays and layouts.
//!
//! A *logical display* (Mutter: logical monitor) is a region of the shared
//! coordinate space. It contains one or more physical monitors; when it
//! contains several, those monitors mirror each other. This is exactly how
//! `ApplyMonitorsConfig` models clone mode, and it was verified against the
//! installed Mutter 50.2 (a logical monitor with DP-7 + HDMI-1 passed verify).

use serde::{Deserialize, Serialize};

use crate::geometry::Rect;
use crate::monitor::{ColorMode, PhysicalMonitor, RgbRange};
use crate::transform::Transform;

/// Mutter layout modes (wire values).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum LayoutMode {
    /// Coordinates are in scaled (logical) pixels — the GNOME default.
    #[default]
    #[serde(rename = "logical")]
    Logical,
    /// Coordinates are in physical pixels.
    #[serde(rename = "physical")]
    Physical,
}

impl LayoutMode {
    pub const fn as_u32(self) -> u32 {
        match self {
            LayoutMode::Logical => 1,
            LayoutMode::Physical => 2,
        }
    }

    pub const fn from_u32(v: u32) -> Option<Self> {
        Some(match v {
            1 => LayoutMode::Logical,
            2 => LayoutMode::Physical,
            _ => return None,
        })
    }
}

/// One physical monitor's assignment inside a logical display.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitorAssignment {
    /// Connector of the physical monitor (resolved against the current
    /// snapshot; profiles store EDID identity instead).
    pub connector: String,
    /// Mode ID from the same snapshot.
    pub mode_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_mode: Option<ColorMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rgb_range: Option<RgbRange>,
    /// Only meaningful for monitors that support underscanning
    /// (`is-underscanning` present in their properties).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underscanning: Option<bool>,
}

/// A logical display: position, scale, transform, primary flag and one or
/// more member monitors (several members = mirror group).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogicalDisplay {
    pub x: i32,
    pub y: i32,
    pub scale: f64,
    #[serde(default)]
    pub transform: Transform,
    #[serde(default)]
    pub primary: bool,
    pub monitors: Vec<MonitorAssignment>,
}

impl LogicalDisplay {
    pub fn is_mirror_group(&self) -> bool {
        self.monitors.len() > 1
    }

    pub fn contains_connector(&self, connector: &str) -> bool {
        self.monitors.iter().any(|m| m.connector == connector)
    }

    /// The mode resolution shared by the group (taken from the first member
    /// whose mode resolves; validation checks the rest agree).
    pub fn mode_size(&self, monitors: &[PhysicalMonitor]) -> Option<(i32, i32)> {
        self.monitors.iter().find_map(|a| {
            monitors
                .iter()
                .find(|m| m.connector() == a.connector)
                .and_then(|m| m.find_mode(&a.mode_id))
                .map(|m| (m.width, m.height))
        })
    }

    /// Size the logical display occupies in layout coordinates.
    ///
    /// Logical layout mode: `round(transformed_size / scale)` (Mutter's
    /// `roundf` semantics). Physical layout mode: the transformed mode size.
    pub fn logical_size(
        &self,
        monitors: &[PhysicalMonitor],
        layout_mode: LayoutMode,
    ) -> Option<(i32, i32)> {
        let (w, h) = self.mode_size(monitors)?;
        let (w, h) = self.transform.apply_to(w, h);
        Some(match layout_mode {
            LayoutMode::Logical => scale_size(w, h, self.scale),
            LayoutMode::Physical => (w, h),
        })
    }

    pub fn rect(&self, monitors: &[PhysicalMonitor], layout_mode: LayoutMode) -> Option<Rect> {
        let (w, h) = self.logical_size(monitors, layout_mode)?;
        Some(Rect::new(self.x, self.y, w, h))
    }
}

/// `round(size / scale)` with C `roundf` (half away from zero) semantics —
/// which is what Rust's `f64::round` implements.
pub fn scale_size(width: i32, height: i32, scale: f64) -> (i32, i32) {
    #[allow(clippy::cast_possible_truncation)]
    (
        (f64::from(width) / scale).round() as i32,
        (f64::from(height) / scale).round() as i32,
    )
}

/// A complete proposed or current layout. Physical monitors that appear in no
/// logical display are disabled.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisplayLayout {
    #[serde(default)]
    pub layout_mode: LayoutMode,
    pub logical_displays: Vec<LogicalDisplay>,
}

impl DisplayLayout {
    /// The logical display containing `connector`, if any.
    pub fn group_of(&self, connector: &str) -> Option<(usize, &LogicalDisplay)> {
        self.logical_displays
            .iter()
            .enumerate()
            .find(|(_, l)| l.contains_connector(connector))
    }

    pub fn primary(&self) -> Option<&LogicalDisplay> {
        self.logical_displays.iter().find(|l| l.primary)
    }

    /// Connectors of all enabled monitors.
    pub fn enabled_connectors(&self) -> Vec<&str> {
        self.logical_displays
            .iter()
            .flat_map(|l| l.monitors.iter().map(|m| m.connector.as_str()))
            .collect()
    }

    /// Monitors from `monitors` that this layout leaves disabled.
    pub fn disabled_monitors<'a>(
        &self,
        monitors: &'a [PhysicalMonitor],
    ) -> Vec<&'a PhysicalMonitor> {
        let enabled = self.enabled_connectors();
        monitors
            .iter()
            .filter(|m| !enabled.contains(&m.connector()))
            .collect()
    }

    /// Set exactly one primary logical display by index.
    pub fn set_primary(&mut self, index: usize) {
        for (i, l) in self.logical_displays.iter_mut().enumerate() {
            l.primary = i == index;
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn scale_size_rounds_half_away_from_zero() {
        assert_eq!(scale_size(3840, 2160, 2.0), (1920, 1080));
        assert_eq!(scale_size(3840, 2160, 1.5), (2560, 1440));
        assert_eq!(scale_size(2560, 1440, 1.5), (1707, 960)); // 1706.67 → 1707
        assert_eq!(scale_size(1920, 1080, 1.25), (1536, 864));
        assert_eq!(scale_size(3840, 2160, 1.75), (2194, 1234)); // 2194.29, 1234.29
    }

    #[test]
    fn layout_mode_wire_values() {
        assert_eq!(LayoutMode::Logical.as_u32(), 1);
        assert_eq!(LayoutMode::Physical.as_u32(), 2);
        assert_eq!(LayoutMode::from_u32(3), None);
    }
}
