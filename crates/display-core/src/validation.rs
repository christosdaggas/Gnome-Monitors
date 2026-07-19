//! Client-side validation of proposed layouts.
//!
//! These rules mirror what Mutter 50.2 enforces server-side (empirically
//! probed with verify-mode `ApplyMonitorsConfig` calls — see
//! `docs/system-audit.md`) so problems can be explained in friendly terms
//! before the compositor is involved. The compositor's verify mode remains
//! the final authority and is still consulted before any apply.

use std::collections::BTreeSet;

use thiserror::Error;

use crate::geometry::Rect;
use crate::layout::{DisplayLayout, LayoutMode, LogicalDisplay};
use crate::monitor::PhysicalMonitor;

/// A problem found in a proposed layout.
///
/// `Display` renders a human-readable English message; the UI may map
/// variants to localized strings.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum LayoutProblem {
    #[error("The layout has no enabled display.")]
    NoActiveDisplays,
    #[error("No display is marked as primary.")]
    MissingPrimary,
    #[error("More than one display is marked as primary.")]
    MultiplePrimary,
    #[error("Monitor {connector} is not currently connected.")]
    UnknownConnector { connector: String },
    #[error("Monitor {connector} appears in more than one display group.")]
    MonitorInMultipleGroups { connector: String },
    #[error("Monitor {connector} does not offer mode {mode_id}.")]
    UnknownMode { connector: String, mode_id: String },
    #[error(
        "The displays mirrored in group {group} are set to different resolutions ({details}); mirrored displays must use the same resolution."
    )]
    MirrorResolutionMismatch { group: usize, details: String },
    #[error("A mirror group has no monitors.")]
    EmptyGroup,
    #[error("Scale {scale} is not available for mode {mode_id} of monitor {connector}.")]
    UnsupportedScale {
        connector: String,
        mode_id: String,
        scale: f64,
    },
    #[error("Scale {scale} is not valid.")]
    InvalidScale { scale: f64 },
    #[error("Displays {a} and {b} overlap.")]
    Overlap { a: usize, b: usize },
    #[error("The displays are not all adjacent; every display must touch another one.")]
    NotAdjacent,
    #[error("The layout origin is at ({min_x}, {min_y}) instead of (0, 0).")]
    NotNormalized { min_x: i32, min_y: i32 },
    #[error(
        "All displays must use the same scale on this system ({details}); the compositor requires a global scale."
    )]
    GlobalScaleMismatch { details: String },
    #[error(
        "Scale {scale} is fractional, but the physical layout mode only allows whole-number scales."
    )]
    FractionalScaleInPhysicalMode { scale: f64 },
    #[error("This system does not support mirrored displays.")]
    MirroringUnsupported,
    #[error(
        "Monitor {connector} is reserved for lease (e.g. VR) and cannot be part of the layout."
    )]
    MonitorForLease { connector: String },
    #[error("Monitor {connector} does not support the selected color mode.")]
    UnsupportedColorMode { connector: String },
}

impl LayoutProblem {
    /// `NotNormalized` is fixable by [`normalize`]; everything else requires
    /// the user (or caller) to change the layout.
    pub fn is_auto_fixable(&self) -> bool {
        matches!(self, LayoutProblem::NotNormalized { .. })
    }
}

/// Compositor policy flags that affect validation, taken from
/// `GetCurrentState` properties.
#[derive(Debug, Clone, Copy)]
pub struct ValidationPolicy {
    /// `global-scale-required`: all logical displays must share one scale.
    pub global_scale_required: bool,
    /// `supports-mirroring` (absent means supported).
    pub supports_mirroring: bool,
}

impl Default for ValidationPolicy {
    fn default() -> Self {
        Self {
            global_scale_required: false,
            supports_mirroring: true,
        }
    }
}

impl ValidationPolicy {
    pub fn from_state(state: &crate::DisplayState) -> Self {
        Self {
            global_scale_required: state.global_scale_required,
            supports_mirroring: state.supports_mirroring.unwrap_or(true),
        }
    }
}

/// Validates `layout` against a full compositor state (monitors + policy
/// flags). Prefer this over [`validate`] whenever a [`crate::DisplayState`]
/// is available.
pub fn validate_state(state: &crate::DisplayState, layout: &DisplayLayout) -> Vec<LayoutProblem> {
    validate_with_policy(&state.monitors, layout, ValidationPolicy::from_state(state))
}

