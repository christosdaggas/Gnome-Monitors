//! Test fixtures: representative monitor topologies.
//!
//! Modeled on the audited machine (two identical LG 27" 4K panels + a
//! Lontium-based KVM that caps 4K at 30 Hz) plus synthetic variations. Used
//! by the mock backend, unit tests, and integration tests.

use display_core::layout::{DisplayLayout, LayoutMode, LogicalDisplay, MonitorAssignment};
use display_core::mode::ExtraProps;
use display_core::monitor::{ColorMode, RgbRange};
use display_core::{DisplayState, MonitorIdentity, MonitorMode, PhysicalMonitor, Transform};

pub fn mode(id: &str, w: i32, h: i32, hz: f64, scales: &[f64]) -> MonitorMode {
    MonitorMode {
        id: id.into(),
        width: w,
        height: h,
        refresh_hz: hz,
        preferred_scale: scales.first().copied().unwrap_or(1.0),
        supported_scales: scales.to_vec(),
        is_current: false,
        is_preferred: false,
        is_interlaced: false,
        refresh_rate_mode: None,
        extra: ExtraProps::new(),
    }
}

pub fn monitor(
    connector: &str,
    vendor: &str,
    product: &str,
    serial: &str,
    display_name: &str,
    modes: Vec<MonitorMode>,
) -> PhysicalMonitor {
    let mut monitor = PhysicalMonitor {
        identity: MonitorIdentity::new(connector, vendor, product, serial),
        display_name: (!display_name.is_empty()).then(|| display_name.to_owned()),
        modes,
        is_builtin: false,
        is_underscanning: false,
        supports_underscanning: false,
        is_for_lease: false,
        color_mode: Some(ColorMode::Default),
        supported_color_modes: vec![ColorMode::Default, ColorMode::SdrNative],
        rgb_range: Some(RgbRange::Auto),
        min_refresh_rate: None,
        physical_size_mm: None,
        extra: ExtraProps::new(),
    };
    if let Some(first) = monitor.modes.first_mut() {
        first.is_preferred = true;
    }
    monitor
}

/// An LG-27-4K-like panel (matches the audited hardware).
pub fn lg27(connector: &str, serial: &str) -> PhysicalMonitor {
    let scales_4k: &[f64] = &[2.0, 1.0, 1.25, 1.5];
    let mut m = monitor(
        connector,
        "GSM",
        "LG HDR 4K",
        serial,
        "LG Electronics 27\"",
        vec![
            mode("3840x2160@59.997", 3840, 2160, 59.996_624, scales_4k),
            mode("3840x2160@30.000", 3840, 2160, 30.0, scales_4k),
            mode("2560x1440@59.951", 2560, 1440, 59.950_55, &[1.0, 1.25, 1.5]),
            mode("1920x1080@60.000", 1920, 1080, 60.0, &[1.0, 1.25]),
            mode("1920x1080@59.940", 1920, 1080, 59.94, &[1.0, 1.25]),
        ],
    );
    m.supported_color_modes = vec![ColorMode::Default, ColorMode::SdrNative, ColorMode::Bt2100];
    m.physical_size_mm = Some((600, 340));
    m
}

/// A Lontium-based KVM capture device (4K only at ≤ 30 Hz).
pub fn kvm(connector: &str) -> PhysicalMonitor {
    let mut m = monitor(
        connector,
        "LTM",
        "Lontium semi",
        "0x88888800",
        "LTM 5\"",
        vec![
            mode("3840x2160@30.000", 3840, 2160, 30.0, &[2.0, 1.0, 1.25, 1.5]),
            mode("2560x1440@59.951", 2560, 1440, 59.950_55, &[1.0, 1.25, 1.5]),
            mode("1920x1080@60.000", 1920, 1080, 60.0, &[1.0, 1.25]),
            mode("1920x1080@50.000", 1920, 1080, 50.0, &[1.0, 1.25]),
        ],
    );
    m.min_refresh_rate = Some(24);
    m
}

pub fn assignment(connector: &str, mode_id: &str) -> MonitorAssignment {
    MonitorAssignment {
        connector: connector.into(),
        mode_id: mode_id.into(),
        color_mode: None,
        rgb_range: None,
        underscanning: None,
    }
}

