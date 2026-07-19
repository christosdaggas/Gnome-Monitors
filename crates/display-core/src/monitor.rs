//! Physical monitors and their capabilities.

use serde::{Deserialize, Serialize};

use crate::identity::MonitorIdentity;
use crate::mode::{ExtraProps, MonitorMode};

/// Color mode values used by `org.gnome.Mutter.DisplayConfig`
/// (verified against Mutter 50.2 / gdctl).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ColorMode {
    #[serde(rename = "default")]
    Default,
    #[serde(rename = "bt2100")]
    Bt2100,
    #[serde(rename = "sdr-native")]
    SdrNative,
}

impl ColorMode {
    pub const fn as_u32(self) -> u32 {
        match self {
            ColorMode::Default => 0,
            ColorMode::Bt2100 => 1,
            ColorMode::SdrNative => 2,
        }
    }

    pub const fn from_u32(v: u32) -> Option<Self> {
        Some(match v {
            0 => ColorMode::Default,
            1 => ColorMode::Bt2100,
            2 => ColorMode::SdrNative,
            _ => return None,
        })
    }

    pub const fn label(self) -> &'static str {
        match self {
            ColorMode::Default => "Standard",
            ColorMode::Bt2100 => "HDR (BT.2100)",
            ColorMode::SdrNative => "SDR (native gamut)",
        }
    }
}

/// RGB range values (note: 1-based on the wire, per Mutter 50.2 gdctl).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RgbRange {
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "full")]
    Full,
    #[serde(rename = "limited")]
    Limited,
}

impl RgbRange {
    pub const fn as_u32(self) -> u32 {
        match self {
            RgbRange::Auto => 1,
            RgbRange::Full => 2,
            RgbRange::Limited => 3,
        }
    }

    pub const fn from_u32(v: u32) -> Option<Self> {
        Some(match v {
            1 => RgbRange::Auto,
            2 => RgbRange::Full,
            3 => RgbRange::Limited,
            _ => return None,
        })
    }

    pub const fn label(self) -> &'static str {
        match self {
            RgbRange::Auto => "Automatic",
            RgbRange::Full => "Full",
            RgbRange::Limited => "Limited",
        }
    }
}

/// A physical monitor as reported by `GetCurrentState`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhysicalMonitor {
    pub identity: MonitorIdentity,
    /// Mutter's `display-name`, e.g. `LG Electronics 27"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub modes: Vec<MonitorMode>,
    #[serde(default)]
    pub is_builtin: bool,
    #[serde(default)]
    pub is_underscanning: bool,
    /// True when the monitor supports underscanning at all (the
    /// `is-underscanning` property was present).
    #[serde(default)]
    pub supports_underscanning: bool,
    #[serde(default)]
    pub is_for_lease: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_mode: Option<ColorMode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_color_modes: Vec<ColorMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rgb_range: Option<RgbRange>,
    /// Present when the monitor/connection supports VRR (minimum refresh).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_refresh_rate: Option<i32>,
    /// From EDID via sysfs (not part of the D-Bus API); best effort.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub physical_size_mm: Option<(i32, i32)>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub extra: ExtraProps,
}

impl PhysicalMonitor {
    pub fn connector(&self) -> &str {
        &self.identity.connector
    }

    pub fn current_mode(&self) -> Option<&MonitorMode> {
        self.modes.iter().find(|m| m.is_current)
    }

    pub fn preferred_mode(&self) -> Option<&MonitorMode> {
        self.modes.iter().find(|m| m.is_preferred)
    }

    pub fn find_mode(&self, id: &str) -> Option<&MonitorMode> {
        self.modes.iter().find(|m| m.id == id)
    }

