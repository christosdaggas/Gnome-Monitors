//! Saved profiles: store, menu, and management dialogs.

use std::rc::Rc;

use adw::prelude::*;
use display_core::profile::DisplayProfile;
use display_core::{DisplayState, store};
use gtk::glib;
use serde::{Deserialize, Serialize};

use crate::UiCtx;

/// On-disk format (wrapper object so future fields stay compatible).
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ProfilesFile {
    #[serde(default)]
    pub profiles: Vec<DisplayProfile>,
}

/// Loads profiles. On a corrupt file the data is preserved as
/// `profiles.json.invalid` and an error message is returned for the UI.
pub fn load() -> (Vec<DisplayProfile>, Option<String>) {
    let Some(path) = display_core::paths::profiles_file() else {
        return (Vec::new(), None);
    };
    match store::read_json::<ProfilesFile>(&path) {
        Ok(Some(file)) => (file.profiles, None),
        Ok(None) => (Vec::new(), None),
        Err(e) => {
            let backup = path.with_extension("json.invalid");
            let moved = std::fs::rename(&path, &backup).is_ok();
            tracing::warn!("profiles file unreadable: {e}");
            let message = if moved {
                format!(
                    "The profiles file could not be read and was kept as {}",
                    backup.display()
                )
            } else {
                "The profiles file could not be read".to_owned()
            };
            (Vec::new(), Some(message))
        }
    }
}

/// Saves profiles; returns `false` (after logging) when the write failed so
/// callers can report honestly.
#[must_use]
pub fn save(profiles: &[DisplayProfile]) -> bool {
    let Some(path) = display_core::paths::profiles_file() else {
        return false;
    };
    let file = ProfilesFile {
        profiles: profiles.to_vec(),
    };
    match store::write_json_atomic(&path, &file) {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!("could not save profiles: {e}");
            false
        }
    }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn new_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("profile-{nanos}")
}

/// Rebuilds the profiles menu (list + management entries).
pub fn rebuild_menu(ctx: &Rc<UiCtx>) {
    let menu = &ctx.profiles_menu;
    menu.remove_all();
    let profiles = ctx.profiles.borrow();
    if !profiles.is_empty() {
        let section = gtk::gio::Menu::new();
        for profile in profiles.iter() {
            let item = gtk::gio::MenuItem::new(Some(&profile.name), None);
            item.set_action_and_target_value(
                Some("app.load-profile"),
                Some(&profile.id.to_variant()),
            );
            section.append_item(&item);
        }
        menu.append_section(Some("Load Profile"), &section);
    }
    let manage = gtk::gio::Menu::new();
    manage.append(Some("_Save Layout as Profile…"), Some("app.save-profile"));
    if !profiles.is_empty() {
        manage.append(Some("_Manage Profiles…"), Some("app.manage-profiles"));
    }
    menu.append_section(None, &manage);
}

/// `app.save-profile`
pub fn save_dialog(ctx: &Rc<UiCtx>) {
    let dialog = adw::AlertDialog::builder()
        .heading("Save Layout as Profile")
        .body("The layout currently shown in the editor will be saved.")
        .build();
    let entry = gtk::Entry::builder()
        .placeholder_text("Profile name")
        .activates_default(true)
        .build();
    let count = ctx.profiles.borrow().len() + 1;
    entry.set_text(&format!("Layout {count}"));
    dialog.set_extra_child(Some(&entry));
    dialog.add_response("cancel", "_Cancel");
    dialog.add_response("save", "_Save");
    dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("save"));
    dialog.set_close_response("cancel");

    let ctx2 = Rc::clone(ctx);
    dialog.connect_response(None, move |dialog, response| {
        let ctx = &ctx2;
        if response != "save" {
            return;
        }
        let name = {
            let text = entry_text(dialog);
            if text.trim().is_empty() {
                format!("Layout {}", now_unix())
            } else {
                text
            }
        };
        let snapshot_state: Option<DisplayState> = {
            let current = ctx.state.current.borrow();
            let edited = ctx.state.edited.borrow();
            match (current.as_ref(), edited.as_ref()) {
                (Some(state), Some(layout)) => Some(DisplayState {
                    layout: layout.clone(),
                    ..state.clone()
                }),
                _ => None,
            }
        };
        if let Some(state) = snapshot_state {
            let profile = DisplayProfile::from_state(new_id(), name.clone(), &state, now_unix());
            ctx.profiles.borrow_mut().push(profile);
            let saved = save(&ctx.profiles.borrow());
            rebuild_menu(ctx);
            if saved {
                ctx.toast(&format!("Profile “{name}” saved"));
            } else {
                ctx.toast("The profile could not be written to disk");
            }
        }
    });
    dialog.present(Some(&ctx.window));
}

