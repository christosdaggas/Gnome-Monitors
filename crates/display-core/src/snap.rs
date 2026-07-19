//! Pure geometry for the layout editor: edge snapping, overlap resolution,
//! and "gravity" that keeps dragged displays attached to the arrangement.
//!
//! All functions operate in layout coordinates; the canvas converts pointer
//! deltas into layout deltas before calling in, so snapping thresholds behave
//! consistently at any zoom level.

use crate::geometry::Rect;

/// Axis of a snap guide line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// A vertical guide at `position` (an x coordinate).
    Vertical,
    /// A horizontal guide at `position` (a y coordinate).
    Horizontal,
}

/// A visual alignment guide produced by snapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapGuide {
    pub axis: Axis,
    pub position: i32,
}

/// Result of snapping a dragged rectangle.
#[derive(Debug, Clone, PartialEq)]
pub struct SnapResult {
    pub x: i32,
    pub y: i32,
    pub guides: Vec<SnapGuide>,
}

/// Snaps `moving` against the edges of `others`.
///
/// Candidates per axis: aligning the moving rect's leading/trailing edge with
/// each other rect's leading/trailing edge (which covers both edge-to-edge
/// attachment and flush alignment). The nearest candidate within `threshold`
/// wins per axis; axes are independent.
pub fn snap_rect(moving: Rect, others: &[Rect], threshold: i32) -> SnapResult {
    let mut best_x: Option<(i32, i32, i32)> = None; // (delta_abs, new_x, guide_pos)
    let mut best_y: Option<(i32, i32, i32)> = None;

    let mut consider_x = |target_x: i32, guide: i32, current: i32| {
        let delta = (current - target_x).abs();
        if delta <= threshold && best_x.is_none_or(|(d, _, _)| delta < d) {
            best_x = Some((delta, target_x, guide));
        }
    };
    let mut consider_y = |target_y: i32, guide: i32, current: i32| {
        let delta = (current - target_y).abs();
        if delta <= threshold && best_y.is_none_or(|(d, _, _)| delta < d) {
            best_y = Some((delta, target_y, guide));
        }
    };

    for other in others {
        // x candidates: left↔left, left↔right, right↔left, right↔right.
        consider_x(other.x, other.x, moving.x);
        consider_x(other.right(), other.right(), moving.x);
        consider_x(other.x - moving.width, other.x, moving.x);
        consider_x(other.right() - moving.width, other.right(), moving.x);
        // y candidates.
        consider_y(other.y, other.y, moving.y);
        consider_y(other.bottom(), other.bottom(), moving.y);
        consider_y(other.y - moving.height, other.y, moving.y);
        consider_y(other.bottom() - moving.height, other.bottom(), moving.y);
    }

    let mut guides = Vec::new();
    let x = if let Some((_, x, guide)) = best_x {
        guides.push(SnapGuide {
            axis: Axis::Vertical,
            position: guide,
        });
        x
    } else {
        moving.x
    };
    let y = if let Some((_, y, guide)) = best_y {
        guides.push(SnapGuide {
            axis: Axis::Horizontal,
            position: guide,
        });
        y
    } else {
        moving.y
    };
    SnapResult { x, y, guides }
}

