//! Conversion from raw D-Bus values to `display-core` domain models.
//!
//! Parsing is tolerant by design: every `a{sv}` property is optional, unknown
//! properties are preserved (stringified) in `extra`, and documented
//! "absence means X" defaults are applied. Only structurally impossible data
//! (a logical monitor referencing a monitor with no current mode) is an error.

use std::collections::HashMap;

use display_core::layout::{DisplayLayout, LayoutMode, LogicalDisplay, MonitorAssignment};
use display_core::mode::ExtraProps;
use display_core::monitor::{ColorMode, RgbRange};
use display_core::{DisplayState, MonitorIdentity, MonitorMode, PhysicalMonitor, Transform};
use zvariant::{OwnedValue, Value};

use crate::error::BackendError;
use crate::proxy::{RawLogicalMonitor, RawMode, RawMonitor, RawState};

type Props = HashMap<String, OwnedValue>;

fn prop_bool(props: &Props, key: &str) -> Option<bool> {
    props.get(key).and_then(|v| v.downcast_ref::<bool>().ok())
}

fn prop_i32(props: &Props, key: &str) -> Option<i32> {
    props.get(key).and_then(|v| v.downcast_ref::<i32>().ok())
}

fn prop_u32(props: &Props, key: &str) -> Option<u32> {
    props.get(key).and_then(|v| v.downcast_ref::<u32>().ok())
}

fn prop_string(props: &Props, key: &str) -> Option<String> {
    props
        .get(key)
        .and_then(|v| v.downcast_ref::<&str>().ok())
        .map(ToOwned::to_owned)
}

fn prop_u32_array(props: &Props, key: &str) -> Option<Vec<u32>> {
    let value = props.get(key)?;
    match &**value {
        Value::Array(array) => Some(
            array
                .iter()
                .filter_map(|v| v.downcast_ref::<u32>().ok())
                .collect(),
        ),
        _ => None,
    }
}

/// Stringify unrecognized properties for diagnostics, bounded in size.
fn extras(props: &Props, known: &[&str]) -> ExtraProps {
    let mut extra = ExtraProps::new();
    for (key, value) in props {
        if known.contains(&key.as_str()) {
            continue;
        }
        let mut text = format!("{value:?}");
        if text.len() > 200 {
            text.truncate(200);
            text.push('…');
        }
        extra.insert(key.clone(), text);
    }
    extra
}

const KNOWN_MONITOR_PROPS: &[&str] = &[
    "width-mm",
    "height-mm",
    "is-underscanning",
    "max-screen-size",
    "is-builtin",
    "display-name",
    "min-refresh-rate",
    "is-for-lease",
    "color-mode",
    "supported-color-modes",
    "rgb-range",
];

const KNOWN_MODE_PROPS: &[&str] = &[
    "is-current",
    "is-preferred",
    "is-interlaced",
    "refresh-rate-mode",
];

const KNOWN_STATE_PROPS: &[&str] = &[
    "layout-mode",
    "supports-changing-layout-mode",
    "global-scale-required",
    "supports-mirroring",
];

fn parse_mode(raw: &RawMode) -> MonitorMode {
    let (id, width, height, refresh_hz, preferred_scale, supported_scales, props) = raw;
    MonitorMode {
        id: id.clone(),
        width: *width,
        height: *height,
        refresh_hz: *refresh_hz,
        preferred_scale: *preferred_scale,
        supported_scales: supported_scales.clone(),
        is_current: prop_bool(props, "is-current").unwrap_or(false),
        is_preferred: prop_bool(props, "is-preferred").unwrap_or(false),
        is_interlaced: prop_bool(props, "is-interlaced").unwrap_or(false),
        refresh_rate_mode: prop_string(props, "refresh-rate-mode"),
        extra: extras(props, KNOWN_MODE_PROPS),
    }
}

