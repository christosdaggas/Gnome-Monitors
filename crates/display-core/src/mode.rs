//! Monitor modes as reported by the compositor.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{REFRESH_EPS, approx_eq};

/// Extra, unrecognized D-Bus properties are preserved (stringified) for
/// diagnostics and forward compatibility with newer Mutter versions.
pub type ExtraProps = BTreeMap<String, String>;

/// One mode of a physical monitor (`(siiddad a{sv})` in `GetCurrentState`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitorMode {
    /// Mutter's mode ID, e.g. `3840x2160@59.997`. Only valid within one
    /// `GetCurrentState` snapshot of one monitor; profiles store
    /// width/height/refresh instead.
    pub id: String,
    pub width: i32,
    pub height: i32,
    pub refresh_hz: f64,
    /// Scale Mutter would pick by default for this mode.
    pub preferred_scale: f64,
    /// Scales Mutter supports for this mode (fractional scaling included).
    pub supported_scales: Vec<f64>,
    #[serde(default)]
    pub is_current: bool,
    #[serde(default)]
    pub is_preferred: bool,
    #[serde(default)]
    pub is_interlaced: bool,
    /// `refresh-rate-mode` property where exposed by Mutter (VRR):
    /// `"fixed"` or `"variable"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_rate_mode: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: ExtraProps,
}

impl MonitorMode {
    pub fn resolution(&self) -> (i32, i32) {
        (self.width, self.height)
    }

    pub fn area(&self) -> i64 {
        i64::from(self.width) * i64::from(self.height)
    }

    /// Whether `scale` is one of the supported scales for this mode.
    pub fn supports_scale(&self, scale: f64) -> bool {
        self.supported_scales
            .iter()
            .any(|s| approx_eq(*s, scale, crate::SCALE_EPS))
    }

    /// The closest supported scale within a `0.1` tolerance (the same rule
    /// Mutter's own gdctl applies when accepting user-specified scales).
    pub fn closest_supported_scale(&self, scale: f64) -> Option<f64> {
        self.supported_scales
            .iter()
            .copied()
            .filter(|s| (s - scale).abs() <= 0.1)
            .min_by(|a, b| {
                (a - scale)
                    .abs()
                    .partial_cmp(&(b - scale).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }
}

/// Are two advertised refresh rates the same rate?
pub fn refresh_eq(a: f64, b: f64) -> bool {
    approx_eq(a, b, REFRESH_EPS)
}

/// Formats a refresh rate the way GNOME Settings does (two decimals, no
/// trailing zero trimming — `60.00 Hz`, `59.94 Hz`).
pub fn format_refresh(hz: f64) -> String {
    format!("{hz:.2}\u{2009}Hz")
}

/// Formats a scale as GNOME Settings does — percentage, truncated, with a
/// narrow space: `1.75` → `175 %`.
#[allow(clippy::cast_possible_truncation)]
pub fn format_scale_percent(scale: f64) -> String {
    format!("{} %", (scale * 100.0) as i32)
}

/// Formats a mode as `3840 × 2160`.
pub fn format_resolution(width: i32, height: i32) -> String {
    format!("{width} × {height}")
}

/// Human aspect-ratio label for common ratios (as GNOME Settings shows).
pub fn aspect_ratio_label(width: i32, height: i32) -> Option<&'static str> {
    if width <= 0 || height <= 0 {
        return None;
    }
    let ratios: [(i32, i32, &str); 10] = [
        (16, 9, "16∶9"),
        (16, 10, "16∶10"),
        (21, 9, "21∶9"),
        (32, 9, "32∶9"),
        (4, 3, "4∶3"),
        (5, 4, "5∶4"),
        (3, 2, "3∶2"),
        (1, 1, "1∶1"),
        (5, 3, "5∶3"),
        (9, 5, "9∶5"),
    ];
    for (rw, rh, label) in ratios {
        // width/height == rw/rh  <=>  width*rh == height*rw
        if i64::from(width) * i64::from(rh) == i64::from(height) * i64::from(rw) {
            return Some(label);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn mode(w: i32, h: i32, hz: f64) -> MonitorMode {
        MonitorMode {
            id: format!("{w}x{h}@{hz:.3}"),
            width: w,
            height: h,
            refresh_hz: hz,
            preferred_scale: 1.0,
            supported_scales: vec![1.0, 1.25, 1.5, 2.0],
            is_current: false,
            is_preferred: false,
            is_interlaced: false,
            refresh_rate_mode: None,
            extra: ExtraProps::new(),
        }
    }

    #[test]
    fn refresh_comparisons_distinguish_close_rates() {
        // 59.94 vs 59.95 are distinct modes; 59.997 vs 59.997 are equal.
        assert!(!refresh_eq(59.94, 59.95));
        assert!(refresh_eq(59.997, 59.9971));
        assert!(!refresh_eq(59.94, 60.0));
    }

    #[test]
    fn scale_support() {
        let m = mode(3840, 2160, 59.997);
        assert!(m.supports_scale(1.25));
        assert!(!m.supports_scale(1.75));
        assert_eq!(m.closest_supported_scale(1.24), Some(1.25));
        assert_eq!(m.closest_supported_scale(1.3), Some(1.25));
        assert_eq!(m.closest_supported_scale(1.75), None); // > 0.1 away from 1.5/2.0
    }

    #[test]
    fn formatting() {
        assert_eq!(format_refresh(59.94), "59.94\u{2009}Hz");
        assert_eq!(format_resolution(3840, 2160), "3840 × 2160");
        assert_eq!(aspect_ratio_label(3840, 2160), Some("16∶9"));
        assert_eq!(aspect_ratio_label(1920, 1200), Some("16∶10"));
        assert_eq!(aspect_ratio_label(1234, 771), None);
        assert_eq!(format_scale_percent(1.75), "175 %");
        assert_eq!(format_scale_percent(1.3333333730697632), "133 %");
        assert_eq!(format_scale_percent(2.0), "200 %");
    }
}
