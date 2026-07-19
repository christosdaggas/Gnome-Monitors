//! Mirror-group compatibility calculation.
//!
//! Mirrored monitors live in one logical display, so all members must use
//! modes with the *same resolution* (verified against Mutter 50.2 — refresh
//! rates may differ between members; e.g. an LG panel at 59.997 Hz mirrored
//! with a KVM capture device at 30.000 Hz is accepted).

use thiserror::Error;

use crate::approx_eq;
use crate::mode::MonitorMode;
use crate::monitor::PhysicalMonitor;

/// The mode chosen for one member of a mirror candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct MirrorMemberMode {
    pub connector: String,
    pub mode_id: String,
    pub refresh_hz: f64,
}

/// One resolution every member can produce, with a per-member mode choice.
#[derive(Debug, Clone, PartialEq)]
pub struct MirrorCandidate {
    pub width: i32,
    pub height: i32,
    /// Best (highest-refresh) mode per member at this resolution.
    pub members: Vec<MirrorMemberMode>,
    /// Scales supported by *all* chosen modes.
    pub common_scales: Vec<f64>,
    /// The slowest member's refresh rate — the practical "feel" of the group.
    pub min_refresh_hz: f64,
}

impl MirrorCandidate {
    pub fn area(&self) -> i64 {
        i64::from(self.width) * i64::from(self.height)
    }
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum MirrorError {
    #[error("A mirror group needs at least two monitors.")]
    TooFewMonitors,
    #[error("{0}")]
    NoCommonResolution(NoCommonResolution),
}

/// Details for explaining *why* no mirror is possible.
#[derive(Debug, Clone, PartialEq)]
pub struct NoCommonResolution {
    /// (connector, native resolution) per member.
    pub natives: Vec<(String, (i32, i32))>,
}

impl std::fmt::Display for NoCommonResolution {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "The selected displays do not share a compatible mirror resolution ("
        )?;
        for (i, (connector, (w, h))) in self.natives.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{connector}: {w}×{h}")?;
        }
        write!(f, ").")
    }
}

/// All resolutions the given monitors can mirror at, largest area first,
/// ties broken by higher minimum refresh.
pub fn mirror_candidates(
    members: &[&PhysicalMonitor],
) -> Result<Vec<MirrorCandidate>, MirrorError> {
    if members.len() < 2 {
        return Err(MirrorError::TooFewMonitors);
    }

    // Intersect resolution sets.
    let mut common: Vec<(i32, i32)> = members[0].resolutions();
    for m in &members[1..] {
        let theirs = m.resolutions();
        common.retain(|r| theirs.contains(r));
    }

    if common.is_empty() {
        return Err(MirrorError::NoCommonResolution(NoCommonResolution {
            natives: members
                .iter()
                .map(|m| {
                    let native = m
                        .preferred_mode()
                        .or_else(|| m.modes.first())
                        .map_or((0, 0), |mode| (mode.width, mode.height));
                    (m.connector().to_owned(), native)
                })
                .collect(),
        }));
    }

    let mut candidates: Vec<MirrorCandidate> = common
        .into_iter()
        .filter_map(|(w, h)| {
            let mut chosen: Vec<(&PhysicalMonitor, &MonitorMode)> = Vec::new();
            for m in members {
                chosen.push((m, m.best_mode_at(w, h)?));
            }
            let common_scales = intersect_scales(chosen.iter().map(|(_, mode)| *mode));
            let min_refresh = chosen
                .iter()
                .map(|(_, mode)| mode.refresh_hz)
                .fold(f64::INFINITY, f64::min);
            Some(MirrorCandidate {
                width: w,
                height: h,
                members: chosen
                    .iter()
                    .map(|(m, mode)| MirrorMemberMode {
                        connector: m.connector().to_owned(),
                        mode_id: mode.id.clone(),
                        refresh_hz: mode.refresh_hz,
                    })
                    .collect(),
                common_scales,
                min_refresh_hz: min_refresh,
            })
        })
        .collect();

    candidates.sort_by(|a, b| {
        b.area().cmp(&a.area()).then(
            b.min_refresh_hz
                .partial_cmp(&a.min_refresh_hz)
                .unwrap_or(std::cmp::Ordering::Equal),
        )
    });
    Ok(candidates)
}

/// Recommended candidate: the largest resolution — unless a slightly smaller
/// one offers a materially smoother minimum refresh (≥ 50 Hz vs < 50 Hz), in
/// which case both are worth surfacing; the caller shows the recommendation
/// and the alternatives. This function only ranks; it never hides options.
pub fn recommended(candidates: &[MirrorCandidate]) -> Option<&MirrorCandidate> {
    candidates.first()
}

