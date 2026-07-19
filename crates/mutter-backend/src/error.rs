//! Backend errors with user-friendly translations.
//!
//! Mutter reports every rejection as a D-Bus error whose message is one of a
//! known set of English strings (verified against the 50.2 sources and live
//! probes). We classify them so the UI can show clear guidance instead of raw
//! D-Bus text, while keeping the original details available.

use thiserror::Error;

/// Why Mutter (or client-side validation) rejected a configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectionKind {
    /// "The requested configuration is based on stale information".
    StaleSerial,
    /// "Monitor configuration via D-Bus is disabled" (monitors.xml policy).
    PolicyDisabled,
    /// "Logical monitors not adjacent" (also covers gaps between islands).
    NotAdjacent,
    /// "Logical monitors overlap".
    Overlap,
    /// "Config contains multiple primary logical monitors".
    MultiplePrimary,
    /// "Config is missing primary logical" monitor.
    MissingPrimary,
    /// "Scale … not valid for resolution …" / "Scale not supported by backend".
    InvalidScale,
    /// "Invalid mode …" / "Monitor mode not found" / mode no longer available.
    InvalidMode,
    /// "Monitors modes in logical monitor not equal" (mirror resolution).
    MirrorModesUnequal,
    /// "Invalid connector …" / "Monitor not found" / disappeared monitor.
    UnknownMonitor,
    /// "Logical monitors positions are offset" / negative positions.
    BadPosition,
    /// "Underscanning requested but unsupported".
    Underscanning,
    /// Anything else.
    Other,
}

impl RejectionKind {
    fn classify(message: &str) -> RejectionKind {
        let m = message.to_ascii_lowercase();
        if m.contains("stale information") {
            RejectionKind::StaleSerial
        } else if m.contains("via d-bus is disabled") {
            RejectionKind::PolicyDisabled
        } else if m.contains("not adjacent") {
            RejectionKind::NotAdjacent
        } else if m.contains("overlap") {
            RejectionKind::Overlap
        } else if m.contains("multiple primary") {
            RejectionKind::MultiplePrimary
        } else if m.contains("missing primary") {
            RejectionKind::MissingPrimary
        } else if m.contains("scale") {
            RejectionKind::InvalidScale
        } else if m.contains("modes in logical monitor not equal") {
            RejectionKind::MirrorModesUnequal
        } else if m.contains("invalid mode") || m.contains("mode not") {
            RejectionKind::InvalidMode
        } else if m.contains("invalid connector")
            || m.contains("monitor not found")
            || m.contains("not found")
        {
            RejectionKind::UnknownMonitor
        } else if m.contains("positions are offset") || m.contains("monitor position") {
            RejectionKind::BadPosition
        } else if m.contains("underscanning") {
            RejectionKind::Underscanning
        } else {
            RejectionKind::Other
        }
    }
}

#[derive(Debug, Error)]
pub enum BackendError {
    /// The compositor rejected the configuration.
    #[error("{}", friendly_rejection(*.kind, .message))]
    Rejected {
        kind: RejectionKind,
        message: String,
    },
    /// The D-Bus call itself failed (service missing, disconnected, …).
    #[error("could not talk to the display server: {0}")]
    Dbus(#[from] zbus::Error),
    /// `GetCurrentState` returned something we could not interpret.
    #[error("could not interpret the display server's state: {0}")]
    Parse(String),
    /// A value could not be encoded for D-Bus (should not happen).
    #[error("could not encode the configuration: {0}")]
    Encode(String),
}

impl BackendError {
    /// Classifies a zbus error, turning Mutter's known rejection messages
    /// into [`BackendError::Rejected`].
    pub fn from_apply_error(error: zbus::Error) -> BackendError {
        if let zbus::Error::MethodError(name, detail, _) = &error {
            let message = detail.clone().unwrap_or_default();
            let name = name.as_str();
            if name.ends_with("AccessDenied")
                || name.ends_with("InvalidArgs")
                || name.ends_with("Failed")
            {
                let kind = RejectionKind::classify(&message);
                return BackendError::Rejected { kind, message };
            }
        }
        BackendError::Dbus(error)
    }

    /// True when the D-Bus peer denied the call outright (e.g. GNOME
    /// Shell's sender allowlist for `ShowMonitorLabels`).
    pub fn is_access_denied(&self) -> bool {
        match self {
            BackendError::Rejected { kind, .. } => *kind == RejectionKind::PolicyDisabled,
            BackendError::Dbus(zbus::Error::MethodError(name, ..)) => {
                name.as_str().ends_with("AccessDenied")
            }
            _ => false,
        }
    }

