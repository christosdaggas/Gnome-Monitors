//! The visual layout canvas: monitor-shaped cards, drag & snap.
//!
//! Pure presentation: geometry (snapping, settling, adjacency) lives in
//! `display_core::snap` where it is unit-tested. The canvas converts pointer
//! deltas to layout coordinates and draws the edited layout.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use display_core::Rect;
use display_core::snap::{self, SnapGuide};
use display_core::validation;
use gtk::prelude::*;
use gtk::{cairo, glib};

use crate::ui_state::SharedState;

const CARD_RADIUS: f64 = 8.0;
const SNAP_THRESHOLD_PX: f64 = 25.0;

pub struct Canvas {
    pub widget: gtk::DrawingArea,
    state: SharedState,
    /// (zoom, offset_x, offset_y): layout → widget transform.
    view: Cell<(f64, f64, f64)>,
    drag: RefCell<Option<DragState>>,
    guides: RefCell<Vec<SnapGuide>>,
    on_changed: RefCell<Option<Box<dyn Fn()>>>,
}

struct DragState {
    index: usize,
    start_rect: Rect,
    moved: bool,
}

impl Canvas {
    pub fn new(state: SharedState) -> Rc<Canvas> {
        let widget = gtk::DrawingArea::new();
        widget.set_hexpand(true);
        widget.set_vexpand(true);
        widget.set_content_width(420);
        widget.set_content_height(280);
        widget.set_focusable(true);

        let canvas = Rc::new(Canvas {
            widget,
            state,
            view: Cell::new((0.1, 0.0, 0.0)),
            drag: RefCell::new(None),
            guides: RefCell::new(Vec::new()),
            on_changed: RefCell::new(None),
        });

        canvas.widget.set_draw_func({
            let canvas = Rc::downgrade(&canvas);
            move |_, cr, w, h| {
                if let Some(canvas) = canvas.upgrade() {
                    canvas.draw(cr, f64::from(w), f64::from(h));
                }
            }
        });

        let click = gtk::GestureClick::new();
        click.connect_pressed({
            let canvas = Rc::downgrade(&canvas);
            move |_, _, x, y| {
                if let Some(canvas) = canvas.upgrade() {
                    canvas.on_press(x, y);
                }
            }
        });
        canvas.widget.add_controller(click);

        let drag = gtk::GestureDrag::new();
        drag.connect_drag_begin({
            let canvas = Rc::downgrade(&canvas);
            move |_, x, y| {
                if let Some(canvas) = canvas.upgrade() {
                    canvas.on_drag_begin(x, y);
                }
            }
        });
        drag.connect_drag_update({
            let canvas = Rc::downgrade(&canvas);
            move |_, dx, dy| {
                if let Some(canvas) = canvas.upgrade() {
                    canvas.on_drag_update(dx, dy);
                }
            }
        });
        drag.connect_drag_end({
            let canvas = Rc::downgrade(&canvas);
            move |_, _, _| {
                if let Some(canvas) = canvas.upgrade() {
                    canvas.on_drag_end();
                }
            }
        });
        canvas.widget.add_controller(drag);

        // Keyboard: arrows move the selected display by 8 logical px
        // (Shift: 64), Enter/Space keeps GTK activation defaults.
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed({
            let canvas = Rc::downgrade(&canvas);
            move |_, key, _, modifier| {
                let Some(canvas) = canvas.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                let step = if modifier.contains(gtk::gdk::ModifierType::SHIFT_MASK) {
                    64
                } else {
                    8
                };
                let (dx, dy) = match key {
                    gtk::gdk::Key::Left => (-step, 0),
                    gtk::gdk::Key::Right => (step, 0),
                    gtk::gdk::Key::Up => (0, -step),
                    gtk::gdk::Key::Down => (0, step),
                    gtk::gdk::Key::Tab | gtk::gdk::Key::ISO_Left_Tab => {
                        let backwards = key == gtk::gdk::Key::ISO_Left_Tab
                            || modifier.contains(gtk::gdk::ModifierType::SHIFT_MASK);
                        canvas.cycle_selection(backwards);
                        return glib::Propagation::Stop;
                    }
                    _ => return glib::Propagation::Proceed,
                };
                canvas.nudge_selected(dx, dy);
                glib::Propagation::Stop
            }
        });
        canvas.widget.add_controller(keys);

        canvas
    }

