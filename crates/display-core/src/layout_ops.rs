//! Higher-level layout construction: building the partial-mirror topology.

use thiserror::Error;

use crate::layout::{DisplayLayout, LogicalDisplay, MonitorAssignment};
use crate::mirror::{self, MirrorError};
use crate::state::DisplayState;
use crate::validation;

/// Where the mirror group is placed relative to the remaining displays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Above,
    Below,
}

#[derive(Debug, Error)]
pub enum MirrorLayoutError {
    #[error("monitor {0} is not connected")]
    UnknownMonitor(String),
    #[error(transparent)]
    Mirror(#[from] MirrorError),
    #[error("the mirror group and the remaining displays could not be sized")]
    Unsizable,
    #[error("the selected displays share no common scale for the mirrored resolution")]
    NoCommonScale,
    #[error("the edit would produce an invalid layout: {0}")]
    WouldBeInvalid(String),
}

/// Builds a layout where `members` form one mirror group placed on `side` of
/// the remaining enabled displays. Non-member displays keep their current
/// mode, scale, and transform, and their current left-to-right order.
///
/// The mirror group uses the best common resolution (largest area, then
/// highest minimum refresh). The primary flag stays with the logical display
/// that contains the previously primary monitor; if that monitor joined the
/// mirror group, the group becomes primary.
pub fn build_mirror_layout(
    state: &DisplayState,
    members: &[String],
    side: Side,
) -> Result<DisplayLayout, MirrorLayoutError> {
    let member_monitors: Vec<&crate::PhysicalMonitor> = members
        .iter()
        .map(|c| {
            state
                .monitor(c)
                .ok_or_else(|| MirrorLayoutError::UnknownMonitor(c.clone()))
        })
        .collect::<Result<_, _>>()?;

    let candidates = mirror::mirror_candidates(&member_monitors)?;
    let candidate = mirror::recommended(&candidates).ok_or(MirrorLayoutError::Unsizable)?;

    // Scale for the group: keep the current scale of the first member's
    // logical display when compatible, else the preferred scale of the chosen
    // mode when compatible, else the largest common scale.
    let current_scale = members
        .first()
        .and_then(|c| state.layout.group_of(c))
        .map(|(_, logical)| logical.scale);
    let preferred_scale = member_monitors
        .first()
        .and_then(|m| m.find_mode(&candidate.members[0].mode_id))
        .map(|mode| mode.preferred_scale);
    let scale = [current_scale, preferred_scale]
        .into_iter()
        .flatten()
        .find(|s| {
            candidate
                .common_scales
                .iter()
                .any(|c| crate::approx_eq(*c, *s, crate::SCALE_EPS))
        })
        .or_else(|| {
            candidate
                .common_scales
                .iter()
                .copied()
                .filter(|s| *s <= 2.0 + crate::SCALE_EPS)
                .fold(None, |acc: Option<f64>, s| {
                    Some(acc.map_or(s, |a| a.max(s)))
                })
        })
        .or_else(|| candidate.common_scales.first().copied())
        .unwrap_or(1.0);

    let previously_primary: Option<String> = state
        .layout
        .primary()
        .and_then(|l| l.monitors.first())
        .map(|a| a.connector.clone());

    let mirror_primary = previously_primary
        .as_ref()
        .is_none_or(|primary| members.contains(primary));

    let mut mirror_group = LogicalDisplay {
        x: 0,
        y: 0,
        scale,
        transform: crate::Transform::Normal,
        primary: mirror_primary,
        monitors: candidate
            .members
            .iter()
            .map(|member| {
                let monitor = state.monitor(&member.connector);
                MonitorAssignment {
                    connector: member.connector.clone(),
                    mode_id: member.mode_id.clone(),
                    color_mode: monitor.and_then(|m| m.color_mode),
                    rgb_range: monitor.and_then(|m| m.rgb_range),
                    underscanning: monitor
                        .and_then(|m| m.supports_underscanning.then_some(m.is_underscanning)),
                }
            })
            .collect(),
    };

    // Remaining displays: preserve their current logical configuration and
    // left-to-right order, re-packed into an adjacent row/starting block.
    let mut rest: Vec<LogicalDisplay> = state
        .layout
        .logical_displays
        .iter()
        .filter(|l| !l.monitors.iter().any(|a| members.contains(&a.connector)))
        .cloned()
        .collect();
    rest.sort_by_key(|l| (l.x, l.y));
    let mut cursor_x = 0;
    for logical in &mut rest {
        logical.x = cursor_x;
        logical.y = 0;
        logical.primary = !mirror_primary
            && previously_primary
                .as_ref()
                .is_some_and(|p| logical.contains_connector(p));
        let (w, _) = logical
            .logical_size(&state.monitors, state.layout.layout_mode)
            .ok_or(MirrorLayoutError::Unsizable)?;
        cursor_x += w;
    }

    // Guarantee exactly one primary even if the previous primary vanished.
    if !mirror_group.primary && !rest.iter().any(|l| l.primary) {
        mirror_group.primary = true;
    }

    let rest_width = cursor_x;
    let rest_height = rest
        .iter()
        .filter_map(|l| l.logical_size(&state.monitors, state.layout.layout_mode))
        .map(|(_, h)| h)
        .max()
        .unwrap_or(0);

    let group_size = {
        let (w, h) = crate::layout::scale_size(candidate.width, candidate.height, scale);
        match state.layout.layout_mode {
            crate::layout::LayoutMode::Logical => (w, h),
            crate::layout::LayoutMode::Physical => (candidate.width, candidate.height),
        }
    };

    match side {
        Side::Left => mirror_group.x = -group_size.0,
        Side::Right => mirror_group.x = rest_width,
        Side::Above => mirror_group.y = -group_size.1,
        Side::Below => mirror_group.y = rest_height,
    }

    let mut logical_displays = vec![mirror_group];
    logical_displays.extend(rest);
    let mut layout = DisplayLayout {
        layout_mode: state.layout.layout_mode,
        logical_displays,
    };
    validation::normalize(&mut layout, &state.monitors);
    Ok(layout)
}

/// Merges `joining_connector`'s monitor into the mirror group (or single
/// display) containing `anchor_connector`, picking the best common
/// resolution. Current color/RGB settings of each member are preserved.
pub fn merge_into_mirror(
    state: &DisplayState,
    layout: &mut DisplayLayout,
    anchor_connector: &str,
    joining_connector: &str,
) -> Result<(), MirrorLayoutError> {
    let mut connectors: Vec<String> = layout
        .group_of(anchor_connector)
        .map(|(_, l)| l.monitors.iter().map(|a| a.connector.clone()).collect())
        .unwrap_or_default();
    if connectors.is_empty() {
        return Err(MirrorLayoutError::UnknownMonitor(
            anchor_connector.to_owned(),
        ));
    }
    if !connectors.iter().any(|c| c == joining_connector) {
        connectors.push(joining_connector.to_owned());
    }

    let members: Vec<&crate::PhysicalMonitor> = connectors
        .iter()
        .map(|c| {
            state
                .monitor(c)
                .ok_or_else(|| MirrorLayoutError::UnknownMonitor(c.clone()))
        })
        .collect::<Result<_, _>>()?;
    let candidates = mirror::mirror_candidates(&members)?;
    let candidate = mirror::recommended(&candidates).ok_or(MirrorLayoutError::Unsizable)?;
    if candidate.common_scales.is_empty() {
        return Err(MirrorLayoutError::NoCommonScale);
    }

    // Transactional: edit a clone, validate it completely, then commit.
    let mut work = layout.clone();

    // Remove the joining monitor from wherever it currently lives.
    for logical in &mut work.logical_displays {
        logical
            .monitors
            .retain(|a| a.connector != joining_connector);
    }
    work.logical_displays.retain(|l| !l.monitors.is_empty());

    let Some(target) = work
        .logical_displays
        .iter_mut()
        .find(|l| l.contains_connector(anchor_connector))
    else {
        return Err(MirrorLayoutError::UnknownMonitor(
            anchor_connector.to_owned(),
        ));
    };
    target.monitors = candidate
        .members
        .iter()
        .map(|member| {
            let monitor = state.monitor(&member.connector);
            MonitorAssignment {
                connector: member.connector.clone(),
                mode_id: member.mode_id.clone(),
                color_mode: monitor.and_then(|m| m.color_mode),
                rgb_range: monitor.and_then(|m| m.rgb_range),
                underscanning: monitor
                    .and_then(|m| m.supports_underscanning.then_some(m.is_underscanning)),
            }
        })
        .collect();
    if !candidate
        .common_scales
        .iter()
        .any(|s| crate::approx_eq(*s, target.scale, crate::SCALE_EPS))
    {
        target.scale = candidate
            .common_scales
            .iter()
            .copied()
            .filter(|s| *s <= 2.0 + crate::SCALE_EPS)
            .fold(None, |acc: Option<f64>, s| {
                Some(acc.map_or(s, |a| a.max(s)))
            })
            .or_else(|| candidate.common_scales.first().copied())
            .unwrap_or(1.0);
    }

    // Primary handling that cannot create duplicates: the merged group only
    // inherits primary when the removal left the layout without one (i.e.
    // the joining monitor's old group was primary and vanished).
    if work.primary().is_none()
        && let Some(group) = work
            .logical_displays
            .iter_mut()
            .find(|l| l.contains_connector(anchor_connector))
    {
        group.primary = true;
    }
    validation::normalize(&mut work, &state.monitors);

    // Full validation before committing; never leave the caller with a
    // broken layout.
    let fatal: Vec<String> = validation::validate_state(state, &work)
        .into_iter()
        .filter(|p| !p.is_auto_fixable())
        .map(|p| p.to_string())
        .collect();
    if !fatal.is_empty() {
        return Err(MirrorLayoutError::WouldBeInvalid(fatal.join(" ")));
    }
    *layout = work;
    Ok(())
}

/// Splits `member_connector` out of its mirror group into its own logical
/// display placed at the right edge, at its preferred mode and scale.
pub fn split_from_mirror(
    state: &DisplayState,
    layout: &mut DisplayLayout,
    member_connector: &str,
) -> Result<(), MirrorLayoutError> {
    let Some((index, group)) = layout.group_of(member_connector) else {
        return Err(MirrorLayoutError::UnknownMonitor(
            member_connector.to_owned(),
        ));
    };
    if group.monitors.len() < 2 {
        return Ok(()); // nothing to split
    }
    let monitor = state
        .monitor(member_connector)
        .ok_or_else(|| MirrorLayoutError::UnknownMonitor(member_connector.to_owned()))?;
    let (mode_id, scale) = monitor
        .preferred_mode()
        .or_else(|| monitor.modes.first())
        .map(|m| (m.id.clone(), m.preferred_scale))
        .ok_or(MirrorLayoutError::Unsizable)?;

    layout.logical_displays[index]
        .monitors
        .retain(|a| a.connector != member_connector);
    let right_edge = layout
        .logical_displays
        .iter()
        .filter_map(|l| l.rect(&state.monitors, layout.layout_mode))
        .map(|r| r.right())
        .max()
        .unwrap_or(0);
    layout.logical_displays.push(LogicalDisplay {
        x: right_edge,
        y: 0,
        scale,
        transform: crate::Transform::Normal,
        primary: false,
        monitors: vec![MonitorAssignment {
            connector: member_connector.to_owned(),
            mode_id,
            color_mode: monitor.color_mode,
            rgb_range: monitor.rgb_range,
            underscanning: monitor
                .supports_underscanning
                .then_some(monitor.is_underscanning),
        }],
    });
    validation::normalize(layout, &state.monitors);
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::validation::validate;

    // A three-monitor state mirroring the audited machine, built by hand to
    // avoid a dependency on the backend fixtures.
    fn target_state() -> DisplayState {
        use crate::identity::MonitorIdentity;
        use crate::layout::{LayoutMode, LogicalDisplay};
        use crate::mode::{ExtraProps, MonitorMode};

        fn mk(
            connector: &str,
            serial: &str,
            modes: &[(&str, i32, i32, f64)],
        ) -> crate::PhysicalMonitor {
            crate::PhysicalMonitor {
                identity: MonitorIdentity::new(connector, "GSM", "LG HDR 4K", serial),
                display_name: None,
                modes: modes
                    .iter()
                    .map(|(id, w, h, hz)| MonitorMode {
                        id: (*id).to_owned(),
                        width: *w,
                        height: *h,
                        refresh_hz: *hz,
                        preferred_scale: 2.0,
                        supported_scales: vec![1.0, 1.5, 2.0],
                        is_current: false,
                        is_preferred: false,
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

        let monitors = vec![
            mk("DP-7", "a", &[("3840x2160@59.997", 3840, 2160, 59.997)]),
            mk("DP-8", "b", &[("3840x2160@59.997", 3840, 2160, 59.997)]),
            mk(
                "HDMI-1",
                "c",
                &[
                    ("3840x2160@30.000", 3840, 2160, 30.0),
                    ("1920x1080@60.000", 1920, 1080, 60.0),
                ],
            ),
        ];
        let mk_assign = |c: &str, m: &str| MonitorAssignment {
            connector: c.into(),
            mode_id: m.into(),
            color_mode: None,
            rgb_range: None,
            underscanning: None,
        };
        let mut state = DisplayState {
            serial: 1,
            monitors,
            layout: DisplayLayout {
                layout_mode: LayoutMode::Logical,
                logical_displays: vec![
                    LogicalDisplay {
                        x: 0,
                        y: 0,
                        scale: 2.0,
                        transform: crate::Transform::Normal,
                        primary: false,
                        monitors: vec![mk_assign("DP-8", "3840x2160@59.997")],
                    },
                    LogicalDisplay {
                        x: 1920,
                        y: 0,
                        scale: 2.0,
                        transform: crate::Transform::Normal,
                        primary: true,
                        monitors: vec![mk_assign("DP-7", "3840x2160@59.997")],
                    },
                    LogicalDisplay {
                        x: 3840,
                        y: 0,
                        scale: 2.0,
                        transform: crate::Transform::Normal,
                        primary: false,
                        monitors: vec![mk_assign("HDMI-1", "3840x2160@30.000")],
                    },
                ],
            },
            supports_changing_layout_mode: true,
            global_scale_required: false,
            supports_mirroring: None,
            extra: Default::default(),
        };
        for monitor in &mut state.monitors {
            let current = state
                .layout
                .logical_displays
                .iter()
                .flat_map(|l| &l.monitors)
                .find(|a| a.connector == monitor.identity.connector)
                .map(|a| a.mode_id.clone());
            for mode in &mut monitor.modes {
                mode.is_current = current.as_deref() == Some(mode.id.as_str());
            }
        }
        state
    }

    #[test]
    fn builds_partial_mirror_on_each_side() {
        let state = target_state();
        let members = vec!["DP-7".to_owned(), "HDMI-1".to_owned()];

        for side in [Side::Left, Side::Right, Side::Above, Side::Below] {
            let layout = build_mirror_layout(&state, &members, side).unwrap();
            assert_eq!(layout.logical_displays.len(), 2, "{side:?}");
            let problems = validate(&state.monitors, &layout);
            assert!(problems.is_empty(), "{side:?}: {problems:?}");

            let (_, group) = layout.group_of("DP-7").unwrap();
            assert!(group.is_mirror_group());
            assert!(group.contains_connector("HDMI-1"));
            // DP-7 keeps 4K@59.997, the KVM uses 4K@30.
            assert!(
                group
                    .monitors
                    .iter()
                    .any(|a| a.mode_id == "3840x2160@59.997")
            );
            assert!(
                group
                    .monitors
                    .iter()
                    .any(|a| a.mode_id == "3840x2160@30.000")
            );
            // Previous primary (DP-7) is in the group, so the group is primary.
            assert!(group.primary);

            let (_, rest) = layout.group_of("DP-8").unwrap();
            let group_rect = group.rect(&state.monitors, layout.layout_mode).unwrap();
            let rest_rect = rest.rect(&state.monitors, layout.layout_mode).unwrap();
            match side {
                Side::Left => assert!(group_rect.x < rest_rect.x),
                Side::Right => assert!(group_rect.x > rest_rect.x),
                Side::Above => assert!(group_rect.y < rest_rect.y),
                Side::Below => assert!(group_rect.y > rest_rect.y),
            }
        }
    }

    #[test]
    fn primary_stays_with_independent_display() {
        let mut state = target_state();
        // Make DP-8 primary instead.
        state.layout.logical_displays[0].primary = true;
        state.layout.logical_displays[1].primary = false;

        let members = vec!["DP-7".to_owned(), "HDMI-1".to_owned()];
        let layout = build_mirror_layout(&state, &members, Side::Right).unwrap();
        let (_, group) = layout.group_of("DP-7").unwrap();
        let (_, rest) = layout.group_of("DP-8").unwrap();
        assert!(!group.primary);
        assert!(rest.primary);
    }

    #[test]
    fn unknown_member_is_reported() {
        let state = target_state();
        let err = build_mirror_layout(&state, &["DP-99".to_owned()], Side::Left).unwrap_err();
        assert!(matches!(err, MirrorLayoutError::UnknownMonitor(c) if c == "DP-99"));
    }

    #[test]
    fn merge_then_split_roundtrip_is_valid() {
        let state = target_state();
        let mut layout = state.layout.clone();

        merge_into_mirror(&state, &mut layout, "DP-7", "HDMI-1").unwrap();
        assert!(validate(&state.monitors, &layout).is_empty());
        assert_eq!(layout.logical_displays.len(), 2);
        let (_, group) = layout.group_of("DP-7").unwrap();
        assert!(group.is_mirror_group());
        assert!(group.contains_connector("HDMI-1"));
        assert!(group.primary, "primary must survive the merge");
        // Best common resolution: 4K, mixed refresh.
        assert!(
            group
                .monitors
                .iter()
                .any(|a| a.mode_id == "3840x2160@59.997")
        );
        assert!(
            group
                .monitors
                .iter()
                .any(|a| a.mode_id == "3840x2160@30.000")
        );

        split_from_mirror(&state, &mut layout, "HDMI-1").unwrap();
        assert!(validate(&state.monitors, &layout).is_empty());
        assert_eq!(layout.logical_displays.len(), 3);
        let (_, solo) = layout.group_of("HDMI-1").unwrap();
        assert!(!solo.is_mirror_group());
    }

    #[test]
    fn merge_from_middle_keeps_layout_adjacent() {
        // Mirror the two outer displays (DP-8 at x=0, HDMI-1 at x=3840),
        // leaving DP-7 from the middle as the independent one.
        let state = target_state();
        let mut layout = state.layout.clone();
        merge_into_mirror(&state, &mut layout, "DP-8", "HDMI-1").unwrap();
        let problems = validate(&state.monitors, &layout);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(layout.logical_displays.len(), 2);
    }

    #[test]
    fn merge_with_unknown_monitor_fails_cleanly() {
        let state = target_state();
        let mut layout = state.layout.clone();
        let before = layout.clone();
        assert!(merge_into_mirror(&state, &mut layout, "DP-7", "DP-99").is_err());
        assert_eq!(layout, before, "failed merge must not modify the layout");
    }

    #[test]
    fn moving_member_out_of_primary_group_keeps_single_primary() {
        let state = target_state();
        let mut layout = state.layout.clone();
        // Build: [DP-7 + DP-8] primary mirror group, [HDMI-1] separate.
        merge_into_mirror(&state, &mut layout, "DP-7", "DP-8").unwrap();
        assert!(layout.group_of("DP-7").unwrap().1.primary);
        // Now pull DP-8 out of the (still populated) primary group into a
        // mirror with HDMI-1. The old group keeps primary; no duplicates.
        merge_into_mirror(&state, &mut layout, "HDMI-1", "DP-8").unwrap();
        let primaries = layout.logical_displays.iter().filter(|l| l.primary).count();
        assert_eq!(primaries, 1, "exactly one primary after cross-group move");
        assert!(layout.group_of("DP-7").unwrap().1.primary);
        assert!(validate(&state.monitors, &layout).is_empty());
    }
}
