//! The configuration side panel for the selected display / mirror group.

use std::rc::Rc;

use adw::prelude::*;
use display_core::layout::{LogicalDisplay, MonitorAssignment};
use display_core::mirror;
use display_core::validation;
use display_core::{Transform, format_refresh, format_scale_percent};
use gtk::glib;

use crate::UiCtx;

/// Orientation choices shown in the panel (unambiguous technical labels).
const ORIENTATIONS: [(Transform, &str); 4] = [
    (Transform::Normal, "Landscape"),
    (Transform::Rotate90, "Portrait (90°)"),
    (Transform::Rotate180, "Upside-down (180°)"),
    (Transform::Rotate270, "Portrait (270°)"),
];

/// Rebuilds the whole panel for the current selection.
pub fn rebuild(ctx: &Rc<UiCtx>) {
    ctx.state.rebuilding.set(true);
    while let Some(child) = ctx.panel.first_child() {
        ctx.panel.remove(&child);
    }

    let current = ctx.state.current.borrow();
    let edited = ctx.state.edited.borrow();
    let (Some(state), Some(layout)) = (current.as_ref(), edited.as_ref()) else {
        ctx.state.rebuilding.set(false);
        return;
    };

    match ctx
        .state
        .selected
        .get()
        .and_then(|i| layout.logical_displays.get(i).map(|l| (i, l)))
    {
        Some((index, logical)) => {
            build_selected(ctx, state, layout, index, logical);
        }
        None => {
            let empty = adw::StatusPage::builder()
                .icon_name("video-display-symbolic")
                .title("No Display Selected")
                .description("Select a display on the canvas to configure it.")
                .build();
            empty.set_vexpand(true);
            ctx.panel.append(&empty);
        }
    }

    build_disabled_section(ctx, state, layout);

    drop(current);
    drop(edited);
    ctx.state.rebuilding.set(false);
}

fn group(title: &str) -> adw::PreferencesGroup {
    adw::PreferencesGroup::builder().title(title).build()
}

