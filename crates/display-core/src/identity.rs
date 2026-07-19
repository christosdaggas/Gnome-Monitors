//! Stable monitor identity.
//!
//! Connector names (`DP-7`, `HDMI-1`, …) are unstable: this machine's own
//! `monitors.xml` history shows the same two LG panels appearing on
//! `DP-5`/`DP-6` and later `DP-7`/`DP-8`, swapping places between boots.
//! Profiles therefore match on EDID identity (vendor + product + serial) and
//! use the connector only as a disambiguation hint.

use serde::{Deserialize, Serialize};

/// Identity of a physical monitor as reported by Mutter's monitor spec
/// (`(ssss)`: connector, vendor, product, serial).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MonitorIdentity {
    pub connector: String,
    pub vendor: String,
    pub product: String,
    pub serial: String,
}

/// How strongly two identities match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchLevel {
    /// No usable correspondence.
    None,
    /// Same connector only (weakest usable signal).
    Connector,
    /// Same vendor + product, serial unknown or different.
    Model,
    /// Same vendor + product + serial (serial non-empty on both sides).
    FullEdid,
}

impl MonitorIdentity {
    pub fn new(
        connector: impl Into<String>,
        vendor: impl Into<String>,
        product: impl Into<String>,
        serial: impl Into<String>,
    ) -> Self {
        Self {
            connector: connector.into(),
            vendor: vendor.into(),
            product: product.into(),
            serial: serial.into(),
        }
    }

    /// True when the EDID triple is meaningful (not entirely empty).
    pub fn has_edid(&self) -> bool {
        !(self.vendor.is_empty() && self.product.is_empty() && self.serial.is_empty())
    }

    /// Key used for alias storage and profile matching. Excludes the
    /// connector, which is unstable. Falls back to the connector only when
    /// the monitor exposes no EDID identity at all (documented fallback).
    pub fn stable_key(&self) -> String {
        if self.has_edid() {
            format!("{}/{}/{}", self.vendor, self.product, self.serial)
        } else {
            format!("connector:{}", self.connector)
        }
    }

    /// Match strength against another identity.
    pub fn match_level(&self, other: &MonitorIdentity) -> MatchLevel {
        let same_model = self.has_edid()
            && !self.vendor.is_empty()
            && self.vendor == other.vendor
            && self.product == other.product;
        if same_model && !self.serial.is_empty() && self.serial == other.serial {
            MatchLevel::FullEdid
        } else if same_model {
            MatchLevel::Model
        } else if !self.connector.is_empty() && self.connector == other.connector {
            MatchLevel::Connector
        } else {
            MatchLevel::None
        }
    }

    /// Serials like `0x88888800`, `0x00000000` or `0x01010101` that EDID
    /// emulators tend to use.
    pub fn serial_looks_synthetic(&self) -> bool {
        let s = self.serial.trim().to_ascii_lowercase();
        if s.is_empty() {
            return true;
        }
        let hex = s.strip_prefix("0x").unwrap_or(&s);
        if hex.chars().all(|c| c == '0') {
            return true;
        }
        // Repeated two-character groups (8888..., 0101...) with at most one
        // distinct pair.
        if hex.len() >= 6 && hex.len().is_multiple_of(2) {
            let pairs: Vec<&str> = (0..hex.len() / 2).map(|i| &hex[i * 2..i * 2 + 2]).collect();
            let mut distinct: Vec<&str> = pairs.clone();
            distinct.sort_unstable();
            distinct.dedup();
            if distinct.len() <= 2 && pairs.len() >= 3 {
                return true;
            }
        }
        false
    }
}

impl std::fmt::Display for MonitorIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({} {})", self.connector, self.vendor, self.product)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn lg(connector: &str, serial: &str) -> MonitorIdentity {
        MonitorIdentity::new(connector, "GSM", "LG HDR 4K", serial)
    }

    #[test]
    fn full_edid_match_beats_model_match() {
        let a = lg("DP-7", "0x0004ee0e");
        let b = lg("DP-5", "0x0004ee0e"); // same panel, different connector
        let c = lg("DP-8", "0x0003e924"); // same model, different panel
        assert_eq!(a.match_level(&b), MatchLevel::FullEdid);
        assert_eq!(a.match_level(&c), MatchLevel::Model);
    }

    #[test]
    fn connector_fallback_when_no_edid() {
        let a = MonitorIdentity::new("HDMI-1", "", "", "");
        let b = MonitorIdentity::new("HDMI-1", "LTM", "Lontium semi", "0x88888800");
        assert_eq!(a.match_level(&b), MatchLevel::Connector);
        assert_eq!(a.stable_key(), "connector:HDMI-1");
        assert!(b.has_edid());
    }

    #[test]
    fn identical_models_have_distinct_stable_keys() {
        let a = lg("DP-7", "0x0004ee0e");
        let b = lg("DP-8", "0x0003e924");
        assert_ne!(a.stable_key(), b.stable_key());
    }

    #[test]
    fn synthetic_serials() {
        assert!(MonitorIdentity::new("HDMI-1", "LTM", "x", "0x88888800").serial_looks_synthetic());
        assert!(MonitorIdentity::new("HDMI-1", "LTM", "x", "0x00000000").serial_looks_synthetic());
        assert!(MonitorIdentity::new("HDMI-1", "LTM", "x", "").serial_looks_synthetic());
        assert!(!MonitorIdentity::new("DP-7", "GSM", "x", "0x0004ee0e").serial_looks_synthetic());
        assert!(
            !MonitorIdentity::new("HDMI-1", "AOC", "x", "AHLL19A000036").serial_looks_synthetic()
        );
    }
}
