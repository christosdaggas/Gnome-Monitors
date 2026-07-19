//! Monitor Layout — GNOME display layout manager (GTK 4 + libadwaita).

mod canvas;
mod identify;
mod panel;
mod preview;
mod profiles;
mod ui_state;

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use display_core::state::ApplyMethod;
use display_core::validation;
use futures_util::StreamExt;
use gtk::glib;
use mutter_backend::{DisplayBackend, MutterBackend};
use tracing::{info, warn};

use crate::canvas::Canvas;
use crate::ui_state::{AppState, SharedState};

pub const APP_ID: &str = "gr.hotwebdesign.MonitorLayout";

/// Widget context shared across the UI modules.
pub struct UiCtx {
    pub state: SharedState,
    pub profiles: RefCell<Vec<display_core::profile::DisplayProfile>>,
    pub profiles_load_error: RefCell<Option<String>>,
    pub profiles_menu: gtk::gio::Menu,
    pub canvas: Rc<Canvas>,
    pub panel: gtk::Box,
    pub banner: adw::Banner,
    pub apply_button: gtk::Button,
    pub reset_button: gtk::Button,
    pub toasts: adw::ToastOverlay,
    pub window: adw::ApplicationWindow,
}

impl UiCtx {
    pub fn toast(&self, message: &str) {
        self.toasts.add_toast(adw::Toast::new(message));
    }

    /// After any edit: redraw, revalidate, update button states.
    pub fn after_edit(self: &Rc<Self>) {
        self.canvas.refresh();
        self.update_status();
    }

    /// After an edit that changes the panel structure (group membership,
    /// enable/disable, mode changes): also rebuild the panel, deferred to an
    /// idle handler to avoid signal loops (the GNOME Settings pattern).
    pub fn after_structural_edit(self: &Rc<Self>) {
        self.after_edit();
        panel::queue_rebuild(self);
    }

    pub fn update_status(&self) {
        let problems = self.state.problems();
        let dirty = self.state.is_dirty();
        if let Some(first) = problems.first() {
            self.banner.set_title(&first.to_string());
            self.banner.set_revealed(true);
        } else {
            self.banner.set_revealed(false);
        }
        self.apply_button
            .set_sensitive(dirty && problems.is_empty() && !self.state.applying.get());
        self.reset_button.set_sensitive(dirty);
    }
}

fn main() -> glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(activate);
    app.run()
}