fn parse_monitor(raw: &RawMonitor) -> PhysicalMonitor {
    let ((connector, vendor, product, serial), modes, props) = raw;
    let width_mm = prop_i32(props, "width-mm");
    let height_mm = prop_i32(props, "height-mm");
    PhysicalMonitor {
        identity: MonitorIdentity::new(connector, vendor, product, serial),
        display_name: prop_string(props, "display-name"),
        modes: modes.iter().map(parse_mode).collect(),
        is_builtin: prop_bool(props, "is-builtin").unwrap_or(false),
        is_underscanning: prop_bool(props, "is-underscanning").unwrap_or(false),
        supports_underscanning: props.contains_key("is-underscanning"),
        is_for_lease: prop_bool(props, "is-for-lease").unwrap_or(false),
        color_mode: prop_u32(props, "color-mode").and_then(ColorMode::from_u32),
        supported_color_modes: prop_u32_array(props, "supported-color-modes")
            .unwrap_or_default()
            .into_iter()
            .filter_map(ColorMode::from_u32)
            .collect(),
        rgb_range: prop_u32(props, "rgb-range").and_then(RgbRange::from_u32),
        min_refresh_rate: prop_i32(props, "min-refresh-rate"),
        physical_size_mm: match (width_mm, height_mm) {
            (Some(w), Some(h)) if w > 0 && h > 0 => Some((w, h)),
            _ => None,
        },
        extra: extras(props, KNOWN_MONITOR_PROPS),
    }
}

fn parse_logical_monitor(
    raw: &RawLogicalMonitor,
    monitors: &[PhysicalMonitor],
) -> Result<LogicalDisplay, BackendError> {
    let (x, y, scale, transform, primary, specs, _props) = raw;
    let transform = Transform::from_u32(*transform).ok_or_else(|| {
        BackendError::Parse(format!(
            "unknown transform value {transform} in logical monitor"
        ))
    })?;
    let mut assignments = Vec::new();
    for (connector, ..) in specs {
        let monitor = monitors
            .iter()
            .find(|m| m.connector() == connector)
            .ok_or_else(|| {
                BackendError::Parse(format!(
                    "logical monitor references unknown monitor {connector}"
                ))
            })?;
        let mode = monitor.current_mode().ok_or_else(|| {
            BackendError::Parse(format!(
                "active monitor {connector} reports no current mode"
            ))
        })?;
        assignments.push(MonitorAssignment {
            connector: connector.clone(),
            mode_id: mode.id.clone(),
            // Carry the monitor's current output settings so that
            // re-applying this layout (e.g. rollback) preserves them.
            color_mode: monitor.color_mode,
            rgb_range: monitor.rgb_range,
            underscanning: monitor
                .supports_underscanning
                .then_some(monitor.is_underscanning),
        });
    }
    Ok(LogicalDisplay {
        x: *x,
        y: *y,
        scale: *scale,
        transform,
        primary: *primary,
        monitors: assignments,
    })
}