pub fn logical(
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

/// Assembles a state, marking each assigned mode `is_current`.
pub fn state(serial: u32, monitors: Vec<PhysicalMonitor>, layout: DisplayLayout) -> DisplayState {
    let mut state = DisplayState {
        serial,
        monitors,
        layout,
        supports_changing_layout_mode: true,
        global_scale_required: false,
        supports_mirroring: None,
        extra: ExtraProps::new(),
    };
    let assignments: Vec<(String, String)> = state
        .layout
        .logical_displays
        .iter()
        .flat_map(|l| {
            l.monitors
                .iter()
                .map(|a| (a.connector.clone(), a.mode_id.clone()))
        })
        .collect();
    for monitor in &mut state.monitors {
        for mode in &mut monitor.modes {
            mode.is_current = assignments
                .iter()
                .any(|(c, id)| c == monitor.identity.connector.as_str() && id == &mode.id);
        }
    }
    state
}

/// Named fixture topologies for the test matrix.
pub fn fixture(name: &str) -> DisplayState {
    match name {
        "single" => state(
            1,
            vec![lg27("DP-1", "0x0001")],
            DisplayLayout {
                layout_mode: LayoutMode::Logical,
                logical_displays: vec![logical(
                    0,
                    0,
                    2.0,
                    true,
                    vec![assignment("DP-1", "3840x2160@59.997")],
                )],
            },
        ),
        "dual-extended" => state(
            1,
            vec![lg27("DP-1", "0x0001"), lg27("DP-2", "0x0002")],
            DisplayLayout {
                layout_mode: LayoutMode::Logical,
                logical_displays: vec![
                    logical(
                        0,
                        0,
                        2.0,
                        true,
                        vec![assignment("DP-1", "3840x2160@59.997")],
                    ),
                    logical(
                        1920,
                        0,
                        2.0,
                        false,
                        vec![assignment("DP-2", "3840x2160@59.997")],
                    ),
                ],
            },
        ),
        "dual-mirrored" => state(
            1,
            vec![lg27("DP-1", "0x0001"), lg27("DP-2", "0x0002")],
            DisplayLayout {
                layout_mode: LayoutMode::Logical,
                logical_displays: vec![logical(
                    0,
                    0,
                    2.0,
                    true,
                    vec![
                        assignment("DP-1", "3840x2160@59.997"),
                        assignment("DP-2", "3840x2160@59.997"),
                    ],
                )],
            },
        ),
        // The target topology of this project: KVM + LG mirrored, LG extended.
        "mirror-plus-extended" => state(
            1,
            vec![
                lg27("DP-7", "0x0004ee0e"),
                lg27("DP-8", "0x0003e924"),
                kvm("HDMI-1"),
            ],
            DisplayLayout {
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
            },
        ),
        "triple-extended" => state(
            1,
            vec![
                lg27("DP-7", "0x0004ee0e"),
                lg27("DP-8", "0x0003e924"),
                kvm("HDMI-1"),
            ],
            DisplayLayout {
                layout_mode: LayoutMode::Logical,
                logical_displays: vec![
                    logical(
                        0,
                        0,
                        2.0,
                        false,
                        vec![assignment("DP-8", "3840x2160@59.997")],
                    ),
                    logical(
                        1920,
                        0,
                        2.0,
                        true,
                        vec![assignment("DP-7", "3840x2160@59.997")],
                    ),
                    logical(
                        3840,
                        0,
                        2.0,
                        false,
                        vec![assignment("HDMI-1", "3840x2160@30.000")],
                    ),
                ],
            },
        ),
        "kvm-no-serial" => {
            let mut k = kvm("HDMI-1");
            k.identity.serial = String::new();
            state(
                1,
                vec![lg27("DP-1", "0x0001"), k],
                DisplayLayout {
                    layout_mode: LayoutMode::Logical,
                    logical_displays: vec![
                        logical(
                            0,
                            0,
                            2.0,
                            true,
                            vec![assignment("DP-1", "3840x2160@59.997")],
                        ),
                        logical(
                            1920,
                            0,
                            1.0,
                            false,
                            vec![assignment("HDMI-1", "1920x1080@60.000")],
                        ),
                    ],
                },
            )
        }
        "identical-twins" => {
            // Two monitors with byte-identical EDID identity.
            let mut a = lg27("DP-1", "0xsame");
            let mut b = lg27("DP-2", "0xsame");
            a.identity.product = "Twin".into();
            b.identity.product = "Twin".into();
            state(
                1,
                vec![a, b],
                DisplayLayout {
                    layout_mode: LayoutMode::Logical,
                    logical_displays: vec![
                        logical(
                            0,
                            0,
                            2.0,
                            true,
                            vec![assignment("DP-1", "3840x2160@59.997")],
                        ),
                        logical(
                            1920,
                            0,
                            2.0,
                            false,
                            vec![assignment("DP-2", "3840x2160@59.997")],
                        ),
                    ],
                },
            )
        }
        "mixed-1080p-4k" => state(
            1,
            vec![
                lg27("DP-1", "0x0001"),
                monitor(
                    "HDMI-1",
                    "AUS",
                    "ROG PG248Q",
                    "L8LMQS075392",
                    "ASUS 24\"",
                    vec![
                        mode("1920x1080@144.001", 1920, 1080, 144.001, &[1.0, 1.25]),
                        mode("1920x1080@60.000", 1920, 1080, 60.0, &[1.0, 1.25]),
                    ],
                ),
            ],
            DisplayLayout {
                layout_mode: LayoutMode::Logical,
                logical_displays: vec![
                    logical(
                        0,
                        0,
                        2.0,
                        true,
                        vec![assignment("DP-1", "3840x2160@59.997")],
                    ),
                    logical(
                        1920,
                        0,
                        1.0,
                        false,
                        vec![assignment("HDMI-1", "1920x1080@144.001")],
                    ),
                ],
            },
        ),
        "mixed-refresh-rates" => state(
            1,
            vec![
                monitor(
                    "DP-1",
                    "AAA",
                    "Sixty",
                    "1",
                    "",
                    vec![
                        mode("1920x1080@60.000", 1920, 1080, 60.0, &[1.0]),
                        mode("1920x1080@59.940", 1920, 1080, 59.94, &[1.0]),
                        mode("1920x1080@59.950", 1920, 1080, 59.95, &[1.0]),
                    ],
                ),
                monitor(
                    "DP-2",
                    "BBB",
                    "Fifty",
                    "2",
                    "",
                    vec![
                        mode("1920x1080@50.000", 1920, 1080, 50.0, &[1.0]),
                        mode("1920x1080@59.940", 1920, 1080, 59.94, &[1.0]),
                    ],
                ),
            ],
            DisplayLayout {
                layout_mode: LayoutMode::Logical,
                logical_displays: vec![
                    logical(
                        0,
                        0,
                        1.0,
                        true,
                        vec![assignment("DP-1", "1920x1080@60.000")],
                    ),
                    logical(
                        1920,
                        0,
                        1.0,
                        false,
                        vec![assignment("DP-2", "1920x1080@50.000")],
                    ),
                ],
            },
        ),
        "fractional-scaling" => state(
            1,
            vec![lg27("DP-1", "0x0001"), lg27("DP-2", "0x0002")],
            DisplayLayout {
                layout_mode: LayoutMode::Logical,
                logical_displays: vec![
                    logical(
                        0,
                        0,
                        1.5,
                        true,
                        vec![assignment("DP-1", "3840x2160@59.997")],
                    ),
                    logical(
                        2560,
                        0,
                        1.25,
                        false,
                        vec![assignment("DP-2", "3840x2160@59.997")],
                    ),
                ],
            },
        ),
        "rotated" => {
            let mut layout = DisplayLayout {
                layout_mode: LayoutMode::Logical,
                logical_displays: vec![
                    logical(
                        0,
                        0,
                        2.0,
                        true,
                        vec![assignment("DP-1", "3840x2160@59.997")],
                    ),
                    logical(
                        1920,
                        0,
                        2.0,
                        false,
                        vec![assignment("DP-2", "3840x2160@59.997")],
                    ),
                ],
            };
            layout.logical_displays[1].transform = Transform::Rotate90;
            state(
                1,
                vec![lg27("DP-1", "0x0001"), lg27("DP-2", "0x0002")],
                layout,
            )
        }
        "unknown-properties" => {
            let mut s = fixture("dual-extended");
            s.extra
                .insert("hypothetical-51-property".into(), "true".into());
            s.monitors[0]
                .extra
                .insert("quantum-flux".into(), "0.7".into());
            s.monitors[0].modes[0]
                .extra
                .insert("is-hyperspace".into(), "false".into());
            s
        }
        "no-common-mirror-mode" => state(
            1,
            vec![
                monitor(
                    "DP-1",
                    "AAA",
                    "WQHD Only",
                    "1",
                    "",
                    vec![mode("2560x1440@60.000", 2560, 1440, 60.0, &[1.0])],
                ),
                monitor(
                    "DP-2",
                    "BBB",
                    "WUXGA Only",
                    "2",
                    "",
                    vec![mode("1920x1200@60.000", 1920, 1200, 60.0, &[1.0])],
                ),
            ],
            DisplayLayout {
                layout_mode: LayoutMode::Logical,
                logical_displays: vec![
                    logical(
                        0,
                        0,
                        1.0,
                        true,
                        vec![assignment("DP-1", "2560x1440@60.000")],
                    ),
                    logical(
                        2560,
                        0,
                        1.0,
                        false,
                        vec![assignment("DP-2", "1920x1200@60.000")],
                    ),
                ],
            },
        ),
        other => panic!("unknown fixture {other:?}"),
    }
}

/// All fixture names, for exhaustive tests.
pub const FIXTURES: &[&str] = &[
    "single",
    "dual-extended",
    "dual-mirrored",
    "mirror-plus-extended",
    "triple-extended",
    "kvm-no-serial",
    "identical-twins",
    "mixed-1080p-4k",
    "mixed-refresh-rates",
    "fractional-scaling",
    "rotated",
    "unknown-properties",
    "no-common-mirror-mode",
];

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use display_core::validation::validate;

    use super::*;

    #[test]
    fn all_fixtures_are_valid_layouts() {
        for name in FIXTURES {
            let s = fixture(name);
            let problems = validate(&s.monitors, &s.layout);
            assert!(problems.is_empty(), "fixture {name}: {problems:?}");
        }
    }

    #[test]
    fn fixtures_roundtrip_through_json() {
        for name in FIXTURES {
            let s = fixture(name);
            let json = serde_json::to_string(&s).unwrap();
            let back: DisplayState = serde_json::from_str(&json).unwrap();
            assert_eq!(s, back, "fixture {name}");
        }
    }

    #[test]
    fn kvm_fixture_is_detected() {
        let s = fixture("mirror-plus-extended");
        let kvm_monitor = s.monitor("HDMI-1").unwrap();
        assert!(kvm_monitor.is_likely_kvm());
        assert!(!s.monitor("DP-7").unwrap().is_likely_kvm());
    }
}