fn intersect_scales<'a>(modes: impl Iterator<Item = &'a MonitorMode>) -> Vec<f64> {
    let mut iter = modes;
    let Some(first) = iter.next() else {
        return Vec::new();
    };
    let mut scales = first.supported_scales.clone();
    for mode in iter {
        scales.retain(|s| {
            mode.supported_scales
                .iter()
                .any(|o| approx_eq(*o, *s, crate::SCALE_EPS))
        });
    }
    scales
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::identity::MonitorIdentity;
    use crate::mode::ExtraProps;

    fn monitor(connector: &str, modes: &[(i32, i32, f64, &[f64])]) -> PhysicalMonitor {
        PhysicalMonitor {
            identity: MonitorIdentity::new(connector, "TST", "Test", connector),
            display_name: None,
            modes: modes
                .iter()
                .enumerate()
                .map(|(i, (w, h, hz, scales))| MonitorMode {
                    id: format!("{w}x{h}@{hz:.3}"),
                    width: *w,
                    height: *h,
                    refresh_hz: *hz,
                    preferred_scale: 1.0,
                    supported_scales: scales.to_vec(),
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

    #[test]
    fn kvm_and_lg_mirror_at_4k_with_mixed_refresh() {
        let lg = monitor(
            "DP-7",
            &[
                (3840, 2160, 59.997, &[1.0, 1.5, 2.0]),
                (3840, 2160, 30.0, &[1.0, 1.5, 2.0]),
                (1920, 1080, 60.0, &[1.0]),
            ],
        );
        let kvm = monitor(
            "HDMI-1",
            &[
                (3840, 2160, 30.0, &[1.0, 1.5, 2.0]),
                (1920, 1080, 60.0, &[1.0, 1.25]),
            ],
        );
        let candidates = mirror_candidates(&[&lg, &kvm]).unwrap();
        assert_eq!(candidates.len(), 2);
        let four_k = &candidates[0];
        assert_eq!((four_k.width, four_k.height), (3840, 2160));
        // LG keeps its 59.997 mode, KVM uses 30.0: different refresh rates.
        assert!(approx_eq(four_k.members[0].refresh_hz, 59.997, 1e-9));
        assert!(approx_eq(four_k.members[1].refresh_hz, 30.0, 1e-9));
        assert!(approx_eq(four_k.min_refresh_hz, 30.0, 1e-9));
        assert_eq!(four_k.common_scales, vec![1.0, 1.5, 2.0]);
        assert_eq!(recommended(&candidates).unwrap().width, 3840);
    }

    #[test]
    fn no_common_resolution_is_explained() {
        let a = monitor("DP-1", &[(2560, 1440, 60.0, &[1.0])]);
        let b = monitor("DP-2", &[(1920, 1200, 60.0, &[1.0])]);
        let err = mirror_candidates(&[&a, &b]).unwrap_err();
        match err {
            MirrorError::NoCommonResolution(detail) => {
                assert_eq!(detail.natives.len(), 2);
                let msg = detail.to_string();
                assert!(msg.contains("DP-1: 2560×1440"));
                assert!(msg.contains("DP-2: 1920×1200"));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn identical_resolution_text_but_disjoint_scales() {
        // Same resolution on both, but scale sets only intersect at 1.0.
        let a = monitor("DP-1", &[(2560, 1440, 59.951, &[1.0, 1.5])]);
        let b = monitor("DP-2", &[(2560, 1440, 59.961, &[1.0, 1.25])]);
        let candidates = mirror_candidates(&[&a, &b]).unwrap();
        assert_eq!(candidates[0].common_scales, vec![1.0]);
        // Close but distinct refresh rates are preserved per member.
        assert!(approx_eq(candidates[0].members[0].refresh_hz, 59.951, 1e-9));
        assert!(approx_eq(candidates[0].members[1].refresh_hz, 59.961, 1e-9));
    }

    #[test]
    fn single_member_is_error() {
        let a = monitor("DP-1", &[(2560, 1440, 60.0, &[1.0])]);
        assert_eq!(
            mirror_candidates(&[&a]).unwrap_err(),
            MirrorError::TooFewMonitors
        );
    }

    #[test]
    fn three_way_mirror() {
        let a = monitor(
            "DP-1",
            &[(3840, 2160, 60.0, &[1.0, 2.0]), (1920, 1080, 60.0, &[1.0])],
        );
        let b = monitor("DP-2", &[(1920, 1080, 59.94, &[1.0, 1.25])]);
        let c = monitor("HDMI-1", &[(1920, 1080, 50.0, &[1.0])]);
        let candidates = mirror_candidates(&[&a, &b, &c]).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!((candidates[0].width, candidates[0].height), (1920, 1080));
        assert_eq!(candidates[0].common_scales, vec![1.0]);
        assert!(approx_eq(candidates[0].min_refresh_hz, 50.0, 1e-9));
    }
}
