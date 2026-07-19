//! User preferences: friendly monitor names, KVM overrides, app settings.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::monitor::PhysicalMonitor;

pub const DEFAULT_CONFIRM_SECONDS: u32 = 30;

/// Per-monitor preferences, keyed by [`crate::MonitorIdentity::stable_key`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorPrefs {
    /// User-assigned friendly name (e.g. `KVM`, `Main Monitor`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    /// User override for the KVM heuristic; `None` = automatic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_kvm: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppPrefs {
    /// Seconds before an unconfirmed configuration is reverted.
    #[serde(default = "default_confirm_seconds")]
    pub confirm_seconds: u32,
    #[serde(default)]
    pub monitors: BTreeMap<String, MonitorPrefs>,
}

fn default_confirm_seconds() -> u32 {
    DEFAULT_CONFIRM_SECONDS
}

impl Default for AppPrefs {
    fn default() -> Self {
        Self {
            confirm_seconds: DEFAULT_CONFIRM_SECONDS,
            monitors: BTreeMap::new(),
        }
    }
}

impl AppPrefs {
    pub fn monitor(&self, key: &str) -> Option<&MonitorPrefs> {
        self.monitors.get(key)
    }

    pub fn set_alias(&mut self, key: &str, alias: Option<String>) {
        let entry = self.monitors.entry(key.to_owned()).or_default();
        entry.alias = alias.filter(|a| !a.trim().is_empty());
        self.prune(key);
    }

    pub fn set_kvm_override(&mut self, key: &str, is_kvm: Option<bool>) {
        let entry = self.monitors.entry(key.to_owned()).or_default();
        entry.is_kvm = is_kvm;
        self.prune(key);
    }

    fn prune(&mut self, key: &str) {
        if self
            .monitors
            .get(key)
            .is_some_and(|p| p == &MonitorPrefs::default())
        {
            self.monitors.remove(key);
        }
    }

    /// Display name for a monitor: alias → Mutter display-name → model.
    pub fn display_name(&self, monitor: &PhysicalMonitor) -> String {
        if let Some(alias) = self
            .monitor(&monitor.identity.stable_key())
            .and_then(|p| p.alias.clone())
        {
            return alias;
        }
        if let Some(name) = &monitor.display_name {
            return name.clone();
        }
        let identity = &monitor.identity;
        if identity.has_edid() {
            format!("{} {}", identity.vendor, identity.product)
        } else {
            identity.connector.clone()
        }
    }

    /// Whether the monitor should be treated as the KVM display
    /// (user override wins over the heuristic).
    pub fn is_kvm(&self, monitor: &PhysicalMonitor) -> bool {
        self.monitor(&monitor.identity.stable_key())
            .and_then(|p| p.is_kvm)
            .unwrap_or_else(|| monitor.is_likely_kvm())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::identity::MonitorIdentity;
    use crate::mode::ExtraProps;

    fn kvm() -> PhysicalMonitor {
        PhysicalMonitor {
            identity: MonitorIdentity::new("HDMI-1", "LTM", "Lontium semi", "0x88888800"),
            display_name: Some("LTM 5\"".into()),
            modes: vec![],
            is_builtin: false,
            is_underscanning: false,
            supports_underscanning: false,
            is_for_lease: false,
            color_mode: None,
            supported_color_modes: vec![],
            rgb_range: None,
            min_refresh_rate: None,
            physical_size_mm: None,
            extra: ExtraProps::new(),
        }
    }

    #[test]
    fn alias_persists_and_wins() {
        let monitor = kvm();
        let mut prefs = AppPrefs::default();
        assert_eq!(prefs.display_name(&monitor), "LTM 5\"");
        prefs.set_alias(&monitor.identity.stable_key(), Some("KVM".into()));
        assert_eq!(prefs.display_name(&monitor), "KVM");

        let json = serde_json::to_string(&prefs).unwrap();
        let back: AppPrefs = serde_json::from_str(&json).unwrap();
        assert_eq!(back.display_name(&monitor), "KVM");
    }

    #[test]
    fn kvm_override_beats_heuristic() {
        let monitor = kvm();
        let mut prefs = AppPrefs::default();
        assert!(prefs.is_kvm(&monitor)); // heuristic
        prefs.set_kvm_override(&monitor.identity.stable_key(), Some(false));
        assert!(!prefs.is_kvm(&monitor));
        prefs.set_kvm_override(&monitor.identity.stable_key(), None);
        assert!(prefs.is_kvm(&monitor));
    }

    #[test]
    fn empty_alias_clears_entry() {
        let monitor = kvm();
        let mut prefs = AppPrefs::default();
        prefs.set_alias(&monitor.identity.stable_key(), Some("  ".into()));
        assert!(prefs.monitors.is_empty());
    }
}
