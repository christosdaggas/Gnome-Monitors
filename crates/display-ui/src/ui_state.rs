//! Shared application state for the UI.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use display_core::prefs::AppPrefs;
use display_core::validation::{self, LayoutProblem};
use display_core::{DisplayLayout, DisplayState};
use mutter_backend::MutterBackend;

/// Everything the widgets need, single-threaded behind `Rc`.
pub struct AppState {
    pub backend: RefCell<Option<MutterBackend>>,
    /// Last state fetched from the compositor.
    pub current: RefCell<Option<DisplayState>>,
    /// The layout being edited (starts as a copy of the current layout).
    pub edited: RefCell<Option<DisplayLayout>>,
    /// Index into `edited.logical_displays` of the selected display.
    pub selected: Cell<Option<usize>>,
    pub prefs: RefCell<AppPrefs>,
    /// Guard: widget callbacks must not react while the UI is being rebuilt.
    pub rebuilding: Cell<bool>,
    /// Set while an apply operation is in flight.
    pub applying: Cell<bool>,
}

pub type SharedState = Rc<AppState>;

impl AppState {
    pub fn new() -> SharedState {
        let prefs = display_core::paths::prefs_file()
            .and_then(|p| display_core::store::read_json(&p).ok().flatten())
            .unwrap_or_default();
        Rc::new(AppState {
            backend: RefCell::new(None),
            current: RefCell::new(None),
            edited: RefCell::new(None),
            selected: Cell::new(None),
            prefs: RefCell::new(prefs),
            rebuilding: Cell::new(false),
            applying: Cell::new(false),
        })
    }

    pub fn save_prefs(&self) {
        if let Some(path) = display_core::paths::prefs_file()
            && let Err(e) = display_core::store::write_json_atomic(&path, &*self.prefs.borrow())
        {
            tracing::warn!("could not save preferences: {e}");
        }
    }

    /// Problems in the edited layout (empty = valid; `NotNormalized` is
    /// auto-fixed before apply, so it is filtered out here).
    pub fn problems(&self) -> Vec<LayoutProblem> {
        let current = self.current.borrow();
        let edited = self.edited.borrow();
        match (current.as_ref(), edited.as_ref()) {
            (Some(state), Some(layout)) => validation::validate_state(state, layout)
                .into_iter()
                .filter(|p| !p.is_auto_fixable())
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Whether the edited layout differs from what is applied.
    pub fn is_dirty(&self) -> bool {
        let current = self.current.borrow();
        let edited = self.edited.borrow();
        match (current.as_ref(), edited.as_ref()) {
            (Some(state), Some(layout)) => {
                let mut a = state.layout.clone();
                let mut b = layout.clone();
                validation::normalize(&mut a, &state.monitors);
                validation::normalize(&mut b, &state.monitors);
                a != b
            }
            _ => false,
        }
    }

    /// Position-based display numbers (left-to-right, top-to-bottom):
    /// `result[logical_index]` is the number to show for that display.
    pub fn display_numbers(&self) -> Vec<usize> {
        let edited = self.edited.borrow();
        let Some(layout) = edited.as_ref() else {
            return Vec::new();
        };
        let mut order: Vec<usize> = (0..layout.logical_displays.len()).collect();
        order.sort_by_key(|&i| {
            let l = &layout.logical_displays[i];
            (l.x, l.y)
        });
        let mut numbers = vec![0; order.len()];
        for (rank, index) in order.into_iter().enumerate() {
            numbers[index] = rank + 1;
        }
        numbers
    }

    /// Friendly display name for a connector.
    pub fn name_of(&self, connector: &str) -> String {
        let current = self.current.borrow();
        current
            .as_ref()
            .and_then(|s| s.monitor(connector))
            .map(|m| self.prefs.borrow().display_name(m))
            .unwrap_or_else(|| connector.to_owned())
    }

    pub fn is_kvm(&self, connector: &str) -> bool {
        let current = self.current.borrow();
        current
            .as_ref()
            .and_then(|s| s.monitor(connector))
            .is_some_and(|m| self.prefs.borrow().is_kvm(m))
    }
}
