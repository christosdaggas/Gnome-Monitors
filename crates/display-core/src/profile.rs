//! Saved display profiles with conservative, identity-based matching.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::layout::{DisplayLayout, LayoutMode, LogicalDisplay, MonitorAssignment};
use crate::monitor::{ColorMode, PhysicalMonitor, RgbRange};
use crate::state::DisplayState;
use crate::transform::Transform;
use crate::{REFRESH_MATCH_EPS, identity::MonitorIdentity};

/// EDID-based identity stored in profiles. The connector is a hint used only
/// to disambiguate physically identical monitors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentitySpec {
    pub vendor: String,
    pub product: String,
    pub serial: String,
    #[serde(default)]
    pub connector_hint: String,
}

impl IdentitySpec {
    pub fn from_identity(identity: &MonitorIdentity) -> Self {
        Self {
            vendor: identity.vendor.clone(),
            product: identity.product.clone(),
            serial: identity.serial.clone(),
            connector_hint: identity.connector.clone(),
        }
    }

    pub fn describe(&self) -> String {
        if self.vendor.is_empty() && self.product.is_empty() {
            format!("connector {}", self.connector_hint)
        } else {
            format!("{} {} ({})", self.vendor, self.product, self.connector_hint)
        }
    }
}

/// A stored mode: resolution + refresh rate (mode IDs are not stable).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModeSpec {
    pub width: i32,
    pub height: i32,
    pub refresh_hz: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileMonitor {
    pub identity: IdentitySpec,
    pub mode: ModeSpec,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_mode: Option<ColorMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rgb_range: Option<RgbRange>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileLogicalDisplay {
    pub x: i32,
    pub y: i32,
    pub scale: f64,
    #[serde(default)]
    pub transform: Transform,
    #[serde(default)]
    pub primary: bool,
    pub monitors: Vec<ProfileMonitor>,
}

/// A named, saved display configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisplayProfile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub layout_mode: LayoutMode,
    pub logical_displays: Vec<ProfileLogicalDisplay>,
    /// Monitors that were explicitly disabled when the profile was saved.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled: Vec<IdentitySpec>,
    /// Unix timestamps (seconds).
    #[serde(default)]
    pub created_unix: u64,
    #[serde(default)]
    pub modified_unix: u64,
}

impl DisplayProfile {
    /// Captures the given state as a profile.
    pub fn from_state(id: String, name: String, state: &DisplayState, now_unix: u64) -> Self {
        let logical_displays = state
            .layout
            .logical_displays
            .iter()
            .map(|logical| ProfileLogicalDisplay {
                x: logical.x,
                y: logical.y,
                scale: logical.scale,
                transform: logical.transform,
                primary: logical.primary,
                monitors: logical
                    .monitors
                    .iter()
                    .filter_map(|assignment| {
                        let monitor = state.monitor(&assignment.connector)?;
                        let mode = monitor.find_mode(&assignment.mode_id)?;
                        Some(ProfileMonitor {
                            identity: IdentitySpec::from_identity(&monitor.identity),
                            mode: ModeSpec {
                                width: mode.width,
                                height: mode.height,
                                refresh_hz: round3(mode.refresh_hz),
                            },
                            color_mode: assignment.color_mode,
                            rgb_range: assignment.rgb_range,
                        })
                    })
                    .collect(),
            })
            .collect();
        let disabled = state
            .disabled_monitors()
            .iter()
            .map(|m| IdentitySpec::from_identity(&m.identity))
            .collect();
        Self {
            id,
            name,
            layout_mode: state.layout.layout_mode,
            logical_displays,
            disabled,
            created_unix: now_unix,
            modified_unix: now_unix,
        }
    }

    /// Identities this profile involves (enabled monitors only).
    pub fn required_identities(&self) -> impl Iterator<Item = &IdentitySpec> {
        self.logical_displays
            .iter()
            .flat_map(|l| l.monitors.iter().map(|m| &m.identity))
    }

