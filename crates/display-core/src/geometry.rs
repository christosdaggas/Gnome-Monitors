//! Integer rectangle geometry in layout coordinate space.

use serde::{Deserialize, Serialize};

/// An axis-aligned rectangle in layout coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub const fn right(&self) -> i32 {
        self.x + self.width
    }

    pub const fn bottom(&self) -> i32 {
        self.y + self.height
    }

    /// True if the interiors of the two rectangles intersect. Rectangles that
    /// merely share an edge do not overlap.
    pub fn overlaps(&self, other: &Rect) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }

    /// Mutter 50's adjacency rule (`mtk_rectangle_is_adjacent_to`,
    /// mtk-rectangle.c:426): two rectangles are adjacent when they share a
    /// border of *positive length*. Touching only at a corner does **not**
    /// count.
    pub fn is_adjacent_to(&self, other: &Rect) -> bool {
        let (x1, y1, x2, y2) = (self.x, self.y, self.right(), self.bottom());
        let (ox1, oy1, ox2, oy2) = (other.x, other.y, other.right(), other.bottom());

        if (x1 == ox2 || x2 == ox1) && !(y2 <= oy1 || y1 >= oy2) {
            true
        } else {
            (y1 == oy2 || y2 == oy1) && !(x2 <= ox1 || x1 >= ox2)
        }
    }

    pub fn translated(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.width, self.height)
    }

    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.right() && py >= self.y && py < self.bottom()
    }

    pub const fn center(&self) -> (i32, i32) {
        (self.x + self.width / 2, self.y + self.height / 2)
    }
}

/// Smallest rectangle containing all given rectangles.
pub fn bounding_box<'a>(rects: impl IntoIterator<Item = &'a Rect>) -> Option<Rect> {
    let mut iter = rects.into_iter();
    let first = *iter.next()?;
    let (mut x1, mut y1, mut x2, mut y2) = (first.x, first.y, first.right(), first.bottom());
    for r in iter {
        x1 = x1.min(r.x);
        y1 = y1.min(r.y);
        x2 = x2.max(r.right());
        y2 = y2.max(r.bottom());
    }
    Some(Rect::new(x1, y1, x2 - x1, y2 - y1))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn edge_touch_is_not_overlap() {
        let a = Rect::new(0, 0, 1920, 1080);
        let b = Rect::new(1920, 0, 1920, 1080);
        assert!(!a.overlaps(&b));
        assert!(a.is_adjacent_to(&b));
    }

    #[test]
    fn interior_intersection_is_overlap() {
        let a = Rect::new(0, 0, 1920, 1080);
        let b = Rect::new(1919, 0, 1920, 1080);
        assert!(a.overlaps(&b));
    }

    #[test]
    fn gap_is_neither_overlap_nor_adjacent() {
        let a = Rect::new(0, 0, 1920, 1080);
        let b = Rect::new(2000, 0, 1920, 1080);
        assert!(!a.overlaps(&b));
        assert!(!a.is_adjacent_to(&b));
    }

    #[test]
    fn corner_touch_is_not_adjacent_in_mutter_50() {
        let a = Rect::new(0, 0, 100, 100);
        let b = Rect::new(100, 100, 100, 100);
        assert!(!a.is_adjacent_to(&b));
        assert!(!a.overlaps(&b));
    }

    #[test]
    fn vertical_adjacency() {
        let a = Rect::new(0, 0, 1920, 1080);
        let b = Rect::new(500, 1080, 1920, 1080);
        assert!(a.is_adjacent_to(&b));
        assert!(b.is_adjacent_to(&a));
    }

    #[test]
    fn bounding_box_spans_all() {
        let a = Rect::new(-10, 0, 20, 20);
        let b = Rect::new(50, 30, 10, 10);
        let bb = bounding_box([&a, &b]).unwrap();
        assert_eq!(bb, Rect::new(-10, 0, 70, 40));
    }
}
