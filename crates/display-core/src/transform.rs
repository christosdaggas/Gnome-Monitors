//! Monitor transforms (rotation / reflection), matching Mutter's numbering.

use serde::{Deserialize, Serialize};

/// Viewport transform of a logical display.
///
/// The `u32` values are the wire representation used by
/// `org.gnome.Mutter.DisplayConfig` (verified against Mutter 50.2 and its
/// bundled `gdctl`). The serde representation uses gdctl's string names so
/// saved profiles stay human-readable.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub enum Transform {
    #[default]
    #[serde(rename = "normal")]
    Normal,
    #[serde(rename = "90")]
    Rotate90,
    #[serde(rename = "180")]
    Rotate180,
    #[serde(rename = "270")]
    Rotate270,
    #[serde(rename = "flipped")]
    Flipped,
    #[serde(rename = "flipped-90")]
    Flipped90,
    #[serde(rename = "flipped-180")]
    Flipped180,
    #[serde(rename = "flipped-270")]
    Flipped270,
}

impl Transform {
    pub const ALL: [Transform; 8] = [
        Transform::Normal,
        Transform::Rotate90,
        Transform::Rotate180,
        Transform::Rotate270,
        Transform::Flipped,
        Transform::Flipped90,
        Transform::Flipped180,
        Transform::Flipped270,
    ];

    /// The four transforms GNOME Settings exposes in its orientation control.
    pub const BASIC: [Transform; 4] = [
        Transform::Normal,
        Transform::Rotate90,
        Transform::Rotate180,
        Transform::Rotate270,
    ];

    pub const fn as_u32(self) -> u32 {
        match self {
            Transform::Normal => 0,
            Transform::Rotate90 => 1,
            Transform::Rotate180 => 2,
            Transform::Rotate270 => 3,
            Transform::Flipped => 4,
            Transform::Flipped90 => 5,
            Transform::Flipped180 => 6,
            Transform::Flipped270 => 7,
        }
    }

    pub const fn from_u32(value: u32) -> Option<Transform> {
        Some(match value {
            0 => Transform::Normal,
            1 => Transform::Rotate90,
            2 => Transform::Rotate180,
            3 => Transform::Rotate270,
            4 => Transform::Flipped,
            5 => Transform::Flipped90,
            6 => Transform::Flipped180,
            7 => Transform::Flipped270,
            _ => return None,
        })
    }

    /// Whether the transform exchanges width and height.
    pub const fn swaps_dimensions(self) -> bool {
        matches!(
            self,
            Transform::Rotate90
                | Transform::Rotate270
                | Transform::Flipped90
                | Transform::Flipped270
        )
    }

    /// Applies the transform to a mode size.
    pub const fn apply_to(self, width: i32, height: i32) -> (i32, i32) {
        if self.swaps_dimensions() {
            (height, width)
        } else {
            (width, height)
        }
    }

    /// Technical name as used by gdctl and this application's profile files.
    pub const fn name(self) -> &'static str {
        match self {
            Transform::Normal => "normal",
            Transform::Rotate90 => "90",
            Transform::Rotate180 => "180",
            Transform::Rotate270 => "270",
            Transform::Flipped => "flipped",
            Transform::Flipped90 => "flipped-90",
            Transform::Flipped270 => "flipped-270",
            Transform::Flipped180 => "flipped-180",
        }
    }
}

impl std::fmt::Display for Transform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn wire_roundtrip() {
        for t in Transform::ALL {
            assert_eq!(Transform::from_u32(t.as_u32()), Some(t));
        }
        assert_eq!(Transform::from_u32(8), None);
    }

    #[test]
    fn flipped_numbering_matches_interface_doc() {
        // org.gnome.Mutter.DisplayConfig XML (50.2): "5: 90° flipped,
        // 6: 180° flipped, 7: 270° flipped" — the wl_output ordering.
        // (The gdctl bundled with Mutter 50 labels 6/7 the other way round;
        // the XML documentation and MtkMonitorTransform are authoritative.)
        assert_eq!(Transform::Flipped90.as_u32(), 5);
        assert_eq!(Transform::Flipped180.as_u32(), 6);
        assert_eq!(Transform::Flipped270.as_u32(), 7);
    }

    #[test]
    fn dimension_swap() {
        assert_eq!(Transform::Rotate90.apply_to(3840, 2160), (2160, 3840));
        assert_eq!(Transform::Rotate180.apply_to(3840, 2160), (3840, 2160));
    }

    #[test]
    fn serde_uses_gdctl_names() {
        let json = serde_json::to_string(&Transform::Flipped90).unwrap();
        assert_eq!(json, "\"flipped-90\"");
        let back: Transform = serde_json::from_str("\"270\"").unwrap();
        assert_eq!(back, Transform::Rotate270);
    }
}
