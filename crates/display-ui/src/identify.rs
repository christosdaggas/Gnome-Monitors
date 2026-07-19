//! Numbered display identification.
//!
//! Primary path: GNOME Shell's `ShowMonitorLabels` — the mechanism GNOME
//! Settings uses. Since GNOME Shell restricts that method to an allowlist of
//! D-Bus senders (third-party apps get `AccessDenied`), a native fallback
//! draws our own overlays: one fullscreen window per monitor showing a big
//! centered number (Mutter backs fullscreen surfaces with opaque black, so
//! the overlay is styled as a deliberate, brief identification screen).
//! Wayland offers no other way to place a window on a specific monitor than
//! `fullscreen_on_monitor`.
//!
//! The automatic show-on-focus behaviour only ever uses the Shell path,
//! because the fallback windows necessarily take keyboard focus and would
//! otherwise fight the focus tracking. The Identify button uses both.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use mutter_backend::{BackendError, MonitorLabeler};
use tracing::info;

use crate::UiCtx;

thread_local! {
    static OVERLAYS: RefCell<Vec<gtk::Window>> = const { RefCell::new(Vec::new()) };
    static SHELL_DENIED: Cell<bool> = const { Cell::new(false) };
}

/// (connector, position-based number, group label text)
fn collect(ctx: &Rc<UiCtx>) -> Vec<(String, i32, String)> {
    let position_numbers = ctx.state.display_numbers();
    let edited = ctx.state.edited.borrow();
    let Some(layout) = edited.as_ref() else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for (i, logical) in layout.logical_displays.iter().enumerate() {
        let number = position_numbers.get(i).copied().unwrap_or(i + 1);
        let names: Vec<String> = logical
            .monitors
            .iter()
            .map(|a| ctx.state.name_of(&a.connector))
            .collect();
        let label = names.join("  +  ");
        for assignment in &logical.monitors {
            #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
            result.push((assignment.connector.clone(), number as i32, label.clone()));
        }
    }
    result
}

async fn shell_show(numbers: &[(String, i32, String)]) -> Result<(), BackendError> {
    let labeler = MonitorLabeler::connect().await?;
    let pairs: Vec<(String, i32)> = numbers.iter().map(|(c, n, _)| (c.clone(), *n)).collect();
    labeler.show(&pairs).await
}

fn is_access_denied(error: &BackendError) -> bool {
    error.is_access_denied()
}

/// Focus-driven labels: Shell path only (silently unavailable when denied).
pub async fn show_auto(ctx: &Rc<UiCtx>) {
    if SHELL_DENIED.with(Cell::get) {
        return;
    }
    let numbers = collect(ctx);
    if numbers.len() < 2 {
        return;
    }
    if let Err(e) = shell_show(&numbers).await
        && is_access_denied(&e)
    {
        info!(
            "GNOME Shell restricts ShowMonitorLabels to allowlisted senders; using overlay fallback"
        );
        SHELL_DENIED.with(|d| d.set(true));
    }
}

pub async fn hide_auto() {
    if SHELL_DENIED.with(Cell::get) {
        return;
    }
    if let Ok(labeler) = MonitorLabeler::connect().await {
        let _ = labeler.hide().await;
    }
}

/// The Identify button: Shell labels when permitted, otherwise our overlays.
pub async fn show(ctx: &Rc<UiCtx>) {
    let numbers = collect(ctx);
    if numbers.len() < 2 {
        ctx.toast("Only one display — nothing to identify");
        return;
    }
    if !SHELL_DENIED.with(Cell::get) {
        match shell_show(&numbers).await {
            Ok(()) => {
                // Shell keeps them up; auto-hide like the focus path does.
                glib::timeout_add_seconds_local_once(4, move || {
                    glib::spawn_future_local(async {
                        hide_auto().await;
                    });
                });
                return;
            }
            Err(e) if is_access_denied(&e) => {
                info!("ShowMonitorLabels denied; falling back to overlay windows");
                SHELL_DENIED.with(|d| d.set(true));
            }
            Err(e) => {
                ctx.toast(&format!("Could not show identification labels: {e}"));
                return;
            }
        }
    }
    show_overlays(ctx, &numbers);
}

fn show_overlays(ctx: &Rc<UiCtx>, numbers: &[(String, i32, String)]) {
    hide_overlays();
    let Some(display) = gtk::gdk::Display::default() else {
        return;
    };
    let monitors = display.monitors();
    let mut shown = 0;
    for index in 0..monitors.n_items() {
        let Some(monitor) = monitors
            .item(index)
            .and_then(|o| o.downcast::<gtk::gdk::Monitor>().ok())
        else {
            continue;
        };
        let Some(connector) = monitor.connector() else {
            continue;
        };
        let Some((_, number, group_label)) = numbers
            .iter()
            .find(|(c, ..)| c.as_str() == connector.as_str())
        else {
            continue;
        };

        let number_label = gtk::Label::new(Some(&number.to_string()));
        number_label.add_css_class("identify-number");
        let name_label = gtk::Label::new(Some(group_label));
        name_label.add_css_class("identify-name");
        let hint_label = gtk::Label::new(Some("Click anywhere to dismiss"));
        hint_label.add_css_class("identify-hint");
        let badge = gtk::Box::new(gtk::Orientation::Vertical, 10);
        badge.append(&number_label);
        badge.append(&name_label);
        badge.append(&hint_label);
        badge.set_halign(gtk::Align::Center);
        badge.set_valign(gtk::Align::Center);

        let window = gtk::Window::new();
        window.add_css_class("identify-overlay");
        window.set_decorated(false);
        window.set_child(Some(&badge));
        if let Some(app) = ctx.window.application() {
            window.set_application(Some(&app));
        }

        // Dismiss on click or Esc.
        let click = gtk::GestureClick::new();
        click.connect_pressed(|_, _, _, _| hide_overlays());
        window.add_controller(click);
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(|_, _, _, _| {
            hide_overlays();
            glib::Propagation::Stop
        });
        window.add_controller(keys);

        window.fullscreen_on_monitor(&monitor);
        window.present();
        OVERLAYS.with(|overlays| overlays.borrow_mut().push(window));
        shown += 1;
    }
    if shown == 0 {
        ctx.toast("Could not match monitors for identification");
        return;
    }
    let hold = std::env::var("MONITOR_LAYOUT_IDENTIFY_HOLD")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);
    glib::timeout_add_seconds_local_once(hold, hide_overlays);
}

pub fn hide_overlays() {
    OVERLAYS.with(|overlays| {
        for window in overlays.borrow_mut().drain(..) {
            window.close();
        }
    });
}

/// CSS for the overlay badges (Shell-label look).
pub fn init_css() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(
        "window.identify-overlay { background: #1c1c22; color: white; }
         .identify-number { font-size: 220px; font-weight: 800; }
         .identify-name { font-size: 26px; opacity: 0.9; }
         .identify-hint { font-size: 14px; opacity: 0.45; margin-top: 24px; }",
    );
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