/// Parses a `GetCurrentState` reply into a [`DisplayState`].
pub fn parse_state(raw: &RawState) -> Result<DisplayState, BackendError> {
    let (serial, raw_monitors, raw_logical, props) = raw;
    let monitors: Vec<PhysicalMonitor> = raw_monitors.iter().map(parse_monitor).collect();
    let logical_displays = raw_logical
        .iter()
        .map(|l| parse_logical_monitor(l, &monitors))
        .collect::<Result<Vec<_>, _>>()?;

    let layout_mode = prop_u32(props, "layout-mode")
        .and_then(LayoutMode::from_u32)
        .unwrap_or(LayoutMode::Logical);

    Ok(DisplayState {
        serial: *serial,
        monitors,
        layout: DisplayLayout {
            layout_mode,
            logical_displays,
        },
        supports_changing_layout_mode: prop_bool(props, "supports-changing-layout-mode")
            .unwrap_or(false),
        global_scale_required: prop_bool(props, "global-scale-required").unwrap_or(false),
        supports_mirroring: prop_bool(props, "supports-mirroring"),
        extra: extras(props, KNOWN_STATE_PROPS),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn ov<T>(v: T) -> OwnedValue
    where
        T: Into<Value<'static>>,
    {
        v.into().try_to_owned().unwrap()
    }

    fn raw_lg(connector: &str, serial: &str, current: bool) -> RawMonitor {
        let mode_props = |is_current: bool| {
            let mut p = Props::new();
            if is_current {
                p.insert("is-current".into(), ov(true));
                p.insert("is-preferred".into(), ov(true));
            }
            p
        };
        let mut props = Props::new();
        props.insert("display-name".into(), ov("LG Electronics 27\""));
        props.insert("is-builtin".into(), ov(false));
        props.insert("color-mode".into(), ov(0u32));
        props.insert(
            "supported-color-modes".into(),
            Value::from(vec![0u32, 2, 1]).try_to_owned().unwrap(),
        );
        props.insert("rgb-range".into(), ov(1u32));
        // An unknown, future property must be preserved, not fatal.
        props.insert("frobnication-level".into(), ov(9000i32));
        (
            (
                connector.into(),
                "GSM".into(),
                "LG HDR 4K".into(),
                serial.into(),
            ),
            vec![
                (
                    "3840x2160@59.997".into(),
                    3840,
                    2160,
                    59.996_623,
                    2.0,
                    vec![1.0, 1.5, 2.0],
                    mode_props(current),
                ),
                (
                    "1920x1080@60.000".into(),
                    1920,
                    1080,
                    60.0,
                    1.0,
                    vec![1.0],
                    mode_props(false),
                ),
            ],
            props,
        )
    }

    fn raw_state() -> RawState {
        let mut state_props = Props::new();
        state_props.insert("layout-mode".into(), ov(1u32));
        state_props.insert("supports-changing-layout-mode".into(), ov(true));
        state_props.insert("something-new".into(), ov("hello"));
        (
            7,
            vec![
                raw_lg("DP-7", "0x0004ee0e", true),
                raw_lg("DP-8", "0x0003e924", true),
            ],
            vec![
                (
                    0,
                    0,
                    2.0,
                    0,
                    true,
                    vec![(
                        "DP-7".into(),
                        "GSM".into(),
                        "LG HDR 4K".into(),
                        "0x0004ee0e".into(),
                    )],
                    Props::new(),
                ),
                (
                    1920,
                    0,
                    2.0,
                    1,
                    false,
                    vec![(
                        "DP-8".into(),
                        "GSM".into(),
                        "LG HDR 4K".into(),
                        "0x0003e924".into(),
                    )],
                    Props::new(),
                ),
            ],
            state_props,
        )
    }

    #[test]
    fn parses_full_state() {
        let state = parse_state(&raw_state()).unwrap();
        assert_eq!(state.serial, 7);
        assert_eq!(state.monitors.len(), 2);
        assert_eq!(state.layout.logical_displays.len(), 2);
        assert_eq!(state.layout.layout_mode, LayoutMode::Logical);
        assert!(state.supports_changing_layout_mode);
        assert!(!state.global_scale_required);

        let dp7 = state.monitor("DP-7").unwrap();
        assert_eq!(dp7.display_name.as_deref(), Some("LG Electronics 27\""));
        assert_eq!(dp7.supported_color_modes.len(), 3);
        assert!(dp7.supports_hdr());
        assert!(!dp7.supports_underscanning);
        assert_eq!(dp7.current_mode().unwrap().id, "3840x2160@59.997");

        // Logical monitor got the current mode and transform.
        let second = &state.layout.logical_displays[1];
        assert_eq!(second.transform, Transform::Rotate90);
        assert_eq!(second.monitors[0].mode_id, "3840x2160@59.997");
        assert_eq!(second.monitors[0].color_mode, Some(ColorMode::Default));
        assert_eq!(second.monitors[0].underscanning, None);
    }

    #[test]
    fn unknown_properties_are_preserved_not_fatal() {
        let state = parse_state(&raw_state()).unwrap();
        let dp7 = state.monitor("DP-7").unwrap();
        assert!(dp7.extra.contains_key("frobnication-level"));
        assert!(state.extra.contains_key("something-new"));
    }

    #[test]
    fn missing_current_mode_is_an_error() {
        let mut raw = raw_state();
        // Strip the is-current flags from DP-7's modes.
        for mode in &mut raw.1[0].1 {
            mode.6.clear();
        }
        let err = parse_state(&raw).unwrap_err();
        assert!(err.to_string().contains("no current mode"), "{err}");
    }

    #[test]
    fn invalid_transform_is_an_error() {
        let mut raw = raw_state();
        raw.2[0].3 = 42;
        assert!(parse_state(&raw).is_err());
    }
}