    /// Resolves the profile against the currently connected monitors.
    pub fn resolve(&self, monitors: &[PhysicalMonitor]) -> ProfileResolution {
        let mut problems = Vec::new();
        let mut logical_displays = Vec::new();
        let mut used: Vec<String> = Vec::new();

        for logical in &self.logical_displays {
            let mut assignments = Vec::new();
            for profile_monitor in &logical.monitors {
                match resolve_monitor(&profile_monitor.identity, monitors, &used) {
                    MonitorMatchOutcome::Unique(monitor) => {
                        used.push(monitor.connector().to_owned());
                        match resolve_mode(monitor, &profile_monitor.mode) {
                            Some(mode_id) => assignments.push(MonitorAssignment {
                                connector: monitor.connector().to_owned(),
                                mode_id,
                                color_mode: profile_monitor.color_mode,
                                rgb_range: profile_monitor.rgb_range,
                                underscanning: None,
                            }),
                            None => problems.push(ProfileProblem::ModeMissing {
                                identity: profile_monitor.identity.clone(),
                                mode: profile_monitor.mode.clone(),
                            }),
                        }
                    }
                    MonitorMatchOutcome::Ambiguous(connectors) => {
                        problems.push(ProfileProblem::Ambiguous {
                            identity: profile_monitor.identity.clone(),
                            candidates: connectors,
                        });
                    }
                    MonitorMatchOutcome::Missing { similar_model } => {
                        problems.push(ProfileProblem::Missing {
                            identity: profile_monitor.identity.clone(),
                            similar_model_present: similar_model,
                        });
                    }
                }
            }
            if !assignments.is_empty() {
                logical_displays.push(LogicalDisplay {
                    x: logical.x,
                    y: logical.y,
                    scale: logical.scale,
                    transform: logical.transform,
                    primary: logical.primary,
                    monitors: assignments,
                });
            }
        }

        // Protect monitors the profile knows nothing about: applying the
        // resolved layout would implicitly disable them (disabling is
        // "omission" in the Mutter API), which must never happen silently.
        for monitor in monitors {
            let connector = monitor.connector();
            let matched = used.iter().any(|c| c == connector);
            let recorded_disabled = self.disabled.iter().any(|spec| {
                (!spec.vendor.is_empty()
                    && spec.vendor == monitor.identity.vendor
                    && spec.product == monitor.identity.product
                    && spec.serial == monitor.identity.serial)
                    || (spec.vendor.is_empty() && spec.connector_hint == connector)
            });
            if !matched && !recorded_disabled {
                problems.push(ProfileProblem::UnexpectedMonitor {
                    connector: connector.to_owned(),
                    name: monitor
                        .display_name
                        .clone()
                        .unwrap_or_else(|| connector.to_owned()),
                });
            }
        }

        let layout = DisplayLayout {
            layout_mode: self.layout_mode,
            logical_displays,
        };
        ProfileResolution { layout, problems }
    }
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

enum MonitorMatchOutcome<'a> {
    Unique(&'a PhysicalMonitor),
    Ambiguous(Vec<String>),
    Missing { similar_model: bool },
}

fn resolve_monitor<'a>(
    spec: &IdentitySpec,
    monitors: &'a [PhysicalMonitor],
    used: &[String],
) -> MonitorMatchOutcome<'a> {
    let available: Vec<&PhysicalMonitor> = monitors
        .iter()
        .filter(|m| !used.contains(&m.connector().to_owned()))
        .collect();

    let has_edid = !(spec.vendor.is_empty() && spec.product.is_empty() && spec.serial.is_empty());
    if !has_edid {
        // Documented fallback: monitors without any EDID identity are matched
        // by connector.
        return match available
            .iter()
            .find(|m| m.connector() == spec.connector_hint)
        {
            Some(m) => MonitorMatchOutcome::Unique(m),
            None => MonitorMatchOutcome::Missing {
                similar_model: false,
            },
        };
    }

    let full: Vec<&&PhysicalMonitor> = available
        .iter()
        .filter(|m| {
            m.identity.vendor == spec.vendor
                && m.identity.product == spec.product
                && m.identity.serial == spec.serial
        })
        .collect();

    match full.len() {
        1 => MonitorMatchOutcome::Unique(full[0]),
        0 => {
            let similar = available
                .iter()
                .any(|m| m.identity.vendor == spec.vendor && m.identity.product == spec.product);
            MonitorMatchOutcome::Missing {
                similar_model: similar,
            }
        }
        _ => {
            // Physically identical monitors (same serial too — some vendors
            // ship EDIDs with duplicate serials). Use the connector hint.
            if let Some(m) = full.iter().find(|m| m.connector() == spec.connector_hint) {
                MonitorMatchOutcome::Unique(m)
            } else {
                MonitorMatchOutcome::Ambiguous(
                    full.iter().map(|m| m.connector().to_owned()).collect(),
                )
            }
        }
    }
}

fn resolve_mode(monitor: &PhysicalMonitor, spec: &ModeSpec) -> Option<String> {
    monitor
        .modes
        .iter()
        .filter(|m| m.width == spec.width && m.height == spec.height)
        .filter(|m| (m.refresh_hz - spec.refresh_hz).abs() <= REFRESH_MATCH_EPS)
        .min_by(|a, b| {
            (a.refresh_hz - spec.refresh_hz)
                .abs()
                .partial_cmp(&(b.refresh_hz - spec.refresh_hz).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|m| m.id.clone())
}

/// Why (part of) a profile could not be applied.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ProfileProblem {
    #[error("{} is not connected{}.", identity.describe(), if *similar_model_present { ", but another monitor of the same model is" } else { "" })]
    Missing {
        identity: IdentitySpec,
        similar_model_present: bool,
    },
    #[error("{} matches several connected monitors ({}); the profile cannot be applied unambiguously.", identity.describe(), candidates.join(", "))]
    Ambiguous {
        identity: IdentitySpec,
        candidates: Vec<String>,
    },
    #[error("{} no longer offers {}×{} at {:.3} Hz.", identity.describe(), mode.width, mode.height, mode.refresh_hz)]
    ModeMissing {
        identity: IdentitySpec,
        mode: ModeSpec,
    },
    #[error(
        "{name} ({connector}) is connected but not part of this profile; applying it as saved would turn that display off."
    )]
    UnexpectedMonitor { connector: String, name: String },
}