#[allow(clippy::too_many_lines)]
fn build_selected(
    ctx: &Rc<UiCtx>,
    state: &display_core::DisplayState,
    layout: &display_core::DisplayLayout,
    index: usize,
    logical: &LogicalDisplay,
) {
    // ---- Members / mirror group ----
    let members_group = if logical.is_mirror_group() {
        group(&format!(
            "Mirror Group — {} Displays",
            logical.monitors.len()
        ))
    } else {
        group("Display")
    };

    for (member_index, assignment) in logical.monitors.iter().enumerate() {
        let monitor = state.monitor(&assignment.connector);
        let name = ctx.state.name_of(&assignment.connector);
        let mut subtitle = assignment.connector.clone();
        if let Some(monitor) = monitor {
            let identity = &monitor.identity;
            if identity.has_edid() {
                subtitle = format!(
                    "{} · {} {}",
                    assignment.connector, identity.vendor, identity.product
                );
            }
            if let Some(mode) = monitor.find_mode(&assignment.mode_id) {
                subtitle.push_str(&format!(" · {}", format_refresh(mode.refresh_hz)));
            }
        }
        let row = adw::ActionRow::builder()
            .title(&name)
            .subtitle(&subtitle)
            .build();
        if ctx.state.is_kvm(&assignment.connector) {
            let badge = gtk::Label::new(Some("KVM"));
            badge.add_css_class("caption");
            badge.add_css_class("accent");
            row.add_suffix(&badge);
        }
        if logical.is_mirror_group() {
            let remove = gtk::Button::from_icon_name("list-remove-symbolic");
            remove.set_tooltip_text(Some("Remove from mirror group"));
            remove.set_valign(gtk::Align::Center);
            remove.add_css_class("flat");
            let ctx2 = Rc::clone(ctx);
            remove.connect_clicked(move |_| split_member(&ctx2, index, member_index));
            row.add_suffix(&remove);
        }
        members_group.add(&row);
    }

    // "Mirror with…" — other displays that could join this group.
    let mut mirror_options: Vec<(String, String)> = Vec::new(); // (connector, label)
    for other in &layout.logical_displays {
        if std::ptr::eq(other, logical) {
            continue;
        }
        for a in &other.monitors {
            mirror_options.push((a.connector.clone(), ctx.state.name_of(&a.connector)));
        }
    }
    for disabled in layout.disabled_monitors(&state.monitors) {
        mirror_options.push((
            disabled.connector().to_owned(),
            ctx.state.name_of(disabled.connector()),
        ));
    }
    if !mirror_options.is_empty() {
        let labels: Vec<String> = std::iter::once("Select a display…".to_owned())
            .chain(mirror_options.iter().map(|(c, n)| format!("{n} ({c})")))
            .collect();
        let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let combo = adw::ComboRow::builder().title("Mirror With").build();
        combo.set_subtitle("Add another display to this mirror group");
        combo.set_model(Some(&gtk::StringList::new(&label_refs)));
        combo.set_selected(0);
        let ctx2 = Rc::clone(ctx);
        let options = mirror_options.clone();
        combo.connect_selected_notify(move |combo| {
            if ctx2.state.rebuilding.get() {
                return;
            }
            let selected = combo.selected() as usize;
            if selected == 0 {
                return;
            }
            if let Some((connector, _)) = options.get(selected - 1) {
                add_mirror_member(&ctx2, index, connector);
            }
        });
        members_group.add(&combo);
    }
    ctx.panel.append(&members_group);

    // ---- Settings ----
    let settings = group("Settings");

    // Primary.
    let primary_row = adw::SwitchRow::builder()
        .title("Primary Display")
        .subtitle("Shows the top bar and receives new windows by default")
        .build();
    primary_row.set_active(logical.primary);
    {
        let ctx2 = Rc::clone(ctx);
        primary_row.connect_active_notify(move |row| {
            if ctx2.state.rebuilding.get() {
                return;
            }
            if row.is_active() {
                if let Some(layout) = ctx2.state.edited.borrow_mut().as_mut() {
                    layout.set_primary(index);
                }
            } else {
                // Exactly one primary is required; turning it off directly is
                // not meaningful — pick another display as primary instead.
                row.set_active(true);
                return;
            }
            ctx2.after_edit();
        });
    }
    settings.add(&primary_row);

    // Enabled (only offered when another display would remain active).
    if layout.logical_displays.len() > 1 {
        let enabled_row = adw::SwitchRow::builder()
            .title("Enabled")
            .subtitle("Turning this off disables the display")
            .build();
        enabled_row.set_active(true);
        let ctx2 = Rc::clone(ctx);
        enabled_row.connect_active_notify(move |row| {
            if ctx2.state.rebuilding.get() || row.is_active() {
                return;
            }
            disable_logical(&ctx2, index);
        });
        settings.add(&enabled_row);
    }

    // Resolution.
    let resolutions: Vec<(i32, i32)> = if logical.is_mirror_group() {
        let members: Vec<&display_core::PhysicalMonitor> = logical
            .monitors
            .iter()
            .filter_map(|a| state.monitor(&a.connector))
            .collect();
        match mirror::mirror_candidates(&members) {
            Ok(candidates) => candidates.iter().map(|c| (c.width, c.height)).collect(),
            Err(_) => Vec::new(),
        }
    } else {
        logical
            .monitors
            .first()
            .and_then(|a| state.monitor(&a.connector))
            .map(|m| m.resolutions())
            .unwrap_or_default()
    };
    let current_size = logical.mode_size(&state.monitors);
    if !resolutions.is_empty() {
        let labels: Vec<String> = resolutions
            .iter()
            .map(
                |(w, h)| match display_core::mode::aspect_ratio_label(*w, *h) {
                    Some(ratio) => format!("{w} × {h} ({ratio})"),
                    None => format!("{w} × {h}"),
                },
            )
            .collect();
        let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let combo = adw::ComboRow::builder().title("Resolution").build();
        combo.set_model(Some(&gtk::StringList::new(&label_refs)));
        if let Some(pos) = resolutions.iter().position(|r| Some(*r) == current_size) {
            combo.set_selected(pos as u32);
        }
        let ctx2 = Rc::clone(ctx);
        let resolutions2 = resolutions.clone();
        combo.connect_selected_notify(move |combo| {
            if ctx2.state.rebuilding.get() {
                return;
            }
            if let Some((w, h)) = resolutions2.get(combo.selected() as usize) {
                set_resolution(&ctx2, index, *w, *h);
            }
        });
        settings.add(&combo);
    }

    // Refresh rate — per member; GNOME Settings hides it for mirror groups,
    // we show it per member inside the group details instead.
    if !logical.is_mirror_group()
        && let (Some(assignment), Some((w, h))) = (logical.monitors.first(), current_size)
        && let Some(monitor) = state.monitor(&assignment.connector)
    {
        let modes = monitor.refresh_rates_at(w, h);
        if modes.len() > 1 {
            let labels: Vec<String> = modes
                .iter()
                .map(|m| {
                    if m.refresh_rate_mode.as_deref() == Some("variable") {
                        format!("Variable (up to {})", format_refresh(m.refresh_hz))
                    } else {
                        format_refresh(m.refresh_hz)
                    }
                })
                .collect();
            let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
            let combo = adw::ComboRow::builder().title("Refresh Rate").build();
            combo.set_model(Some(&gtk::StringList::new(&label_refs)));
            if let Some(pos) = modes.iter().position(|m| m.id == assignment.mode_id) {
                combo.set_selected(pos as u32);
            }
            let mode_ids: Vec<String> = modes.iter().map(|m| m.id.clone()).collect();
            let connector = assignment.connector.clone();
            let ctx2 = Rc::clone(ctx);
            combo.connect_selected_notify(move |combo| {
                if ctx2.state.rebuilding.get() {
                    return;
                }
                if let Some(mode_id) = mode_ids.get(combo.selected() as usize) {
                    set_mode(&ctx2, index, &connector, mode_id);
                }
            });
            settings.add(&combo);
        }
    }

    // Scale — intersection of the members' supported scales.
    let scales: Vec<f64> = {
        let mut scales: Option<Vec<f64>> = None;
        for assignment in &logical.monitors {
            if let Some(mode) = state
                .monitor(&assignment.connector)
                .and_then(|m| m.find_mode(&assignment.mode_id))
            {
                scales = Some(match scales {
                    None => mode.supported_scales.clone(),
                    Some(mut acc) => {
                        acc.retain(|s| mode.supports_scale(*s));
                        acc
                    }
                });
            }
        }
        let mut scales = scales.unwrap_or_default();
        scales.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        scales
    };
    if scales.len() > 1 {
        let labels: Vec<String> = scales.iter().map(|s| format_scale_percent(*s)).collect();
        let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let combo = adw::ComboRow::builder().title("Scale").build();
        combo.set_model(Some(&gtk::StringList::new(&label_refs)));
        if let Some(pos) = scales
            .iter()
            .position(|s| (s - logical.scale).abs() < display_core::SCALE_EPS)
        {
            combo.set_selected(pos as u32);
        }
        let ctx2 = Rc::clone(ctx);
        let scales2 = scales.clone();
        combo.connect_selected_notify(move |combo| {
            if ctx2.state.rebuilding.get() {
                return;
            }
            if let Some(scale) = scales2.get(combo.selected() as usize) {
                if let Some(layout) = ctx2.state.edited.borrow_mut().as_mut()
                    && let Some(logical) = layout.logical_displays.get_mut(index)
                {
                    logical.scale = *scale;
                }
                ctx2.after_edit();
            }
        });
        settings.add(&combo);
    }

    // Orientation.
    {
        let labels: Vec<&str> = ORIENTATIONS.iter().map(|(_, l)| *l).collect();
        let combo = adw::ComboRow::builder().title("Orientation").build();
        combo.set_model(Some(&gtk::StringList::new(&labels)));
        if let Some(pos) = ORIENTATIONS
            .iter()
            .position(|(t, _)| *t == logical.transform)
        {
            combo.set_selected(pos as u32);
        }
        let ctx2 = Rc::clone(ctx);
        combo.connect_selected_notify(move |combo| {
            if ctx2.state.rebuilding.get() {
                return;
            }
            if let Some((transform, _)) = ORIENTATIONS.get(combo.selected() as usize) {
                if let Some(layout) = ctx2.state.edited.borrow_mut().as_mut()
                    && let Some(logical) = layout.logical_displays.get_mut(index)
                {
                    logical.transform = *transform;
                }
                ctx2.after_edit();
            }
        });
        settings.add(&combo);
    }
    ctx.panel.append(&settings);

    // ---- Identity / technical details ----
    let details = group("Details");
    for assignment in &logical.monitors {
        let Some(monitor) = state.monitor(&assignment.connector) else {
            continue;
        };
        let expander = adw::ExpanderRow::builder()
            .title(ctx.state.name_of(&assignment.connector))
            .subtitle(&assignment.connector)
            .build();

        let alias_row = adw::EntryRow::builder().title("Friendly Name").build();
        let key = monitor.identity.stable_key();
        if let Some(alias) = ctx
            .state
            .prefs
            .borrow()
            .monitor(&key)
            .and_then(|p| p.alias.clone())
        {
            alias_row.set_text(&alias);
        }
        {
            let ctx2 = Rc::clone(ctx);
            let key = key.clone();
            alias_row.connect_apply(move |row| {
                let text = row.text().to_string();
                ctx2.state
                    .prefs
                    .borrow_mut()
                    .set_alias(&key, (!text.trim().is_empty()).then_some(text));
                ctx2.state.save_prefs();
                ctx2.after_edit();
            });
        }
        alias_row.set_show_apply_button(true);
        expander.add_row(&alias_row);

        let kvm_row = adw::SwitchRow::builder()
            .title("This Is the KVM Display")
            .subtitle("Marks the display as a KVM / remote-management device")
            .build();
        kvm_row.set_active(ctx.state.prefs.borrow().is_kvm(monitor));
        {
            let ctx2 = Rc::clone(ctx);
            let key = key.clone();
            kvm_row.connect_active_notify(move |row| {
                if ctx2.state.rebuilding.get() {
                    return;
                }
                ctx2.state
                    .prefs
                    .borrow_mut()
                    .set_kvm_override(&key, Some(row.is_active()));
                ctx2.state.save_prefs();
                ctx2.canvas.refresh();
            });
        }
        expander.add_row(&kvm_row);

        let mut facts: Vec<(String, String)> = vec![
            ("Connector".into(), monitor.identity.connector.clone()),
            ("Vendor".into(), or_dash(&monitor.identity.vendor)),
            ("Product".into(), or_dash(&monitor.identity.product)),
            ("Serial".into(), or_dash(&monitor.identity.serial)),
        ];
        if let Some((w, h)) = monitor.physical_size_mm {
            facts.push(("Physical size".into(), format!("{w} × {h} mm")));
        }
        if let Some(mode) = monitor.find_mode(&assignment.mode_id) {
            facts.push(("Current mode".into(), mode.id.clone()));
        }
        facts.push(("Capabilities".into(), {
            let mut caps = Vec::new();
            if monitor.supports_hdr() {
                caps.push("HDR");
            }
            if monitor.supports_vrr() {
                caps.push("VRR");
            }
            if monitor.supports_underscanning {
                caps.push("Underscan");
            }
            if caps.is_empty() {
                "—".into()
            } else {
                caps.join(", ")
            }
        }));
        for (title, value) in facts {
            let row = adw::ActionRow::builder()
                .title(&title)
                .subtitle(&value)
                .build();
            row.add_css_class("property");
            expander.add_row(&row);
        }
        details.add(&expander);
    }
    ctx.panel.append(&details);
}