/// Places a dropped rectangle so the resulting arrangement is valid:
/// no overlap with `others`, and adjacent to at least one of them (when any
/// exist). Returns the corrected rectangle, moving it as little as possible.
pub fn settle_rect(moving: Rect, others: &[Rect]) -> Rect {
    if others.is_empty() {
        return moving;
    }

    let overlapping = others.iter().any(|o| moving.overlaps(o));
    let adjacent = others.iter().any(|o| moving.is_adjacent_to(o));
    if !overlapping && adjacent {
        return moving;
    }

    // Candidate positions: attach `moving` to each side of each other rect,
    // sliding along that side to stay as close as possible to the current
    // position (clamped so the shared border keeps positive length where the
    // side allows it).
    let mut best: Option<(i64, Rect)> = None;
    for other in others {
        let slide_y = clamp(moving.y, other.y - moving.height + 1, other.bottom() - 1);
        let slide_x = clamp(moving.x, other.x - moving.width + 1, other.right() - 1);
        let candidates = [
            Rect::new(other.x - moving.width, slide_y, moving.width, moving.height),
            Rect::new(other.right(), slide_y, moving.width, moving.height),
            Rect::new(
                slide_x,
                other.y - moving.height,
                moving.width,
                moving.height,
            ),
            Rect::new(slide_x, other.bottom(), moving.width, moving.height),
        ];
        for candidate in candidates {
            if others.iter().any(|o| candidate.overlaps(o)) {
                continue;
            }
            debug_assert!(others.iter().any(|o| candidate.is_adjacent_to(o)));
            let dx = i64::from(candidate.x - moving.x);
            let dy = i64::from(candidate.y - moving.y);
            let distance = dx * dx + dy * dy;
            if best.as_ref().is_none_or(|(d, _)| distance < *d) {
                best = Some((distance, candidate));
            }
        }
    }

    // Every non-degenerate arrangement offers at least one free side, but be
    // defensive: fall back to the original position rather than panicking.
    best.map_or(moving, |(_, rect)| rect)
}

fn clamp(v: i32, lo: i32, hi: i32) -> i32 {
    if lo > hi { lo } else { v.clamp(lo, hi) }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn snaps_left_edge_to_neighbours_right_edge() {
        let others = [Rect::new(0, 0, 1920, 1080)];
        let moving = Rect::new(1935, 12, 1920, 1080);
        let snapped = snap_rect(moving, &others, 24);
        assert_eq!((snapped.x, snapped.y), (1920, 0));
        assert_eq!(snapped.guides.len(), 2);
        assert!(snapped.guides.contains(&SnapGuide {
            axis: Axis::Vertical,
            position: 1920
        }));
        assert!(snapped.guides.contains(&SnapGuide {
            axis: Axis::Horizontal,
            position: 0
        }));
    }

    #[test]
    fn does_not_snap_beyond_threshold() {
        let others = [Rect::new(0, 0, 1920, 1080)];
        let moving = Rect::new(1990, 500, 1920, 1080);
        let snapped = snap_rect(moving, &others, 24);
        assert_eq!((snapped.x, snapped.y), (1990, 500));
        assert!(snapped.guides.is_empty());
    }

    #[test]
    fn settle_closes_gaps() {
        let others = [Rect::new(0, 0, 1920, 1080)];
        let dropped = Rect::new(2400, 200, 1920, 1080);
        let settled = settle_rect(dropped, &others);
        assert_eq!(settled, Rect::new(1920, 200, 1920, 1080));
    }

    #[test]
    fn settle_resolves_overlap() {
        let others = [Rect::new(0, 0, 1920, 1080)];
        let dropped = Rect::new(1000, 10, 1920, 1080);
        let settled = settle_rect(dropped, &others);
        assert!(!settled.overlaps(&others[0]));
        assert!(settled.is_adjacent_to(&others[0]));
        // Nearest free side is the right one.
        assert_eq!(settled.x, 1920);
    }

    #[test]
    fn settle_keeps_valid_position() {
        let others = [Rect::new(0, 0, 1920, 1080)];
        let dropped = Rect::new(1920, 0, 1000, 1000);
        assert_eq!(settle_rect(dropped, &others), dropped);
    }

    #[test]
    fn settle_with_no_others_is_identity() {
        let dropped = Rect::new(77, 88, 100, 100);
        assert_eq!(settle_rect(dropped, &[]), dropped);
    }

    #[test]
    fn settle_around_two_monitors_picks_nearest_side() {
        let others = [Rect::new(0, 0, 1920, 1080), Rect::new(1920, 0, 1920, 1080)];
        // Dropped fully inside the second monitor, close to its bottom edge.
        let dropped = Rect::new(2200, 700, 960, 540);
        let settled = settle_rect(dropped, &others);
        assert!(!settled.overlaps(&others[0]));
        assert!(!settled.overlaps(&others[1]));
        assert!(others.iter().any(|o| settled.is_adjacent_to(o)));
        assert_eq!(settled, Rect::new(2200, 1080, 960, 540));
    }
}