    pub fn set_on_changed(&self, f: impl Fn() + 'static) {
        *self.on_changed.borrow_mut() = Some(Box::new(f));
    }

    fn emit_changed(&self) {
        if let Some(f) = self.on_changed.borrow().as_ref() {
            f();
        }
    }

    pub fn refresh(&self) {
        self.update_accessible_label();
        self.widget.queue_draw();
    }

    /// Moves the selection to the next/previous display (Tab / Shift+Tab),
    /// so the canvas is usable without a pointer.
    pub fn cycle_selection(&self, backwards: bool) {
        let count = self
            .state
            .edited
            .borrow()
            .as_ref()
            .map_or(0, |l| l.logical_displays.len());
        if count == 0 {
            return;
        }
        let next = match self.state.selected.get() {
            None => {
                if backwards {
                    count - 1
                } else {
                    0
                }
            }
            Some(current) => {
                if backwards {
                    (current + count - 1) % count
                } else {
                    (current + 1) % count
                }
            }
        };
        self.state.selected.set(Some(next));
        self.refresh();
        self.emit_changed();
    }

    /// Describes the arrangement and selection for assistive technology
    /// (the cards themselves are canvas drawings, so the description carries
    /// the information Orca cannot pick up from them).
    fn update_accessible_label(&self) {
        let description = {
            let edited = self.state.edited.borrow();
            let Some(layout) = edited.as_ref() else {
                return;
            };
            let selected = self.state.selected.get();
            let parts: Vec<String> = layout
                .logical_displays
                .iter()
                .enumerate()
                .map(|(i, logical)| {
                    let names: Vec<String> = logical
                        .monitors
                        .iter()
                        .map(|a| self.state.name_of(&a.connector))
                        .collect();
                    let mut part = format!("Display {}: {}", i + 1, names.join(" mirrored with "));
                    if logical.primary {
                        part.push_str(", primary");
                    }
                    if selected == Some(i) {
                        part.push_str(", selected");
                    }
                    part
                })
                .collect();
            format!(
                "Monitor arrangement with {} displays. {}. Use Tab to cycle displays and arrow keys to move the selected display.",
                layout.logical_displays.len(),
                parts.join(". ")
            )
        };
        self.widget
            .update_property(&[gtk::accessible::Property::Label(&description)]);
    }

    /// Rectangles of the edited layout in layout coordinates.
    fn layout_rects(&self) -> Vec<Rect> {
        let current = self.state.current.borrow();
        let edited = self.state.edited.borrow();
        let (Some(state), Some(layout)) = (current.as_ref(), edited.as_ref()) else {
            return Vec::new();
        };
        layout
            .logical_displays
            .iter()
            .map(|l| {
                l.rect(&state.monitors, layout.layout_mode)
                    .unwrap_or(Rect::new(l.x, l.y, 640, 360))
            })
            .collect()
    }

    fn update_view(&self, w: f64, h: f64) {
        let rects = self.layout_rects();
        let Some(bb) = display_core::geometry::bounding_box(rects.iter()) else {
            return;
        };
        let margin = 48.0;
        let zoom_x = (w - 2.0 * margin) / f64::from(bb.width.max(1));
        let zoom_y = (h - 2.0 * margin) / f64::from(bb.height.max(1));
        let zoom = zoom_x.min(zoom_y).min(0.35);
        let off_x = (w - f64::from(bb.width) * zoom) / 2.0 - f64::from(bb.x) * zoom;
        let off_y = (h - f64::from(bb.height) * zoom) / 2.0 - f64::from(bb.y) * zoom;
        self.view.set((zoom, off_x, off_y));
    }

    fn to_widget(&self, r: &Rect) -> (f64, f64, f64, f64) {
        let (zoom, ox, oy) = self.view.get();
        (
            f64::from(r.x) * zoom + ox,
            f64::from(r.y) * zoom + oy,
            f64::from(r.width) * zoom,
            f64::from(r.height) * zoom,
        )
    }

    fn hit_test(&self, x: f64, y: f64) -> Option<usize> {
        let rects = self.layout_rects();
        // Iterate topmost-last like drawing order; prefer the selected one.
        let mut hit = None;
        for (i, r) in rects.iter().enumerate() {
            let (wx, wy, ww, wh) = self.to_widget(r);
            if x >= wx && x < wx + ww && y >= wy && y < wy + wh {
                hit = Some(i);
            }
        }
        hit
    }

