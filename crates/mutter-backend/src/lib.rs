//! Typed backend for `org.gnome.Mutter.DisplayConfig`.
//!
//! Wire signatures, property names, enum values, and validation semantics in
//! this crate were verified against the installed Mutter 50.2 (live
//! introspection + verify-mode probes) and the upstream 50.2 sources; see
//! `docs/mutter-dbus-notes.md`. Everything version-sensitive is parsed
//! defensively: unknown `a{sv}` entries are preserved, absent ones get their
//! documented defaults.

pub mod backend;
pub mod error;
pub mod fixtures;
pub mod parse;
pub mod proxy;
pub mod serialize;
pub mod shell_labels;

pub use backend::{BackendEvent, DisplayBackend, MockBackend, MutterBackend};
pub use error::{BackendError, RejectionKind};
pub use shell_labels::MonitorLabeler;