    /// Best (highest refresh, non-interlaced preferred) mode at a resolution.
    pub fn best_mode_at(&self, width: i32, height: i32) -> Option<&MonitorMode> {
        self.modes
            .iter()
            .filter(|m| m.width == width && m.height == height)
            .min_by(|a, b| {
                (a.is_interlaced, std::cmp::Reverse(OrderedF64(a.refresh_hz)))
                    .partial_cmp(&(b.is_interlaced, std::cmp::Reverse(OrderedF64(b.refresh_hz))))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    /// All refresh rates available at a resolution, descending, deduplicated.
    pub fn refresh_rates_at(&self, width: i32, height: i32) -> Vec<&MonitorMode> {
        let mut modes: Vec<&MonitorMode> = self
            .modes
            .iter()
            .filter(|m| m.width == width && m.height == height && !m.is_interlaced)
            .collect();
        modes.sort_by(|a, b| {
            b.refresh_hz
                .partial_cmp(&a.refresh_hz)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        modes
    }

    /// Distinct resolutions, largest first.
    pub fn resolutions(&self) -> Vec<(i32, i32)> {
        let mut res: Vec<(i32, i32)> = Vec::new();
        for m in &self.modes {
            if m.is_interlaced {
                continue;
            }
            if !res.contains(&(m.width, m.height)) {
                res.push((m.width, m.height));
            }
        }
        res.sort_by_key(|(w, h)| std::cmp::Reverse((i64::from(*w) * i64::from(*h), *w)));
        res
    }

    pub fn supports_hdr(&self) -> bool {
        self.supported_color_modes.contains(&ColorMode::Bt2100)
    }

    pub fn supports_vrr(&self) -> bool {
        self.min_refresh_rate.is_some()
            || self
                .modes
                .iter()
                .any(|m| m.refresh_rate_mode.as_deref() == Some("variable"))
    }

    /// Heuristic: does this look like a KVM / capture / EDID-emulating device
    /// rather than a real panel? The user can always override the result.
    ///
    /// Signals (score-based, threshold 2):
    /// * vendor is a known capture-chip / KVM vendor (Lontium, …)
    /// * product name mentions KVM/capture
    /// * synthetic-looking EDID serial
    /// * implausibly small claimed panel size for a high-resolution mode
    pub fn kvm_score(&self) -> u32 {
        let mut score = 0;
        let vendor = self.identity.vendor.to_ascii_uppercase();
        let product = self.identity.product.to_ascii_lowercase();
        if ["LTM", "LNT"].contains(&vendor.as_str()) {
            score += 2; // Lontium Semiconductor: HDMI capture chips in KVMs
        }
        if product.contains("kvm") || product.contains("capture") || product.contains("dummy") {
            score += 2;
        }
        if self.identity.serial_looks_synthetic() {
            score += 1;
        }
        if let Some((w_mm, h_mm)) = self.physical_size_mm {
            let native_width = self.preferred_mode().map_or(0, |m| m.width);
            if w_mm > 0 && w_mm < 200 && h_mm > 0 && native_width >= 2560 {
                score += 1; // "5-inch 4K panel"
            }
        } else if let Some(name) = &self.display_name {
            // display-name encodes the claimed diagonal, e.g. `LTM 5"`.
            if let Some(inches) = parse_diagonal_inches(name) {
                let native_width = self.preferred_mode().map_or(0, |m| m.width);
                if inches <= 8 && native_width >= 2560 {
                    score += 1;
                }
            }
        }
        score
    }

    pub fn is_likely_kvm(&self) -> bool {
        self.kvm_score() >= 2
    }
}

fn parse_diagonal_inches(display_name: &str) -> Option<i32> {
    let idx = display_name.find('"')?;
    let head = &display_name[..idx];
    let digits: String = head
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    digits.parse().ok()
}

#[derive(PartialEq, PartialOrd)]
struct OrderedF64(f64);

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::mode::ExtraProps;

    pub(crate) fn mk_mode(w: i32, h: i32, hz: f64, preferred: bool) -> MonitorMode {
        MonitorMode {
            id: format!("{w}x{h}@{hz:.3}"),
            width: w,
            height: h,
            refresh_hz: hz,
            preferred_scale: 1.0,
            supported_scales: vec![1.0, 1.25, 1.5, 2.0],
            is_current: preferred,
            is_preferred: preferred,
            is_interlaced: false,
            refresh_rate_mode: None,
            extra: ExtraProps::new(),
        }
    }

    fn kvm_monitor() -> PhysicalMonitor {
        PhysicalMonitor {
            identity: MonitorIdentity::new("HDMI-1", "LTM", "Lontium semi", "0x88888800"),
            display_name: Some("LTM 5\"".into()),
            modes: vec![
                mk_mode(3840, 2160, 30.0, true),
                mk_mode(1920, 1080, 60.0, false),
            ],
            is_builtin: false,
            is_underscanning: false,
            supports_underscanning: false,
            is_for_lease: false,
            color_mode: Some(ColorMode::Default),
            supported_color_modes: vec![ColorMode::Default, ColorMode::SdrNative],
            rgb_range: Some(RgbRange::Auto),
            min_refresh_rate: Some(24),
            physical_size_mm: None,
            extra: ExtraProps::new(),
        }
    }

    #[test]
    fn kvm_heuristic_flags_lontium() {
        assert!(kvm_monitor().is_likely_kvm());
    }

    #[test]
    fn kvm_heuristic_spares_real_panels() {
        let lg = PhysicalMonitor {
            identity: MonitorIdentity::new("DP-7", "GSM", "LG HDR 4K", "0x0004ee0e"),
            display_name: Some("LG Electronics 27\"".into()),
            modes: vec![mk_mode(3840, 2160, 59.997, true)],
            is_builtin: false,
            is_underscanning: false,
            supports_underscanning: false,
            is_for_lease: false,
            color_mode: Some(ColorMode::Default),
            supported_color_modes: vec![
                ColorMode::Default,
                ColorMode::Bt2100,
                ColorMode::SdrNative,
            ],
            rgb_range: Some(RgbRange::Auto),
            min_refresh_rate: None,
            physical_size_mm: Some((600, 340)),
            extra: ExtraProps::new(),
        };
        assert!(!lg.is_likely_kvm());
        assert!(lg.supports_hdr());
    }

    #[test]
    fn best_mode_prefers_highest_refresh() {
        let mut m = kvm_monitor();
        m.modes.push(mk_mode(1920, 1080, 50.0, false));
        let best = m.best_mode_at(1920, 1080).unwrap();
        assert!((best.refresh_hz - 60.0).abs() < 1e-9);
    }

    #[test]
    fn resolutions_sorted_desc() {
        let m = kvm_monitor();
        assert_eq!(m.resolutions(), vec![(3840, 2160), (1920, 1080)]);
    }

    #[test]
    fn wire_enums_roundtrip() {
        assert_eq!(ColorMode::from_u32(1), Some(ColorMode::Bt2100));
        assert_eq!(RgbRange::from_u32(1), Some(RgbRange::Auto));
        assert_eq!(RgbRange::Auto.as_u32(), 1);
        assert_eq!(RgbRange::from_u32(0), None);
    }

    #[test]
    fn diagonal_parsing() {
        assert_eq!(parse_diagonal_inches("LTM 5\""), Some(5));
        assert_eq!(parse_diagonal_inches("LG Electronics 27\""), Some(27));
        assert_eq!(parse_diagonal_inches("Dell U2720Q"), None);
    }
}