fn or_dash(s: &str) -> String {
    if s.is_empty() {
        "—".into()
    } else {
        s.to_owned()
    }
}

fn build_disabled_section(
    ctx: &Rc<UiCtx>,
    state: &display_core::DisplayState,
    layout: &display_core::DisplayLayout,
) {
    let disabled = layout.disabled_monitors(&state.monitors);
    if disabled.is_empty() {
        return;
    }
    let section = group("Disabled Displays");
    for monitor in disabled {
        let row = adw::ActionRow::builder()
            .title(ctx.state.name_of(monitor.connector()))
            .subtitle(monitor.connector())
            .build();
        let enable = gtk::Button::with_label("Enable");
        enable.set_valign(gtk::Align::Center);
        let connector = monitor.connector().to_owned();
        let ctx2 = Rc::clone(ctx);
        enable.connect_clicked(move |_| enable_monitor(&ctx2, &connector));
        row.add_suffix(&enable);
        section.add(&row);
    }
    ctx.panel.append(&section);
}

// ---- Edit operations (mutate the edited layout, then notify) ----

fn split_member(ctx: &Rc<UiCtx>, index: usize, member_index: usize) {
    let result = {
        let current = ctx.state.current.borrow();
        let Some(state) = current.as_ref() else {
            return;
        };
        let mut edited = ctx.state.edited.borrow_mut();
        let Some(layout) = edited.as_mut() else {
            return;
        };
        let Some(connector) = layout
            .logical_displays
            .get(index)
            .and_then(|l| l.monitors.get(member_index))
            .map(|a| a.connector.clone())
        else {
            return;
        };
        display_core::layout_ops::split_from_mirror(state, layout, &connector)
    };
    match result {
        Ok(()) => ctx.after_structural_edit(),
        Err(e) => ctx.toast(&e.to_string()),
    }
}