    fn on_press(&self, x: f64, y: f64) {
        self.widget.grab_focus();
        let hit = self.hit_test(x, y);
        if self.state.selected.get() != hit {
            self.state.selected.set(hit);
            self.refresh();
            self.emit_changed();
        }
    }

    fn on_drag_begin(&self, x: f64, y: f64) {
        let Some(index) = self.hit_test(x, y) else {
            return;
        };
        self.state.selected.set(Some(index));
        let rects = self.layout_rects();
        if let Some(rect) = rects.get(index) {
            *self.drag.borrow_mut() = Some(DragState {
                index,
                start_rect: *rect,
                moved: false,
            });
        }
        self.refresh();
        self.emit_changed();
    }

    fn on_drag_update(&self, dx: f64, dy: f64) {
        let (zoom, ..) = self.view.get();
        if zoom <= 0.0 {
            return;
        }
        let mut drag = self.drag.borrow_mut();
        let Some(drag) = drag.as_mut() else {
            return;
        };
        drag.moved = true;
        #[allow(clippy::cast_possible_truncation)]
        let (ldx, ldy) = ((dx / zoom).round() as i32, (dy / zoom).round() as i32);
        let moving = drag.start_rect.translated(ldx, ldy);

        let others: Vec<Rect> = self
            .layout_rects()
            .into_iter()
            .enumerate()
            .filter(|(i, _)| *i != drag.index)
            .map(|(_, r)| r)
            .collect();
        #[allow(clippy::cast_possible_truncation)]
        let threshold = (SNAP_THRESHOLD_PX / zoom).round() as i32;
        let snapped = snap::snap_rect(moving, &others, threshold.max(1));
        *self.guides.borrow_mut() = snapped.guides.clone();

        let index = drag.index;
        if let Some(layout) = self.state.edited.borrow_mut().as_mut()
            && let Some(logical) = layout.logical_displays.get_mut(index)
        {
            logical.x = snapped.x;
            logical.y = snapped.y;
        }
        self.widget.queue_draw();
    }

    fn on_drag_end(&self) {
        let Some(drag) = self.drag.borrow_mut().take() else {
            return;
        };
        self.guides.borrow_mut().clear();
        if drag.moved {
            self.settle(drag.index);
            self.emit_changed();
        }
        self.widget.queue_draw();
    }

    fn nudge_selected(&self, dx: i32, dy: i32) {
        let Some(index) = self.state.selected.get() else {
            return;
        };
        if let Some(layout) = self.state.edited.borrow_mut().as_mut()
            && let Some(logical) = layout.logical_displays.get_mut(index)
        {
            logical.x += dx;
            logical.y += dy;
        }
        self.settle(index);
        self.emit_changed();
        self.widget.queue_draw();
    }