/// Result of resolving a profile against connected monitors.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileResolution {
    /// The layout, containing only the monitors that resolved. Complete only
    /// when `problems` is empty.
    pub layout: DisplayLayout,
    pub problems: Vec<ProfileProblem>,
}

impl ProfileResolution {
    /// True when every profile monitor resolved to a connected monitor and
    /// mode. Applying a partially resolved profile requires explicit user
    /// consent.
    pub fn is_exact(&self) -> bool {
        self.problems.is_empty()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::mode::{ExtraProps, MonitorMode};

    fn monitor(connector: &str, vendor: &str, product: &str, serial: &str) -> PhysicalMonitor {
        PhysicalMonitor {
            identity: MonitorIdentity::new(connector, vendor, product, serial),
            display_name: None,
            modes: vec![
                MonitorMode {
                    id: "3840x2160@59.997".into(),
                    width: 3840,
                    height: 2160,
                    refresh_hz: 59.996_623_992_919_92,
                    preferred_scale: 2.0,
                    supported_scales: vec![1.0, 1.5, 2.0],
                    is_current: true,
                    is_preferred: true,
                    is_interlaced: false,
                    refresh_rate_mode: None,
                    extra: ExtraProps::new(),
                },
                MonitorMode {
                    id: "1920x1080@60.000".into(),
                    width: 1920,
                    height: 1080,
                    refresh_hz: 60.0,
                    preferred_scale: 1.0,
                    supported_scales: vec![1.0, 1.25],
                    is_current: false,
                    is_preferred: false,
                    is_interlaced: false,
                    refresh_rate_mode: None,
                    extra: ExtraProps::new(),
                },
            ],
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

    fn state_two_lg() -> DisplayState {
        DisplayState {
            serial: 7,
            monitors: vec![
                monitor("DP-7", "GSM", "LG HDR 4K", "0x0004ee0e"),
                monitor("DP-8", "GSM", "LG HDR 4K", "0x0003e924"),
            ],
            layout: DisplayLayout {
                layout_mode: LayoutMode::Logical,
                logical_displays: vec![
                    LogicalDisplay {
                        x: 0,
                        y: 0,
                        scale: 2.0,
                        transform: Transform::Normal,
                        primary: true,
                        monitors: vec![MonitorAssignment {
                            connector: "DP-7".into(),
                            mode_id: "3840x2160@59.997".into(),
                            color_mode: None,
                            rgb_range: None,
                            underscanning: None,
                        }],
                    },
                    LogicalDisplay {
                        x: 1920,
                        y: 0,
                        scale: 2.0,
                        transform: Transform::Normal,
                        primary: false,
                        monitors: vec![MonitorAssignment {
                            connector: "DP-8".into(),
                            mode_id: "3840x2160@59.997".into(),
                            color_mode: None,
                            rgb_range: None,
                            underscanning: None,
                        }],
                    },
                ],
            },
            supports_changing_layout_mode: true,
            global_scale_required: false,
            supports_mirroring: None,
            extra: ExtraProps::new(),
        }
    }

    #[test]
    fn roundtrip_profile_resolves_after_connector_swap() {
        let state = state_two_lg();
        let profile = DisplayProfile::from_state("p1".into(), "Two LG".into(), &state, 1_000);

        // Same panels, swapped connectors (as observed in monitors.xml
        // history on the audited machine).
        let mut swapped = state.monitors.clone();
        swapped[0].identity.connector = "DP-5".into();
        swapped[1].identity.connector = "DP-6".into();

        let resolution = profile.resolve(&swapped);
        assert!(resolution.is_exact(), "problems: {:?}", resolution.problems);
        // The panel with serial 0x0004ee0e is now DP-5 and must stay primary.
        let primary = resolution.layout.primary().unwrap();
        assert_eq!(primary.monitors[0].connector, "DP-5");
    }

    #[test]
    fn missing_monitor_is_reported_with_model_hint() {
        let state = state_two_lg();
        let profile = DisplayProfile::from_state("p1".into(), "Two LG".into(), &state, 0);
        let only_one = vec![monitor("DP-7", "GSM", "LG HDR 4K", "0x0004ee0e")];
        let resolution = profile.resolve(&only_one);
        assert!(!resolution.is_exact());
        assert!(matches!(
            &resolution.problems[0],
            ProfileProblem::Missing {
                similar_model_present: false,
                ..
            }
        ));

        // A same-model panel with a different serial gives the hint, and the
        // unknown panel itself is flagged as would-be-disabled.
        let different_serial = vec![monitor("DP-7", "GSM", "LG HDR 4K", "0xdeadbeef")];
        let resolution = profile.resolve(&different_serial);
        let missing_with_hint = resolution
            .problems
            .iter()
            .filter(|p| {
                matches!(
                    p,
                    ProfileProblem::Missing {
                        similar_model_present: true,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(missing_with_hint, 2);
        assert!(
            resolution
                .problems
                .iter()
                .any(|p| matches!(p, ProfileProblem::UnexpectedMonitor { .. }))
        );
    }

    #[test]
    fn duplicate_serials_use_connector_hint_then_report_ambiguity() {
        // Two monitors with byte-identical EDID identity.
        let twins = vec![
            monitor("HDMI-1", "AAA", "Twin", "123"),
            monitor("HDMI-2", "AAA", "Twin", "123"),
        ];
        let spec = IdentitySpec {
            vendor: "AAA".into(),
            product: "Twin".into(),
            serial: "123".into(),
            connector_hint: "HDMI-2".into(),
        };
        match resolve_monitor(&spec, &twins, &[]) {
            MonitorMatchOutcome::Unique(m) => assert_eq!(m.connector(), "HDMI-2"),
            _ => panic!("expected unique via hint"),
        }
        let spec_no_hint = IdentitySpec {
            connector_hint: "DP-9".into(),
            ..spec
        };
        assert!(matches!(
            resolve_monitor(&spec_no_hint, &twins, &[]),
            MonitorMatchOutcome::Ambiguous(c) if c == vec!["HDMI-1".to_owned(), "HDMI-2".to_owned()]
        ));
    }

    #[test]
    fn edid_less_monitor_falls_back_to_connector() {
        let anon = vec![monitor("HDMI-1", "", "", "")];
        let spec = IdentitySpec {
            vendor: String::new(),
            product: String::new(),
            serial: String::new(),
            connector_hint: "HDMI-1".into(),
        };
        assert!(matches!(
            resolve_monitor(&spec, &anon, &[]),
            MonitorMatchOutcome::Unique(_)
        ));
    }

    #[test]
    fn mode_matching_uses_refresh_tolerance() {
        let m = monitor("DP-7", "GSM", "LG HDR 4K", "1");
        // Stored rounded 59.997 must match the exact 59.99662399…
        let id = resolve_mode(
            &m,
            &ModeSpec {
                width: 3840,
                height: 2160,
                refresh_hz: 59.997,
            },
        );
        assert_eq!(id.as_deref(), Some("3840x2160@59.997"));
        // A clearly different rate must not match.
        let none = resolve_mode(
            &m,
            &ModeSpec {
                width: 3840,
                height: 2160,
                refresh_hz: 30.0,
            },
        );
        assert_eq!(none, None);
    }

    #[test]
    fn profile_serialization_roundtrip() {
        let state = state_two_lg();
        let profile = DisplayProfile::from_state("p1".into(), "Two LG".into(), &state, 42);
        let json = serde_json::to_string_pretty(&profile).unwrap();
        let back: DisplayProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(profile, back);
        // Transforms serialize as friendly strings.
        assert!(json.contains("\"transform\": \"normal\""));
    }

    #[test]
    fn profile_migration_tolerates_unknown_fields() {
        // A future version may add fields; loading must not fail.
        let json = r#"{
            "id": "x", "name": "Future", "layout_mode": "logical",
            "logical_displays": [], "some_future_field": {"a": 1}
        }"#;
        let profile: DisplayProfile = serde_json::from_str(json).unwrap();
        assert_eq!(profile.name, "Future");
    }

    #[test]
    fn newly_connected_monitor_is_flagged_not_silently_disabled() {
        let state = state_two_lg();
        let profile = DisplayProfile::from_state("p1".into(), "Two LG".into(), &state, 0);
        // A third monitor appears that the profile has never seen.
        let mut monitors = state.monitors.clone();
        monitors.push(monitor("HDMI-1", "LTM", "Lontium semi", "0x88888800"));
        let resolution = profile.resolve(&monitors);
        assert!(!resolution.is_exact());
        assert!(resolution.problems.iter().any(|p| matches!(
            p,
            ProfileProblem::UnexpectedMonitor { connector, .. } if connector == "HDMI-1"
        )));
        // Monitors the profile recorded as disabled stay exact.
        let resolution = profile.resolve(&state.monitors);
        assert!(resolution.is_exact());
    }
}