fn add_mirror_member(ctx: &Rc<UiCtx>, index: usize, connector: &str) {
    let result = {
        let current = ctx.state.current.borrow();
        let Some(state) = current.as_ref() else {
            return;
        };
        let mut edited = ctx.state.edited.borrow_mut();
        let Some(layout) = edited.as_mut() else {
            return;
        };
        let Some(anchor) = layout
            .logical_displays
            .get(index)
            .and_then(|l| l.monitors.first())
            .map(|a| a.connector.clone())
        else {
            return;
        };
        display_core::layout_ops::merge_into_mirror(state, layout, &anchor, connector)
            .map(|()| anchor)
    };
    match result {
        Ok(anchor) => {
            // Keep the merged group selected.
            let new_index = ctx
                .state
                .edited
                .borrow()
                .as_ref()
                .and_then(|l| l.group_of(&anchor).map(|(i, _)| i));
            ctx.state.selected.set(new_index);
            ctx.after_structural_edit();
            ctx.toast("Displays mirrored — press Apply to make it live");
        }
        Err(e) => ctx.toast(&e.to_string()),
    }
}

fn disable_logical(ctx: &Rc<UiCtx>, index: usize) {
    {
        let current = ctx.state.current.borrow();
        let Some(state) = current.as_ref() else {
            return;
        };
        let mut edited = ctx.state.edited.borrow_mut();
        let Some(layout) = edited.as_mut() else {
            return;
        };
        if index >= layout.logical_displays.len() || layout.logical_displays.len() < 2 {
            return;
        }
        let removed = layout.logical_displays.remove(index);
        if removed.primary
            && let Some(first) = layout.logical_displays.first_mut()
        {
            first.primary = true;
        }
        validation::normalize(layout, &state.monitors);
    }
    ctx.state.selected.set(None);
    ctx.after_structural_edit();
}

