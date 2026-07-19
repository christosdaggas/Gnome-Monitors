//! Conversion from `display-core` layouts to `ApplyMonitorsConfig` payloads.

use std::collections::HashMap;

use display_core::layout::{DisplayLayout, LayoutMode};
use zvariant::{OwnedValue, Value};

use crate::error::BackendError;
use crate::proxy::{WireLogicalMonitor, WireMonitorAssignment};

fn own<'a>(value: impl Into<Value<'a>>) -> Result<OwnedValue, BackendError> {
    value
        .into()
        .try_to_owned()
        .map_err(|e| BackendError::Encode(e.to_string()))
}

/// Builds the `a(iiduba(ssa{sv}))` argument.
pub fn wire_logical_monitors(
    layout: &DisplayLayout,
) -> Result<Vec<WireLogicalMonitor>, BackendError> {
    let mut result = Vec::with_capacity(layout.logical_displays.len());
    for logical in &layout.logical_displays {
        let mut monitors: Vec<WireMonitorAssignment> = Vec::with_capacity(logical.monitors.len());
        for assignment in &logical.monitors {
            let mut props: HashMap<String, OwnedValue> = HashMap::new();
            if let Some(color_mode) = assignment.color_mode {
                props.insert("color-mode".into(), own(color_mode.as_u32())?);
            }
            if let Some(rgb_range) = assignment.rgb_range {
                props.insert("rgb-range".into(), own(rgb_range.as_u32())?);
            }
            if let Some(underscanning) = assignment.underscanning {
                props.insert("underscanning".into(), own(underscanning)?);
            }
            monitors.push((
                assignment.connector.clone(),
                assignment.mode_id.clone(),
                props,
            ));
        }
        result.push((
            logical.x,
            logical.y,
            logical.scale,
            logical.transform.as_u32(),
            logical.primary,
            monitors,
        ));
    }
    Ok(result)
}

/// Builds the top-level `a{sv}` properties argument.
///
/// `layout-mode` is only sent when the server advertises
/// `supports-changing-layout-mode` (matching gdctl and GNOME Settings).
pub fn wire_properties(
    layout_mode: LayoutMode,
    supports_changing_layout_mode: bool,
) -> Result<HashMap<String, OwnedValue>, BackendError> {
    let mut props = HashMap::new();
    if supports_changing_layout_mode {
        props.insert("layout-mode".to_owned(), own(layout_mode.as_u32())?);
    }
    Ok(props)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use display_core::Transform;
    use display_core::layout::{LogicalDisplay, MonitorAssignment};
    use display_core::monitor::{ColorMode, RgbRange};

    use super::*;

    #[test]
    fn serializes_partial_mirror_layout() {
        let layout = DisplayLayout {
            layout_mode: LayoutMode::Logical,
            logical_displays: vec![
                LogicalDisplay {
                    x: 0,
                    y: 0,
                    scale: 2.0,
                    transform: Transform::Normal,
                    primary: true,
                    monitors: vec![
                        MonitorAssignment {
                            connector: "DP-7".into(),
                            mode_id: "3840x2160@59.997".into(),
                            color_mode: Some(ColorMode::Default),
                            rgb_range: Some(RgbRange::Auto),
                            underscanning: None,
                        },
                        MonitorAssignment {
                            connector: "HDMI-1".into(),
                            mode_id: "3840x2160@30.000".into(),
                            color_mode: None,
                            rgb_range: None,
                            underscanning: Some(false),
                        },
                    ],
                },
                LogicalDisplay {
                    x: 1920,
                    y: 0,
                    scale: 2.0,
                    transform: Transform::Rotate270,
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
        };

        let wire = wire_logical_monitors(&layout).unwrap();
        assert_eq!(wire.len(), 2);
        let (x, y, scale, transform, primary, monitors) = &wire[0];
        assert_eq!((*x, *y, *scale, *transform, *primary), (0, 0, 2.0, 0, true));
        assert_eq!(monitors.len(), 2);
        assert_eq!(monitors[0].0, "DP-7");
        assert_eq!(monitors[0].1, "3840x2160@59.997");
        assert_eq!(
            monitors[0]
                .2
                .get("color-mode")
                .unwrap()
                .downcast_ref::<u32>()
                .unwrap(),
            0
        );
        assert_eq!(
            monitors[0]
                .2
                .get("rgb-range")
                .unwrap()
                .downcast_ref::<u32>()
                .unwrap(),
            1
        );
        assert!(!monitors[0].2.contains_key("underscanning"));
        assert!(
            !monitors[1]
                .2
                .get("underscanning")
                .unwrap()
                .downcast_ref::<bool>()
                .unwrap()
        );
        assert_eq!(wire[1].3, 3); // Rotate270 wire value

        let props = wire_properties(LayoutMode::Logical, true).unwrap();
        assert_eq!(
            props
                .get("layout-mode")
                .unwrap()
                .downcast_ref::<u32>()
                .unwrap(),
            1
        );
        assert!(
            wire_properties(LayoutMode::Logical, false)
                .unwrap()
                .is_empty()
        );
    }
}