fn activate(app: &adw::Application) {
    // Single editor window: re-activation presents the existing one.
    if let Some(window) = app.active_window() {
        window.present();
        return;
    }

    // Make the bundled icon findable when running from the source tree.
    if let Some(display) = gtk::gdk::Display::default() {
        let theme = gtk::IconTheme::for_display(&display);
        let dev_icons = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/icons");
        if dev_icons.exists() {
            theme.add_search_path(&dev_icons);
        }
    }

    identify::init_css();

    let state = AppState::new();
    let canvas = Canvas::new(Rc::clone(&state));

    // ---- Header bar ----
    let header = adw::HeaderBar::new();
    let refresh_button = gtk::Button::from_icon_name("view-refresh-symbolic");
    refresh_button.set_tooltip_text(Some("Reload the current monitor state"));
    header.pack_start(&refresh_button);
    let identify_button = gtk::Button::with_label("Identify");
    identify_button.set_tooltip_text(Some(
        "Show identification numbers on each display (also shown automatically while this window is focused)",
    ));
    header.pack_start(&identify_button);

    let profiles_menu = gtk::gio::Menu::new();
    let profiles_button = gtk::MenuButton::builder()
        .label("Profiles")
        .menu_model(&profiles_menu)
        .build();
    header.pack_start(&profiles_button);

    let menu = gtk::gio::Menu::new();
    menu.append(Some("_About Monitor Layout"), Some("app.about"));
    let menu_button = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .menu_model(&menu)
        .build();
    header.pack_end(&menu_button);

    let apply_button = gtk::Button::with_label("Apply…");
    apply_button.add_css_class("suggested-action");
    apply_button.set_sensitive(false);
    header.pack_end(&apply_button);

    let reset_button = gtk::Button::with_label("Reset");
    reset_button.set_tooltip_text(Some("Discard edits and return to the current layout"));
    reset_button.set_sensitive(false);
    header.pack_end(&reset_button);

    // ---- Content ----
    let banner = adw::Banner::new("");
    let panel_box = gtk::Box::new(gtk::Orientation::Vertical, 12);
    panel_box.set_margin_top(12);
    panel_box.set_margin_bottom(12);
    panel_box.set_margin_start(12);
    panel_box.set_margin_end(12);
    let panel_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&panel_box)
        .build();

    let split = adw::OverlaySplitView::new();
    split.set_sidebar_position(gtk::PackType::End);
    split.set_min_sidebar_width(320.0);
    split.set_max_sidebar_width(420.0);
    split.set_content(Some(&canvas.widget));
    split.set_sidebar(Some(&panel_scroll));

    // A header button always toggles the display-settings sidebar (it shows
    // as an overlay when the window is narrow, side-by-side otherwise).
    let sidebar_toggle = gtk::ToggleButton::builder()
        .icon_name("sidebar-show-right-symbolic")
        .tooltip_text("Show or hide the display settings sidebar")
        .build();
    // Bind FROM the split view so the initial sync copies the split's
    // default (sidebar shown) onto the toggle — not the other way round.
    split
        .bind_property("show-sidebar", &sidebar_toggle, "active")
        .bidirectional()
        .sync_create()
        .build();
    header.pack_end(&sidebar_toggle);

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&banner);
    content.append(&split);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&content));

    let toasts = adw::ToastOverlay::new();
    toasts.set_child(Some(&toolbar));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Monitor Layout")
        .default_width(1080)
        .default_height(680)
        .content(&toasts)
        .build();
    let breakpoint = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        860.0,
        adw::LengthUnit::Px,
    ));
    breakpoint.add_setter(&split, "collapsed", Some(&true.to_value()));
    window.add_breakpoint(breakpoint);

    let (loaded_profiles, profiles_load_error) = profiles::load();
    let ctx = Rc::new(UiCtx {
        state,
        profiles: RefCell::new(loaded_profiles),
        profiles_load_error: RefCell::new(profiles_load_error),
        profiles_menu,
        canvas,
        panel: panel_box,
        banner,
        apply_button,
        reset_button,
        toasts,
        window: window.clone(),
    });

    ctx.canvas.set_on_changed({
        let ctx = Rc::clone(&ctx);
        move || {
            ctx.update_status();
            panel::queue_rebuild(&ctx);
        }
    });

    if let Some(error) = ctx.profiles_load_error.borrow_mut().take() {
        ctx.toast(&error);
    }
    ctx.refresh_button_wiring(&refresh_button, &identify_button);
    ctx.apply_wiring();
    ctx.reset_wiring();
    about_action(app);
    profiles::register_actions(app, &ctx);
    profiles::rebuild_menu(&ctx);
    preview::startup_check(&ctx);

    // Show the numbered identification overlays (GNOME Shell's own monitor
    // labels) while the window is focused — the GNOME Settings behaviour.
    window.connect_is_active_notify({
        let ctx = Rc::clone(&ctx);
        move |window| {
            let active = window.is_active();
            let ctx = Rc::clone(&ctx);
            glib::spawn_future_local(async move {
                if active {
                    identify::show_auto(&ctx).await;
                } else {
                    identify::hide_auto().await;
                }
            });
        }
    });
    window.connect_close_request({
        let ctx = Rc::clone(&ctx);
        move |window| {
            identify::hide_overlays();
            glib::spawn_future_local(async {
                identify::hide_auto().await;
            });
            if ctx.state.is_dirty() {
                let window = window.clone();
                confirm_discard(&ctx, "Close the window and discard them?", move || {
                    window.destroy();
                });
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        }
    });

    window.present();

    // Initial load + event loop.
    let ctx2 = Rc::clone(&ctx);
    glib::spawn_future_local(async move {
        match MutterBackend::connect().await {
            Ok(backend) => {
                *ctx2.state.backend.borrow_mut() = Some(backend.clone());
                refresh(&ctx2, false).await;
                // Self-test hook: show the identify overlays right after
                // startup (used by automated UI verification).
                if std::env::var_os("MONITOR_LAYOUT_IDENTIFY_ON_START").is_some() {
                    identify::show(&ctx2).await;
                }
                // Demo hook: build a mirror group in the EDITOR ONLY (never
                // applied) — used for screenshots and visual verification.
                if std::env::var_os("MONITOR_LAYOUT_DEMO_MIRROR").is_some() {
                    let merged = {
                        let current = ctx2.state.current.borrow();
                        let mut edited = ctx2.state.edited.borrow_mut();
                        if let (Some(state), Some(layout)) = (current.as_ref(), edited.as_mut()) {
                            let kvm = state
                                .monitors
                                .iter()
                                .find(|m| ctx2.state.prefs.borrow().is_kvm(m))
                                .map(|m| m.connector().to_owned());
                            let anchor = state
                                .layout
                                .primary()
                                .and_then(|l| l.monitors.first())
                                .map(|a| a.connector.clone());
                            if let (Some(kvm), Some(anchor)) = (kvm, anchor) {
                                display_core::layout_ops::merge_into_mirror(
                                    state, layout, &anchor, &kvm,
                                )
                                .is_ok()
                                .then_some(anchor)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    };
                    if let Some(anchor) = merged {
                        let index = ctx2
                            .state
                            .edited
                            .borrow()
                            .as_ref()
                            .and_then(|l| l.group_of(&anchor).map(|(i, _)| i));
                        ctx2.state.selected.set(index);
                        ctx2.after_structural_edit();
                    }
                }
                listen(&ctx2, backend).await;
            }
            Err(e) => {
                ctx2.banner
                    .set_title(&format!("Could not connect to the display server: {e}"));
                ctx2.banner.set_revealed(true);
            }
        }
    });
}

impl UiCtx {
    fn refresh_button_wiring(
        self: &Rc<Self>,
        refresh_button: &gtk::Button,
        identify: &gtk::Button,
    ) {
        let ctx = Rc::clone(self);
        refresh_button.connect_clicked(move |_| {
            let ctx2 = Rc::clone(&ctx);
            confirm_discard(
                &ctx,
                "Reload the monitor state and discard them?",
                move || {
                    glib::spawn_future_local(async move {
                        if refresh(&ctx2, false).await {
                            ctx2.toast("Monitor state reloaded");
                        }
                    });
                },
            );
        });

        let ctx = Rc::clone(self);
        identify.connect_clicked(move |_| {
            let ctx = Rc::clone(&ctx);
            glib::spawn_future_local(async move {
                identify::show(&ctx).await;
            });
        });
    }

    fn reset_wiring(self: &Rc<Self>) {
        let ctx = Rc::clone(self);
        self.reset_button.connect_clicked(move |_| {
            {
                let current = ctx.state.current.borrow();
                if let Some(state) = current.as_ref() {
                    *ctx.state.edited.borrow_mut() = Some(state.layout.clone());
                }
            }
            ctx.state.selected.set(None);
            ctx.after_structural_edit();
            ctx.toast("Edits discarded");
        });
    }

    fn apply_wiring(self: &Rc<Self>) {
        let ctx = Rc::clone(self);
        self.apply_button.connect_clicked(move |_| {
            let dialog = adw::AlertDialog::builder()
                .heading("Apply Display Configuration?")
                .body(
                    "The new layout is applied using GNOME's standard confirmation: \
                     a system dialog will ask you to keep the changes and everything \
                     reverts automatically after 20 seconds if you do not confirm.\n\n\
                     Screens may blank briefly while modes change. Note: GNOME Shell \
                     on this machine has crashed during display changes before — if \
                     that happens, the session restarts and no change is saved.",
                )
                .build();
            dialog.add_response("cancel", "_Cancel");
            let preview_seconds = ctx.state.prefs.borrow().confirm_seconds.max(5);
            dialog.add_response("preview", &format!("_Try for {preview_seconds} s"));
            dialog.add_response("apply", "_Apply");
            dialog.set_response_appearance("apply", adw::ResponseAppearance::Suggested);
            dialog.set_default_response(Some("cancel"));
            dialog.set_close_response("cancel");
            let ctx2 = Rc::clone(&ctx);
            dialog.connect_response(None, move |_, response| {
                let ctx2 = Rc::clone(&ctx2);
                match response {
                    "apply" => {
                        glib::spawn_future_local(async move {
                            do_apply(&ctx2).await;
                        });
                    }
                    "preview" => {
                        glib::spawn_future_local(async move {
                            preview::start(&ctx2).await;
                        });
                    }
                    _ => {}
                }
            });
            dialog.present(Some(&ctx.window));
        });
    }
}

pub(crate) async fn refresh(ctx: &Rc<UiCtx>, preserve_edits: bool) -> bool {
    let backend = ctx.state.backend.borrow().clone();
    let Some(backend) = backend else { return false };
    match backend.current_state().await {
        Ok(state) => {
            let keep_edits = preserve_edits && ctx.state.is_dirty();
            {
                let mut current = ctx.state.current.borrow_mut();
                *current = Some(state);
            }
            if !keep_edits {
                let current = ctx.state.current.borrow();
                if let Some(state) = current.as_ref() {
                    *ctx.state.edited.borrow_mut() = Some(state.layout.clone());
                }
            }
            // Clamp the selection.
            let count = ctx
                .state
                .edited
                .borrow()
                .as_ref()
                .map_or(0, |l| l.logical_displays.len());
            if ctx.state.selected.get().is_some_and(|i| i >= count) {
                ctx.state.selected.set(None);
            }
            // First load: pre-select the primary display so the sidebar is
            // immediately useful and keyboard-navigable.
            if !preserve_edits && ctx.state.selected.get().is_none() {
                let primary_index = ctx
                    .state
                    .edited
                    .borrow()
                    .as_ref()
                    .and_then(|layout| layout.logical_displays.iter().position(|l| l.primary));
                ctx.state.selected.set(primary_index);
            }
            ctx.canvas.refresh();
            panel::rebuild(ctx);
            ctx.update_status();
            true
        }
        Err(e) => {
            warn!("refresh failed: {e}");
            ctx.banner
                .set_title(&format!("Could not read the monitor state: {e}"));
            ctx.banner.set_revealed(true);
            false
        }
    }
}

/// Runs `action` immediately when there are no unsaved edits; otherwise asks
/// the user to confirm discarding them first.
pub(crate) fn confirm_discard(ctx: &Rc<UiCtx>, question: &str, action: impl FnOnce() + 'static) {
    if !ctx.state.is_dirty() {
        action();
        return;
    }
    let dialog = adw::AlertDialog::builder()
        .heading("Discard Unapplied Changes?")
        .body(format!(
            "The layout has edits that were not applied. {question}"
        ))
        .build();
    dialog.add_response("cancel", "_Cancel");
    dialog.add_response("discard", "_Discard Changes");
    dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    let action = std::cell::RefCell::new(Some(action));
    dialog.connect_response(None, move |_, response| {
        if response == "discard"
            && let Some(action) = action.borrow_mut().take()
        {
            action();
        }
    });
    dialog.present(Some(&ctx.window));
}

async fn listen(ctx: &Rc<UiCtx>, backend: MutterBackend) {
    match backend.events().await {
        Ok(mut events) => {
            while events.next().await.is_some() {
                info!("MonitorsChanged received; refreshing");
                refresh(ctx, true).await;
                if ctx.state.is_dirty() {
                    ctx.toast("The monitor configuration changed while you were editing");
                }
            }
        }
        Err(e) => warn!("could not subscribe to MonitorsChanged: {e}"),
    }
}

/// Fetches a fresh compositor state immediately before sending a
/// configuration. Aborts (with an explanation and a UI refresh) when the
/// monitor topology changed compared to what the editor was based on, or
/// when the edited layout is no longer valid against the fresh state.
/// On success the fresh state replaces the cached one and the fresh serial
/// is returned with the normalized layout.
async fn fresh_state_for_send(
    ctx: &Rc<UiCtx>,
    backend: &MutterBackend,
) -> Option<(u32, display_core::DisplayLayout)> {
    let fresh = match backend.current_state().await {
        Ok(state) => state,
        Err(e) => {
            error_dialog(ctx, "Could Not Read the Current Monitor State", &e);
            return None;
        }
    };

    let cached_keys: Vec<String> = ctx
        .state
        .current
        .borrow()
        .as_ref()
        .map(|s| s.monitors.iter().map(|m| m.identity.stable_key()).collect())
        .unwrap_or_default();
    let fresh_keys: Vec<String> = fresh
        .monitors
        .iter()
        .map(|m| m.identity.stable_key())
        .collect();

    let layout = {
        let edited = ctx.state.edited.borrow();
        edited.as_ref().cloned()
    }?;

    if cached_keys != fresh_keys {
        *ctx.state.current.borrow_mut() = Some(fresh);
        refresh(ctx, true).await;
        ctx.toast("The connected monitors changed — please review the layout and try again");
        return None;
    }

    let mut layout = layout;
    validation::normalize(&mut layout, &fresh.monitors);
    let problems: Vec<String> = validation::validate_state(&fresh, &layout)
        .into_iter()
        .filter(|p| !p.is_auto_fixable())
        .map(|p| p.to_string())
        .collect();
    if !problems.is_empty() {
        *ctx.state.current.borrow_mut() = Some(fresh);
        refresh(ctx, true).await;
        ctx.toast(&problems.join(" "));
        return None;
    }

    let serial = fresh.serial;
    *ctx.state.current.borrow_mut() = Some(fresh);
    Some((serial, layout))
}

async fn do_apply(ctx: &Rc<UiCtx>) {
    if ctx.state.applying.get() {
        return;
    }
    let backend = ctx.state.backend.borrow().clone();
    let Some(backend) = backend else { return };

    // Genuinely fresh state, fetched right now — not the cached snapshot.
    let Some((serial, layout)) = fresh_state_for_send(ctx, &backend).await else {
        return;
    };

    ctx.state.applying.set(true);
    ctx.update_status();

    // Verify first (never changes anything), then apply persistently —
    // Mutter itself arms the 20 s auto-revert and GNOME Shell asks the user
    // to keep the changes (the exact GNOME Settings flow).
    let result = match backend.apply(serial, &layout, ApplyMethod::Verify).await {
        Ok(()) => {
            backend
                .apply(serial, &layout, ApplyMethod::Persistent)
                .await
        }
        Err(e) => Err(e),
    };

    ctx.state.applying.set(false);
    match result {
        Ok(()) => {
            info!("configuration applied persistently (awaiting shell confirmation)");
            ctx.toast(
                "Applied — confirm “Keep Changes” in the system dialog, or it reverts in 20 s",
            );
            refresh(ctx, false).await;
        }
        Err(e) => {
            error_dialog(ctx, "The Configuration Was Not Applied", &e);
            refresh(ctx, true).await;
        }
    }
}

/// Friendly error dialog with an expandable technical-details section.
pub(crate) fn error_dialog(ctx: &Rc<UiCtx>, heading: &str, e: &mutter_backend::BackendError) {
    let dialog = adw::AlertDialog::builder()
        .heading(heading)
        .body(e.to_string())
        .build();
    let details = gtk::Expander::builder().label("Technical details").build();
    let label = gtk::Label::builder()
        .label(e.technical_details())
        .wrap(true)
        .selectable(true)
        .xalign(0.0)
        .build();
    label.add_css_class("monospace");
    label.add_css_class("caption");
    details.set_child(Some(&label));
    dialog.set_extra_child(Some(&details));
    dialog.add_response("ok", "_OK");
    dialog.set_default_response(Some("ok"));
    dialog.present(Some(&ctx.window));
}

fn about_action(app: &adw::Application) {
    let about = gtk::gio::ActionEntry::builder("about")
        .activate(|app: &adw::Application, _, _| {
            let dialog = adw::AboutDialog::builder()
                .application_name("Monitor Layout")
                .application_icon(APP_ID)
                .version(env!("CARGO_PKG_VERSION"))
                .developer_name("Christos A. Daggas")
                .license_type(gtk::License::Gpl30)
                .comments(
                    "Visual display layout manager for GNOME on Wayland, with partial \
                     mirroring for KVM setups. Talks directly to Mutter's DisplayConfig \
                     D-Bus interface.",
                )
                .build();
            if let Some(window) = app.active_window() {
                dialog.present(Some(&window));
            }
        })
        .build();
    app.add_action_entries([about]);
}