    /// Technical details for the expandable section of error dialogs.
    pub fn technical_details(&self) -> String {
        match self {
            BackendError::Rejected { kind, message } => {
                format!("Mutter rejection ({kind:?}): {message}")
            }
            BackendError::Dbus(e) => format!("D-Bus error: {e}"),
            BackendError::Parse(e) => format!("Parse error: {e}"),
            BackendError::Encode(e) => format!("Encode error: {e}"),
        }
    }
}

fn friendly_rejection(kind: RejectionKind, message: &str) -> String {
    match kind {
        RejectionKind::StaleSerial => {
            "The monitor configuration changed before it could be applied. The layout has been refreshed — please try again.".into()
        }
        RejectionKind::PolicyDisabled => {
            "Display configuration over D-Bus has been disabled by system policy (monitors.xml).".into()
        }
        RejectionKind::NotAdjacent => {
            "Mutter rejected this layout because the displays are not adjacent; every display must touch another one with no gaps.".into()
        }
        RejectionKind::Overlap => "Mutter rejected this layout because displays overlap.".into(),
        RejectionKind::MultiplePrimary => {
            "More than one display is marked as primary; exactly one is required.".into()
        }
        RejectionKind::MissingPrimary => {
            "No display is marked as primary; exactly one is required.".into()
        }
        RejectionKind::InvalidScale => {
            "The selected scale is unavailable for this mode.".into()
        }
        RejectionKind::InvalidMode => {
            "The selected mode is no longer available on one of the monitors.".into()
        }
        RejectionKind::MirrorModesUnequal => {
            "The selected displays do not share a compatible mirror resolution.".into()
        }
        RejectionKind::UnknownMonitor => {
            "One of the monitors in this layout is no longer connected.".into()
        }
        RejectionKind::BadPosition => {
            "The layout positions are invalid; displays must start at (0, 0) without negative coordinates.".into()
        }
        RejectionKind::Underscanning => {
            "Underscanning was requested for a monitor that does not support it.".into()
        }
        RejectionKind::Other => format!("The display server rejected the configuration: {message}"),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn classifies_known_mutter_messages() {
        let cases = [
            (
                "The requested configuration is based on stale information",
                RejectionKind::StaleSerial,
            ),
            ("Logical monitors not adjacent", RejectionKind::NotAdjacent),
            ("Logical monitors overlap", RejectionKind::Overlap),
            (
                "Config contains multiple primary logical monitors",
                RejectionKind::MultiplePrimary,
            ),
            (
                "Config is missing primary logical",
                RejectionKind::MissingPrimary,
            ),
            (
                "Scale 1.75 not valid for resolution 3840x2160",
                RejectionKind::InvalidScale,
            ),
            (
                "Scale not supported by backend",
                RejectionKind::InvalidScale,
            ),
            (
                "Invalid mode '800x600@56.000' specified",
                RejectionKind::InvalidMode,
            ),
            (
                "Monitors modes in logical monitor not equal",
                RejectionKind::MirrorModesUnequal,
            ),
            (
                "Invalid connector 'DP-9' specified",
                RejectionKind::UnknownMonitor,
            ),
            ("Monitor not found", RejectionKind::UnknownMonitor),
            (
                "Logical monitors positions are offset",
                RejectionKind::BadPosition,
            ),
            (
                "Invalid logical monitor position (-100, 0)",
                RejectionKind::BadPosition,
            ),
            (
                "Underscanning requested but unsupported",
                RejectionKind::Underscanning,
            ),
            (
                "Monitor configuration via D-Bus is disabled",
                RejectionKind::PolicyDisabled,
            ),
            ("something novel", RejectionKind::Other),
        ];
        for (message, expected) in cases {
            assert_eq!(RejectionKind::classify(message), expected, "{message}");
        }
    }

    #[test]
    fn friendly_messages_are_not_dbus_dumps() {
        let err = BackendError::Rejected {
            kind: RejectionKind::MirrorModesUnequal,
            message: "Monitors modes in logical monitor not equal".into(),
        };
        let text = err.to_string();
        assert!(text.contains("compatible mirror resolution"));
        assert!(!text.contains("logical monitor"));
        assert!(err.technical_details().contains("Monitors modes"));
    }
}