fn entry_text(dialog: &adw::AlertDialog) -> String {
    dialog
        .extra_child()
        .and_then(|w| w.downcast::<gtk::Entry>().ok())
        .map(|e| e.text().to_string())
        .unwrap_or_default()
}

/// `app.load-profile` — resolves the profile and loads it into the editor.
/// Applying still goes through the explicit Apply flow.
pub fn load_into_editor(ctx: &Rc<UiCtx>, profile_id: &str) {
    let profile = {
        let profiles = ctx.profiles.borrow();
        profiles.iter().find(|p| p.id == profile_id).cloned()
    };
    let Some(profile) = profile else { return };

    let resolution = {
        let current = ctx.state.current.borrow();
        let Some(state) = current.as_ref() else {
            return;
        };
        profile.resolve(&state.monitors)
    };

    if resolution.is_exact() {
        apply_resolution(ctx, &profile.name, resolution.layout);
        return;
    }

    // Conservative: never silently apply an ambiguous or partial match.
    let problems: Vec<String> = resolution
        .problems
        .iter()
        .map(std::string::ToString::to_string)
        .collect();
    let can_partial = !resolution.layout.logical_displays.is_empty();
    let dialog = adw::AlertDialog::builder()
        .heading("Profile Does Not Fully Match")
        .body(format!(
            "“{}” cannot be matched exactly to the connected displays:\n\n• {}",
            profile.name,
            problems.join("\n• ")
        ))
        .build();
    dialog.add_response("cancel", "_Cancel");
    if can_partial {
        dialog.add_response("partial", "Load _Partial Layout");
        dialog.set_response_appearance("partial", adw::ResponseAppearance::Destructive);
    }
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    let ctx2 = Rc::clone(ctx);
    let name = profile.name.clone();
    let layout = resolution.layout;
    dialog.connect_response(None, move |_, response| {
        if response == "partial" {
            apply_resolution(&ctx2, &name, layout.clone());
        }
    });
    dialog.present(Some(&ctx.window));
}

fn apply_resolution(ctx: &Rc<UiCtx>, name: &str, layout: display_core::DisplayLayout) {
    let ctx2 = Rc::clone(ctx);
    let name = name.to_owned();
    crate::confirm_discard(ctx, "Load the profile and discard them?", move || {
        apply_resolution_inner(&ctx2, &name, layout);
    });
}

fn apply_resolution_inner(ctx: &Rc<UiCtx>, name: &str, layout: display_core::DisplayLayout) {
    {
        let current = ctx.state.current.borrow();
        let mut edited = ctx.state.edited.borrow_mut();
        let mut layout = layout;
        if let Some(state) = current.as_ref() {
            display_core::validation::normalize(&mut layout, &state.monitors);
        }
        *edited = Some(layout);
    }
    ctx.state.selected.set(None);
    ctx.after_structural_edit();
    ctx.toast(&format!(
        "Profile “{name}” loaded — review the layout, then press Apply"
    ));
}