fn enable_monitor(ctx: &Rc<UiCtx>, connector: &str) {
    {
        let current = ctx.state.current.borrow();
        let Some(state) = current.as_ref() else {
            return;
        };
        let Some(monitor) = state.monitor(connector) else {
            return;
        };
        let Some(mode) = monitor.preferred_mode().or_else(|| monitor.modes.first()) else {
            return;
        };
        let mut edited = ctx.state.edited.borrow_mut();
        let Some(layout) = edited.as_mut() else {
            return;
        };
        let right_edge = layout
            .logical_displays
            .iter()
            .filter_map(|l| l.rect(&state.monitors, layout.layout_mode))
            .map(|r| r.right())
            .max()
            .unwrap_or(0);
        layout.logical_displays.push(LogicalDisplay {
            x: right_edge,
            y: 0,
            scale: mode.preferred_scale,
            transform: Transform::Normal,
            primary: layout.logical_displays.is_empty(),
            monitors: vec![MonitorAssignment {
                connector: connector.to_owned(),
                mode_id: mode.id.clone(),
                color_mode: None,
                rgb_range: None,
                underscanning: None,
            }],
        });
        validation::normalize(layout, &state.monitors);
    }
    ctx.after_structural_edit();
}

fn set_resolution(ctx: &Rc<UiCtx>, index: usize, w: i32, h: i32) {
    {
        let current = ctx.state.current.borrow();
        let Some(state) = current.as_ref() else {
            return;
        };
        let mut edited = ctx.state.edited.borrow_mut();
        let Some(layout) = edited.as_mut() else {
            return;
        };
        let Some(logical) = layout.logical_displays.get_mut(index) else {
            return;
        };
        for assignment in &mut logical.monitors {
            if let Some(mode) = state
                .monitor(&assignment.connector)
                .and_then(|m| m.best_mode_at(w, h))
            {
                assignment.mode_id = mode.id.clone();
            }
        }
        // Keep the scale valid for the new modes.
        let all_support = logical.monitors.iter().all(|a| {
            state
                .monitor(&a.connector)
                .and_then(|m| m.find_mode(&a.mode_id))
                .is_some_and(|mode| mode.supports_scale(logical.scale))
        });
        if !all_support {
            let fallback = logical
                .monitors
                .first()
                .and_then(|a| {
                    state
                        .monitor(&a.connector)
                        .and_then(|m| m.find_mode(&a.mode_id))
                })
                .map(|mode| {
                    mode.closest_supported_scale(logical.scale)
                        .unwrap_or(mode.preferred_scale)
                });
            if let Some(scale) = fallback {
                logical.scale = scale;
            }
        }
        validation::normalize(layout, &state.monitors);
    }
    ctx.after_structural_edit();
}

fn set_mode(ctx: &Rc<UiCtx>, index: usize, connector: &str, mode_id: &str) {
    {
        let mut edited = ctx.state.edited.borrow_mut();
        let Some(layout) = edited.as_mut() else {
            return;
        };
        let Some(logical) = layout.logical_displays.get_mut(index) else {
            return;
        };
        for assignment in &mut logical.monitors {
            if assignment.connector == connector {
                assignment.mode_id = mode_id.to_owned();
            }
        }
    }
    ctx.after_edit();
}

/// Deferred rebuild used after edits that restructure the panel itself,
/// mirroring GNOME Settings' idle-rebuild pattern to avoid signal loops.
pub fn queue_rebuild(ctx: &Rc<UiCtx>) {
    let ctx = Rc::clone(ctx);
    glib::idle_add_local_once(move || {
        rebuild(&ctx);
    });
}