    /// After a drop: close gaps/overlaps and re-anchor the layout at (0,0).
    fn settle(&self, index: usize) {
        let rects = self.layout_rects();
        let Some(moving) = rects.get(index).copied() else {
            return;
        };
        let others: Vec<Rect> = rects
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != index)
            .map(|(_, r)| *r)
            .collect();
        let settled = snap::settle_rect(moving, &others);
        let current = self.state.current.borrow();
        if let (Some(state), Some(layout)) =
            (current.as_ref(), self.state.edited.borrow_mut().as_mut())
        {
            if let Some(logical) = layout.logical_displays.get_mut(index) {
                logical.x = settled.x;
                logical.y = settled.y;
            }
            validation::normalize(layout, &state.monitors);
        }
    }

    #[allow(clippy::too_many_lines)]
    fn draw(&self, cr: &cairo::Context, w: f64, h: f64) {
        self.update_view(w, h);

        let style_dark = adw::StyleManager::default().is_dark();
        let fg = if style_dark {
            (1.0, 1.0, 1.0)
        } else {
            (0.0, 0.0, 0.0)
        };
        let accent = adw::StyleManager::default().accent_color_rgba();
        let accent = (
            f64::from(accent.red()),
            f64::from(accent.green()),
            f64::from(accent.blue()),
        );

        let current = self.state.current.borrow();
        let edited = self.state.edited.borrow();
        let (Some(state), Some(layout)) = (current.as_ref(), edited.as_ref()) else {
            return;
        };

        let selected = self.state.selected.get();
        let numbers = {
            // Position-based numbering, consistent with the Shell labels.
            let mut order: Vec<usize> = (0..layout.logical_displays.len()).collect();
            order.sort_by_key(|&i| {
                let l = &layout.logical_displays[i];
                (l.x, l.y)
            });
            let mut numbers = vec![0usize; order.len()];
            for (rank, index) in order.into_iter().enumerate() {
                numbers[index] = rank + 1;
            }
            numbers
        };
        for (i, logical) in layout.logical_displays.iter().enumerate() {
            let rect = logical
                .rect(&state.monitors, layout.layout_mode)
                .unwrap_or(Rect::new(logical.x, logical.y, 640, 360));
            let (x, y, rw, rh) = self.to_widget(&rect);
            let is_selected = selected == Some(i);

            // Card body.
            rounded_rect(cr, x, y, rw, rh, CARD_RADIUS);
            if is_selected {
                cr.set_source_rgba(accent.0, accent.1, accent.2, 0.25);
            } else {
                cr.set_source_rgba(fg.0, fg.1, fg.2, 0.08);
            }
            let _ = cr.fill_preserve();
            if is_selected {
                cr.set_source_rgba(accent.0, accent.1, accent.2, 0.9);
                cr.set_line_width(2.0);
            } else {
                cr.set_source_rgba(fg.0, fg.1, fg.2, 0.25);
                cr.set_line_width(1.0);
            }
            let _ = cr.stroke();

            // Labels.
            cr.set_source_rgba(fg.0, fg.1, fg.2, 0.95);
            let names: Vec<String> = logical
                .monitors
                .iter()
                .map(|a| {
                    let mut name = self.state.name_of(&a.connector);
                    if self.state.is_kvm(&a.connector) {
                        name.push_str("  (KVM)");
                    }
                    name
                })
                .collect();
            let (mode_w, mode_h) = logical.mode_size(&state.monitors).unwrap_or((0, 0));
            let mut lines = vec![format!("{}", numbers.get(i).copied().unwrap_or(i + 1))];
            lines.extend(names);
            let mode_line = format!(
                "{}  ·  {} %",
                display_core::mode::format_resolution(mode_w, mode_h),
                {
                    #[allow(clippy::cast_possible_truncation)]
                    let pct = (logical.scale * 100.0) as i32;
                    pct
                }
            );
            lines.push(mode_line);
            let mut badges = Vec::new();
            if logical.primary {
                badges.push("★ Primary");
            }
            if logical.is_mirror_group() {
                badges.push("⧉ Mirrored");
            }
            if !badges.is_empty() {
                lines.push(badges.join("   "));
            }

            let layout_text = self.widget.create_pango_layout(Some(&lines.join("\n")));
            layout_text.set_width((rw.max(10.0) as i32 - 16) * gtk::pango::SCALE);
            layout_text.set_ellipsize(gtk::pango::EllipsizeMode::End);
            let (_, text_h) = layout_text.pixel_size();
            cr.move_to(x + 10.0, y + (rh - f64::from(text_h)).max(0.0) / 2.0);
            pangocairo::functions::show_layout(cr, &layout_text);
        }

        // Snap guides while dragging.
        let (zoom, ox, oy) = self.view.get();
        cr.set_source_rgba(accent.0, accent.1, accent.2, 0.6);
        cr.set_line_width(1.0);
        cr.set_dash(&[4.0, 4.0], 0.0);
        for guide in self.guides.borrow().iter() {
            match guide.axis {
                display_core::snap::Axis::Vertical => {
                    let gx = f64::from(guide.position) * zoom + ox;
                    cr.move_to(gx, 0.0);
                    cr.line_to(gx, h);
                }
                display_core::snap::Axis::Horizontal => {
                    let gy = f64::from(guide.position) * zoom + oy;
                    cr.move_to(0.0, gy);
                    cr.line_to(w, gy);
                }
            }
            let _ = cr.stroke();
        }
        cr.set_dash(&[], 0.0);
    }
}

fn rounded_rect(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(
        x + r,
        y + h - r,
        r,
        std::f64::consts::FRAC_PI_2,
        std::f64::consts::PI,
    );
    cr.arc(
        x + r,
        y + r,
        r,
        std::f64::consts::PI,
        1.5 * std::f64::consts::PI,
    );
    cr.close_path();
}