/// `app.manage-profiles`
pub fn manage_dialog(ctx: &Rc<UiCtx>) {
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::None);
    list.add_css_class("boxed-list");

    let profiles = ctx.profiles.borrow().clone();
    for profile in profiles {
        let row = adw::EntryRow::builder().title("Name").build();
        row.set_text(&profile.name);
        row.set_show_apply_button(true);
        {
            let ctx = Rc::clone(ctx);
            let id = profile.id.clone();
            row.connect_apply(move |row| {
                let name = row.text().to_string();
                if name.trim().is_empty() {
                    return;
                }
                {
                    let mut profiles = ctx.profiles.borrow_mut();
                    if let Some(p) = profiles.iter_mut().find(|p| p.id == id) {
                        p.name = name;
                        p.modified_unix = now_unix();
                    }
                    if !save(&profiles) {
                        ctx.toast("The profile could not be written to disk");
                    }
                }
                rebuild_menu(&ctx);
                ctx.toast("Profile renamed");
            });
        }

        let duplicate = gtk::Button::from_icon_name("edit-copy-symbolic");
        duplicate.set_tooltip_text(Some("Duplicate profile"));
        duplicate.set_valign(gtk::Align::Center);
        duplicate.add_css_class("flat");
        {
            let ctx = Rc::clone(ctx);
            let id = profile.id.clone();
            duplicate.connect_clicked(move |_| {
                {
                    let mut profiles = ctx.profiles.borrow_mut();
                    if let Some(p) = profiles.iter().find(|p| p.id == id).cloned() {
                        let mut copy = p;
                        copy.id = new_id();
                        copy.name = format!("{} (copy)", copy.name);
                        copy.created_unix = now_unix();
                        copy.modified_unix = copy.created_unix;
                        profiles.push(copy);
                    }
                    if !save(&profiles) {
                        ctx.toast("The profile could not be written to disk");
                    }
                }
                rebuild_menu(&ctx);
                ctx.toast("Profile duplicated");
            });
        }
        row.add_suffix(&duplicate);

        let delete = gtk::Button::from_icon_name("user-trash-symbolic");
        delete.set_tooltip_text(Some("Delete profile"));
        delete.set_valign(gtk::Align::Center);
        delete.add_css_class("flat");
        {
            let ctx = Rc::clone(ctx);
            let id = profile.id.clone();
            let row_widget = row.clone();
            delete.connect_clicked(move |_| {
                {
                    let mut profiles = ctx.profiles.borrow_mut();
                    profiles.retain(|p| p.id != id);
                    if !save(&profiles) {
                        ctx.toast("The profile could not be written to disk");
                    }
                }
                rebuild_menu(&ctx);
                row_widget.set_visible(false);
                ctx.toast("Profile deleted");
            });
        }
        row.add_suffix(&delete);
        list.append(&row);
    }

    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .max_content_height(420)
        .child(&list)
        .build();
    scroll.set_margin_top(12);
    scroll.set_margin_bottom(12);
    scroll.set_margin_start(12);
    scroll.set_margin_end(12);

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&scroll));

    let dialog = adw::Dialog::builder()
        .title("Profiles")
        .content_width(420)
        .child(&toolbar)
        .build();
    dialog.present(Some(&ctx.window));
}

/// Registers the profile-related application actions.
pub fn register_actions(app: &adw::Application, ctx: &Rc<UiCtx>) {
    let save_action = {
        let ctx = Rc::clone(ctx);
        gtk::gio::ActionEntry::builder("save-profile")
            .activate(move |_: &adw::Application, _, _| save_dialog(&ctx))
            .build()
    };
    let load_action = {
        let ctx = Rc::clone(ctx);
        gtk::gio::ActionEntry::builder("load-profile")
            .parameter_type(Some(glib::VariantTy::STRING))
            .activate(move |_: &adw::Application, _, param| {
                if let Some(id) = param.and_then(glib::Variant::str) {
                    load_into_editor(&ctx, id);
                }
            })
            .build()
    };
    let manage_action = {
        let ctx = Rc::clone(ctx);
        gtk::gio::ActionEntry::builder("manage-profiles")
            .activate(move |_: &adw::Application, _, _| manage_dialog(&ctx))
            .build()
    };
    app.add_action_entries([save_action, load_action, manage_action]);
}