/// Validates `layout` against the connected `monitors` with default policy
/// (no global-scale requirement, mirroring supported).
///
/// Returns all problems found (empty = valid). Geometry checks are skipped
/// for logical displays whose mode cannot be resolved (those already produce
/// `UnknownConnector` / `UnknownMode`).
pub fn validate(monitors: &[PhysicalMonitor], layout: &DisplayLayout) -> Vec<LayoutProblem> {
    validate_with_policy(monitors, layout, ValidationPolicy::default())
}

/// Full validation with explicit policy flags.
pub fn validate_with_policy(
    monitors: &[PhysicalMonitor],
    layout: &DisplayLayout,
    policy: ValidationPolicy,
) -> Vec<LayoutProblem> {
    let mut problems = Vec::new();

    if layout.logical_displays.is_empty() {
        problems.push(LayoutProblem::NoActiveDisplays);
        return problems;
    }

    // Monitor / mode resolution and duplicates.
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for logical in &layout.logical_displays {
        if logical.monitors.is_empty() {
            problems.push(LayoutProblem::EmptyGroup);
        }
        if logical.scale <= 0.0 || !logical.scale.is_finite() {
            problems.push(LayoutProblem::InvalidScale {
                scale: logical.scale,
            });
        }
        for assignment in &logical.monitors {
            if !seen.insert(assignment.connector.as_str()) {
                problems.push(LayoutProblem::MonitorInMultipleGroups {
                    connector: assignment.connector.clone(),
                });
            }
            match monitors
                .iter()
                .find(|m| m.connector() == assignment.connector)
            {
                None => problems.push(LayoutProblem::UnknownConnector {
                    connector: assignment.connector.clone(),
                }),
                Some(monitor) => match monitor.find_mode(&assignment.mode_id) {
                    None => problems.push(LayoutProblem::UnknownMode {
                        connector: assignment.connector.clone(),
                        mode_id: assignment.mode_id.clone(),
                    }),
                    Some(mode) => {
                        if layout.layout_mode == LayoutMode::Logical
                            && logical.scale > 0.0
                            && !mode.supports_scale(logical.scale)
                        {
                            problems.push(LayoutProblem::UnsupportedScale {
                                connector: assignment.connector.clone(),
                                mode_id: assignment.mode_id.clone(),
                                scale: logical.scale,
                            });
                        }
                    }
                },
            }
        }
    }

    // Policy: mirroring support.
    if !policy.supports_mirroring
        && layout
            .logical_displays
            .iter()
            .any(LogicalDisplay::is_mirror_group)
    {
        problems.push(LayoutProblem::MirroringUnsupported);
    }

    // Policy: one global scale when the compositor demands it.
    if policy.global_scale_required {
        let scales: Vec<f64> = layout.logical_displays.iter().map(|l| l.scale).collect();
        if let Some(first) = scales.first()
            && scales
                .iter()
                .any(|s| !crate::approx_eq(*s, *first, crate::SCALE_EPS))
        {
            let details = scales
                .iter()
                .map(|s| format!("{s}"))
                .collect::<Vec<_>>()
                .join(", ");
            problems.push(LayoutProblem::GlobalScaleMismatch { details });
        }
    }

    // Physical layout mode allows integer scales only (Mutter:
    // "A fractional scale with physical layout mode not allowed").
    if layout.layout_mode == LayoutMode::Physical {
        for logical in &layout.logical_displays {
            if (logical.scale - logical.scale.round()).abs() > crate::SCALE_EPS {
                problems.push(LayoutProblem::FractionalScaleInPhysicalMode {
                    scale: logical.scale,
                });
            }
        }
    }

    // Per-monitor capability checks: leased monitors and color modes.
    for logical in &layout.logical_displays {
        for assignment in &logical.monitors {
            let Some(monitor) = monitors
                .iter()
                .find(|m| m.connector() == assignment.connector)
            else {
                continue;
            };
            if monitor.is_for_lease {
                problems.push(LayoutProblem::MonitorForLease {
                    connector: assignment.connector.clone(),
                });
            }
            if let Some(color_mode) = assignment.color_mode
                && !monitor.supported_color_modes.is_empty()
                && !monitor.supported_color_modes.contains(&color_mode)
            {
                problems.push(LayoutProblem::UnsupportedColorMode {
                    connector: assignment.connector.clone(),
                });
            }
        }
    }

    // Mirror groups: all members must share one mode resolution.
    for (index, logical) in layout.logical_displays.iter().enumerate() {
        if logical.monitors.len() < 2 {
            continue;
        }
        let mut sizes: Vec<(String, (i32, i32))> = Vec::new();
        for assignment in &logical.monitors {
            if let Some(mode) = monitors
                .iter()
                .find(|m| m.connector() == assignment.connector)
                .and_then(|m| m.find_mode(&assignment.mode_id))
            {
                sizes.push((assignment.connector.clone(), (mode.width, mode.height)));
            }
        }
        let distinct: BTreeSet<(i32, i32)> = sizes.iter().map(|(_, s)| *s).collect();
        if distinct.len() > 1 {
            let details = sizes
                .iter()
                .map(|(c, (w, h))| format!("{c}: {w}×{h}"))
                .collect::<Vec<_>>()
                .join(", ");
            problems.push(LayoutProblem::MirrorResolutionMismatch {
                group: index + 1,
                details,
            });
        }
    }

    // Primary flags.
    let primaries = layout.logical_displays.iter().filter(|l| l.primary).count();
    if primaries == 0 {
        problems.push(LayoutProblem::MissingPrimary);
    } else if primaries > 1 {
        problems.push(LayoutProblem::MultiplePrimary);
    }

    // Geometry.
    let rects: Vec<Option<Rect>> = layout
        .logical_displays
        .iter()
        .map(|l| l.rect(monitors, layout.layout_mode))
        .collect();
    let resolved: Vec<(usize, Rect)> = rects
        .iter()
        .enumerate()
        .filter_map(|(i, r)| r.map(|r| (i, r)))
        .collect();

    for (ai, (a_index, a)) in resolved.iter().enumerate() {
        for (b_index, b) in resolved.iter().skip(ai + 1).map(|(i, r)| (i, r)) {
            if a.overlaps(b) {
                problems.push(LayoutProblem::Overlap {
                    a: a_index + 1,
                    b: *b_index + 1,
                });
            }
        }
    }

    if resolved.len() > 1 && !all_connected(&resolved) {
        problems.push(LayoutProblem::NotAdjacent);
    }

    if let Some(bb) = crate::geometry::bounding_box(resolved.iter().map(|(_, r)| r))
        && (bb.x != 0 || bb.y != 0)
    {
        problems.push(LayoutProblem::NotNormalized {
            min_x: bb.x,
            min_y: bb.y,
        });
    }

    problems
}

/// True when the problem list allows applying after auto-fixes.
pub fn is_appliable(problems: &[LayoutProblem]) -> bool {
    problems.iter().all(LayoutProblem::is_auto_fixable)
}

/// Translates the whole layout so its bounding-box origin is `(0, 0)` —
/// Mutter requires normalized configurations.
pub fn normalize(layout: &mut DisplayLayout, monitors: &[PhysicalMonitor]) {
    let rects: Vec<Rect> = layout
        .logical_displays
        .iter()
        .filter_map(|l| l.rect(monitors, layout.layout_mode))
        .collect();
    let Some(bb) = crate::geometry::bounding_box(rects.iter()) else {
        return;
    };
    if bb.x != 0 || bb.y != 0 {
        for logical in &mut layout.logical_displays {
            logical.x -= bb.x;
            logical.y -= bb.y;
        }
    }
}

/// Union-find-free connectivity check over Mutter's adjacency relation.
fn all_connected(rects: &[(usize, Rect)]) -> bool {
    if rects.is_empty() {
        return true;
    }
    let mut visited = vec![false; rects.len()];
    let mut stack = vec![0usize];
    visited[0] = true;
    let mut count = 1;
    while let Some(i) = stack.pop() {
        for j in 0..rects.len() {
            if !visited[j] && rects[i].1.is_adjacent_to(&rects[j].1) {
                visited[j] = true;
                count += 1;
                stack.push(j);
            }
        }
    }
    count == rects.len()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::identity::MonitorIdentity;
    use crate::layout::{LogicalDisplay, MonitorAssignment};
    use crate::mode::{ExtraProps, MonitorMode};
    use crate::transform::Transform;

    fn mk_monitor(connector: &str, serial: &str, modes: &[(i32, i32, f64)]) -> PhysicalMonitor {
        PhysicalMonitor {
            identity: MonitorIdentity::new(connector, "GSM", "LG HDR 4K", serial),
            display_name: None,
            modes: modes
                .iter()
                .enumerate()
                .map(|(i, (w, h, hz))| MonitorMode {
                    id: format!("{w}x{h}@{hz:.3}"),
                    width: *w,
                    height: *h,
                    refresh_hz: *hz,
                    preferred_scale: 1.0,
                    supported_scales: vec![1.0, 1.5, 2.0],
                    is_current: i == 0,
                    is_preferred: i == 0,
                    is_interlaced: false,
                    refresh_rate_mode: None,
                    extra: ExtraProps::new(),
                })
                .collect(),
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

    fn assignment(connector: &str, mode: &str) -> MonitorAssignment {
        MonitorAssignment {
            connector: connector.into(),
            mode_id: mode.into(),
            color_mode: None,
            rgb_range: None,
            underscanning: None,
        }
    }

    fn logical(
        x: i32,
        y: i32,
        scale: f64,
        primary: bool,
        monitors: Vec<MonitorAssignment>,
    ) -> LogicalDisplay {
        LogicalDisplay {
            x,
            y,
            scale,
            transform: Transform::Normal,
            primary,
            monitors,
        }
    }

    fn monitors3() -> Vec<PhysicalMonitor> {
        vec![
            mk_monitor(
                "DP-7",
                "0x0004ee0e",
                &[(3840, 2160, 59.997), (1920, 1080, 60.0)],
            ),
            mk_monitor(
                "DP-8",
                "0x0003e924",
                &[(3840, 2160, 59.997), (1920, 1080, 60.0)],
            ),
            mk_monitor(
                "HDMI-1",
                "0x88888800",
                &[(3840, 2160, 30.0), (1920, 1080, 60.0)],
            ),
        ]
    }

    #[test]
    fn valid_partial_mirror_layout() {
        let monitors = monitors3();
        let layout = DisplayLayout {
            layout_mode: LayoutMode::Logical,
            logical_displays: vec![
                logical(
                    0,
                    0,
                    2.0,
                    true,
                    vec![
                        assignment("DP-7", "3840x2160@59.997"),
                        assignment("HDMI-1", "3840x2160@30.000"),
                    ],
                ),
                logical(
                    1920,
                    0,
                    2.0,
                    false,
                    vec![assignment("DP-8", "3840x2160@59.997")],
                ),
            ],
        };
        assert_eq!(validate(&monitors, &layout), vec![]);
    }

    #[test]
    fn detects_gap_and_overlap() {
        let monitors = monitors3();
        let mut layout = DisplayLayout {
            layout_mode: LayoutMode::Logical,
            logical_displays: vec![
                logical(
                    0,
                    0,
                    2.0,
                    true,
                    vec![assignment("DP-7", "3840x2160@59.997")],
                ),
                logical(
                    2000,
                    0,
                    2.0,
                    false,
                    vec![assignment("DP-8", "3840x2160@59.997")],
                ),
            ],
        };
        assert_eq!(
            validate(&monitors, &layout),
            vec![LayoutProblem::NotAdjacent]
        );

        layout.logical_displays[1].x = 1000;
        let problems = validate(&monitors, &layout);
        assert!(problems.contains(&LayoutProblem::Overlap { a: 1, b: 2 }));
    }

    #[test]
    fn detects_primary_problems() {
        let monitors = monitors3();
        let mut layout = DisplayLayout {
            layout_mode: LayoutMode::Logical,
            logical_displays: vec![
                logical(
                    0,
                    0,
                    2.0,
                    false,
                    vec![assignment("DP-7", "3840x2160@59.997")],
                ),
                logical(
                    1920,
                    0,
                    2.0,
                    false,
                    vec![assignment("DP-8", "3840x2160@59.997")],
                ),
            ],
        };
        assert!(validate(&monitors, &layout).contains(&LayoutProblem::MissingPrimary));
        layout.logical_displays[0].primary = true;
        layout.logical_displays[1].primary = true;
        assert!(validate(&monitors, &layout).contains(&LayoutProblem::MultiplePrimary));
    }

    #[test]
    fn detects_mirror_resolution_mismatch() {
        let monitors = monitors3();
        let layout = DisplayLayout {
            layout_mode: LayoutMode::Logical,
            logical_displays: vec![logical(
                0,
                0,
                1.0,
                true,
                vec![
                    assignment("DP-7", "3840x2160@59.997"),
                    assignment("HDMI-1", "1920x1080@60.000"),
                ],
            )],
        };
        let problems = validate(&monitors, &layout);
        assert!(matches!(
            problems[0],
            LayoutProblem::MirrorResolutionMismatch { group: 1, .. }
        ));
    }

    #[test]
    fn detects_unknown_connector_mode_and_stale_entities() {
        let monitors = monitors3();
        let layout = DisplayLayout {
            layout_mode: LayoutMode::Logical,
            logical_displays: vec![
                logical(
                    0,
                    0,
                    1.0,
                    true,
                    vec![assignment("DP-9", "1920x1080@60.000")],
                ),
                logical(
                    0,
                    1080,
                    1.0,
                    false,
                    vec![assignment("DP-7", "800x600@56.000")],
                ),
            ],
        };
        let problems = validate(&monitors, &layout);
        assert!(problems.contains(&LayoutProblem::UnknownConnector {
            connector: "DP-9".into()
        }));
        assert!(problems.contains(&LayoutProblem::UnknownMode {
            connector: "DP-7".into(),
            mode_id: "800x600@56.000".into()
        }));
    }

    #[test]
    fn detects_unsupported_scale() {
        let monitors = monitors3();
        let layout = DisplayLayout {
            layout_mode: LayoutMode::Logical,
            logical_displays: vec![logical(
                0,
                0,
                1.75,
                true,
                vec![assignment("DP-7", "3840x2160@59.997")],
            )],
        };
        let problems = validate(&monitors, &layout);
        assert!(matches!(
            problems[0],
            LayoutProblem::UnsupportedScale { .. }
        ));
    }

    #[test]
    fn detects_duplicate_monitor_across_groups() {
        let monitors = monitors3();
        let layout = DisplayLayout {
            layout_mode: LayoutMode::Logical,
            logical_displays: vec![
                logical(
                    0,
                    0,
                    2.0,
                    true,
                    vec![assignment("DP-7", "3840x2160@59.997")],
                ),
                logical(
                    1920,
                    0,
                    2.0,
                    false,
                    vec![assignment("DP-7", "3840x2160@59.997")],
                ),
            ],
        };
        let problems = validate(&monitors, &layout);
        assert!(problems.contains(&LayoutProblem::MonitorInMultipleGroups {
            connector: "DP-7".into()
        }));
    }

    #[test]
    fn normalize_moves_origin() {
        let monitors = monitors3();
        let mut layout = DisplayLayout {
            layout_mode: LayoutMode::Logical,
            logical_displays: vec![
                logical(
                    -1920,
                    100,
                    2.0,
                    true,
                    vec![assignment("DP-7", "3840x2160@59.997")],
                ),
                logical(
                    0,
                    100,
                    2.0,
                    false,
                    vec![assignment("DP-8", "3840x2160@59.997")],
                ),
            ],
        };
        let problems = validate(&monitors, &layout);
        assert!(problems.contains(&LayoutProblem::NotNormalized {
            min_x: -1920,
            min_y: 100
        }));
        assert!(is_appliable(&problems));
        normalize(&mut layout, &monitors);
        assert_eq!(
            (layout.logical_displays[0].x, layout.logical_displays[0].y),
            (0, 0)
        );
        assert_eq!(
            (layout.logical_displays[1].x, layout.logical_displays[1].y),
            (1920, 0)
        );
        assert_eq!(validate(&monitors, &layout), vec![]);
    }

    #[test]
    fn rotated_display_uses_transformed_size() {
        let monitors = monitors3();
        // DP-7 rotated 90°: logical size becomes 1080x1920 at scale 1... at
        // scale 2.0 → 540x1080? No: 2160x3840 / 2 = 1080x1920.
        let layout = DisplayLayout {
            layout_mode: LayoutMode::Logical,
            logical_displays: vec![
                LogicalDisplay {
                    x: 0,
                    y: 0,
                    scale: 2.0,
                    transform: Transform::Rotate90,
                    primary: true,
                    monitors: vec![assignment("DP-7", "3840x2160@59.997")],
                },
                logical(
                    1080,
                    0,
                    2.0,
                    false,
                    vec![assignment("DP-8", "3840x2160@59.997")],
                ),
            ],
        };
        assert_eq!(validate(&monitors, &layout), vec![]);
        let rect = layout.logical_displays[0]
            .rect(&monitors, LayoutMode::Logical)
            .unwrap();
        assert_eq!((rect.width, rect.height), (1080, 1920));
    }

    #[test]
    fn empty_layout_is_rejected() {
        let monitors = monitors3();
        let layout = DisplayLayout {
            layout_mode: LayoutMode::Logical,
            logical_displays: vec![],
        };
        assert_eq!(
            validate(&monitors, &layout),
            vec![LayoutProblem::NoActiveDisplays]
        );
        assert!(!is_appliable(&validate(&monitors, &layout)));
    }

    #[test]
    fn policy_global_scale_and_mirroring() {
        let monitors = monitors3();
        let layout = DisplayLayout {
            layout_mode: LayoutMode::Logical,
            logical_displays: vec![
                logical(
                    0,
                    0,
                    2.0,
                    true,
                    vec![assignment("DP-7", "3840x2160@59.997")],
                ),
                logical(
                    1920,
                    0,
                    1.5,
                    false,
                    vec![assignment("DP-8", "3840x2160@59.997")],
                ),
            ],
        };
        // Default policy: differing scales are fine.
        assert!(validate(&monitors, &layout).is_empty());
        let strict = ValidationPolicy {
            global_scale_required: true,
            supports_mirroring: false,
        };
        let problems = validate_with_policy(&monitors, &layout, strict);
        assert!(
            problems
                .iter()
                .any(|p| matches!(p, LayoutProblem::GlobalScaleMismatch { .. }))
        );

        // Mirroring rejected when unsupported.
        let mirror = DisplayLayout {
            layout_mode: LayoutMode::Logical,
            logical_displays: vec![logical(
                0,
                0,
                2.0,
                true,
                vec![
                    assignment("DP-7", "3840x2160@59.997"),
                    assignment("HDMI-1", "3840x2160@30.000"),
                ],
            )],
        };
        let problems = validate_with_policy(&monitors, &mirror, strict);
        assert!(problems.contains(&LayoutProblem::MirroringUnsupported));
        assert!(validate(&monitors, &mirror).is_empty());
    }

    #[test]
    fn physical_mode_requires_integer_scales() {
        let monitors = monitors3();
        let layout = DisplayLayout {
            layout_mode: LayoutMode::Physical,
            logical_displays: vec![
                logical(
                    0,
                    0,
                    1.5,
                    true,
                    vec![assignment("DP-7", "3840x2160@59.997")],
                ),
                logical(
                    3840,
                    0,
                    1.0,
                    false,
                    vec![assignment("DP-8", "3840x2160@59.997")],
                ),
            ],
        };
        let problems = validate(&monitors, &layout);
        assert!(
            problems
                .iter()
                .any(|p| matches!(p, LayoutProblem::FractionalScaleInPhysicalMode { .. }))
        );
    }

    #[test]
    fn leased_monitors_are_rejected() {
        let mut monitors = monitors3();
        monitors[0].is_for_lease = true;
        let layout = DisplayLayout {
            layout_mode: LayoutMode::Logical,
            logical_displays: vec![
                logical(
                    0,
                    0,
                    2.0,
                    true,
                    vec![assignment("DP-7", "3840x2160@59.997")],
                ),
                logical(
                    1920,
                    0,
                    2.0,
                    false,
                    vec![assignment("DP-8", "3840x2160@59.997")],
                ),
            ],
        };
        let problems = validate(&monitors, &layout);
        assert!(problems.contains(&LayoutProblem::MonitorForLease {
            connector: "DP-7".into()
        }));
    }
}
