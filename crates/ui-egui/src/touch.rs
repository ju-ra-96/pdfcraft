//! Touch and Apple Pencil input (PdfCraft for iPad).
//!
//! egui-winit turns the first touch on screen into a mouse: it presses where the finger lands,
//! drags with it and releases where it lifts. On a tablet that makes every finger a pen. Scrolling
//! a page drags out a selection, a resting palm draws, and a second finger is ignored. So in
//! tablet mode, [`TouchRouter`] rewrites the frame's input before egui sees it
//! ([`crate::PdfCraftApp`]'s `raw_input_hook`). It drops egui-winit's emulated pointer events and
//! makes its own from the touches:
//!
//! - **The Apple Pencil is the pointer.** It presses, drags and releases like a mouse button, so
//!   every tool works with it as on the desktop: Draw, highlight, select text, move comments. Its
//!   pressure is kept for the stroke ([`TouchRouter::pencil_pressure`]). While it is down,
//!   fingers and palms are ignored (palm rejection).
//! - **Fingers on the page navigate.** One or two fingers scroll, as wheel events, and keep
//!   gliding after a flick. Pinching zooms around the fingers, as `Event::Zoom` with the pointer
//!   hovering between them. A tap is a click (links, form fields, choosing a comment), and a long
//!   press is a right-click (context menus).
//! - **Fingers on the interface** (toolbars, panels, dialogs, menus) are the pointer, as before,
//!   so buttons, sliders and lists work.
//! - **"Draw with fingers too".** Until a Pencil has been used, or with the preference
//!   (Preferences ▸ Apple Pencil) turned off, one finger on the page drags like the Pencil once it
//!   moves; two fingers still scroll and zoom. That's how the Notes app behaves.
//!
//! The router can't tell a Pencil from a finger by touch type: egui-winit drops it. On iPads,
//! though, UIKit reports a force for Pencil touches only (no iPad has 3D Touch), and winit passes
//! it on. So a touch with a force is the Pencil (vendor/winit `platform_impl/ios/view.rs`).
//!
//! Positions are in egui points. The router is pure state over events, so it is tested without
//! a window.

use egui::{Event, Modifiers, MouseWheelUnit, PointerButton, Pos2, TouchId, TouchPhase, Vec2};

/// Which tablet the app runs on. Only the iPad so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tablet {
    IPad,
}

/// A finger that moves less than this (points) before lifting is a tap.
const TAP_SLOP: f32 = 10.0;
/// A finger held still this long (seconds) is a long press, which right-clicks.
const LONG_PRESS: f64 = 0.55;
/// A flick faster than this (points per second) keeps scrolling after the fingers lift.
const FLING_MIN_SPEED: f32 = 150.0;
/// The glide stops below this speed.
const FLING_STOP_SPEED: f32 = 12.0;
/// How quickly the glide slows: velocity is multiplied by `exp(-FLING_DECAY * dt)`.
const FLING_DECAY: f32 = 4.0;
/// Contacts tracked at once; more (a whole hand on the glass) are ignored.
const MAX_CONTACTS: usize = 10;
/// Touch-friendly interface scale: egui's sizes are made for a mouse.
pub const TABLET_ZOOM: f32 = 1.15;
/// Minimum height of buttons and other controls in tablet mode, in points (before the zoom).
const TABLET_INTERACT_HEIGHT: f32 = 26.0;

/// What a touch is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Pencil,
    Finger,
}

/// What a contact is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    /// Driving the pointer (pressed at its start).
    Pointer,
    /// A finger on the page that hasn't moved far enough to be a pan or a stroke yet.
    Pending,
    /// Scrolling and zooming the page.
    Pan,
    /// A palm, an extra finger, or a long press already used: no effect until it lifts.
    Ignored,
}

#[derive(Clone, Copy, Debug)]
struct Contact {
    id: TouchId,
    tool: Tool,
    role: Role,
    start: Pos2,
    pos: Pos2,
    started_at: f64,
    force: Option<f32>,
}

/// What the router needs to know about the frame.
pub struct Frame<'a> {
    /// The time of this input, in seconds (`RawInput::time`).
    pub now: f64,
    /// Whether a touch landing at this position is on the page (not on the interface).
    pub on_page: &'a dyn Fn(Pos2) -> bool,
    /// Fingers on the page act like the Pencil once they move (see the module docs).
    pub fingers_draw: bool,
    pub modifiers: Modifiers,
}

/// Rewrites touches into pointer, scroll and zoom events (see the module docs).
#[derive(Debug, Default)]
pub struct TouchRouter {
    contacts: Vec<Contact>,
    /// The centroid and spread of the panning fingers at the last event, to measure motion.
    pan: Option<(Pos2, f32, usize)>,
    /// Scroll velocity (points per second) from recent pan motion, for the glide.
    velocity: Vec2,
    last_pan_at: Option<f64>,
    /// The glide's velocity, when it was last advanced, and where the pointer hovers meanwhile
    /// (the page scrolls only under the pointer).
    fling: Option<(Vec2, f64, Pos2)>,
    /// A Pencil has touched the screen (persisted: decides "Draw with fingers too").
    pub pencil_seen: bool,
}

impl TouchRouter {
    /// The pressure of the Pencil driving the pointer, 0–1, if a Pencil is down.
    pub fn pencil_pressure(&self) -> Option<f32> {
        self.contacts.iter().find(|c| c.role == Role::Pointer && c.tool == Tool::Pencil).and_then(|c| c.force)
    }

    /// Whether a Pencil is on the glass.
    pub fn pencil_down(&self) -> bool {
        self.contacts.iter().any(|c| c.tool == Tool::Pencil)
    }

    /// Rewrite one frame's events. Returns whether the router needs another frame soon (a
    /// glide in progress, or a long press to time), whatever the input.
    pub fn route(&mut self, events: &mut Vec<Event>, frame: &Frame) -> bool {
        let input = std::mem::take(events);
        let mut out = Vec::with_capacity(input.len() + 4);
        for event in input {
            match event {
                // egui-winit's mouse emulation of the first touch: replaced by ours.
                Event::PointerMoved(_) | Event::PointerButton { .. } | Event::PointerGone => {}
                Event::Touch { id, phase, pos, force, .. } => self.touch(id, phase, pos, force, frame, &mut out),
                other => out.push(other),
            }
        }
        self.pan_motion(frame, &mut out);
        let pressing = self.long_presses(frame, &mut out);
        let gliding = self.glide(frame, &mut out);
        *events = out;
        pressing || gliding
    }

    fn touch(&mut self, id: TouchId, phase: TouchPhase, pos: Pos2, force: Option<f32>, frame: &Frame, out: &mut Vec<Event>) {
        if !(pos.x.is_finite() && pos.y.is_finite()) {
            return;
        }
        let force = force.filter(|f| f.is_finite()).map(|f| f.clamp(0.0, 1.0));
        match phase {
            TouchPhase::Start => self.start(id, pos, force, frame, out),
            TouchPhase::Move => self.moved(id, pos, force, frame, out),
            TouchPhase::End => self.lift(id, pos, false, frame, out),
            TouchPhase::Cancel => self.lift(id, pos, true, frame, out),
        }
    }

    fn start(&mut self, id: TouchId, pos: Pos2, force: Option<f32>, frame: &Frame, out: &mut Vec<Event>) {
        // A repeated start (shouldn't happen) replaces the old contact.
        self.contacts.retain(|c| c.id != id);
        if self.contacts.len() >= MAX_CONTACTS {
            return;
        }
        self.fling = None;
        let tool = if force.is_some() { Tool::Pencil } else { Tool::Finger };
        let role = match tool {
            Tool::Pencil => {
                self.pencil_seen = true;
                // The Pencil wins: a finger that was about to pan or tap, or that drives the
                // pointer, stands down.
                self.release_pointer(out);
                for c in &mut self.contacts {
                    if c.role == Role::Pending || c.role == Role::Pointer {
                        c.role = Role::Ignored;
                    }
                }
                self.pan = None;
                Role::Pointer
            }
            Tool::Finger if self.pencil_down() => Role::Ignored,
            Tool::Finger if self.contacts.iter().any(|c| c.role == Role::Pointer) => Role::Ignored,
            Tool::Finger if (frame.on_page)(pos) => {
                // A second finger on the page: both scroll and zoom.
                if let Some(other) = self.contacts.iter_mut().find(|c| c.role == Role::Pending || c.role == Role::Pan) {
                    other.role = Role::Pan;
                    self.pan = None;
                    Role::Pan
                } else {
                    Role::Pending
                }
            }
            Tool::Finger => {
                if self.contacts.is_empty() {
                    Role::Pointer
                } else {
                    Role::Ignored
                }
            }
        };
        if role == Role::Pointer {
            press(pos, PointerButton::Primary, frame.modifiers, out);
        }
        self.contacts.push(Contact { id, tool, role, start: pos, pos, started_at: frame.now, force });
    }

    fn moved(&mut self, id: TouchId, pos: Pos2, force: Option<f32>, frame: &Frame, out: &mut Vec<Event>) {
        let Some(c) = self.contacts.iter_mut().find(|c| c.id == id) else { return };
        c.pos = pos;
        if c.tool == Tool::Pencil {
            c.force = force.or(c.force);
        }
        match c.role {
            Role::Pointer => out.push(Event::PointerMoved(pos)),
            Role::Pending if (pos - c.start).length() > TAP_SLOP => {
                if frame.fingers_draw {
                    // Drags like the Pencil, from where it landed.
                    c.role = Role::Pointer;
                    press(c.start, PointerButton::Primary, frame.modifiers, out);
                    out.push(Event::PointerMoved(pos));
                } else {
                    c.role = Role::Pan;
                    self.pan = None;
                }
            }
            Role::Pending | Role::Pan | Role::Ignored => {}
        }
    }

    fn lift(&mut self, id: TouchId, pos: Pos2, cancelled: bool, frame: &Frame, out: &mut Vec<Event>) {
        let Some(i) = self.contacts.iter().position(|c| c.id == id) else { return };
        let c = self.contacts.remove(i);
        match c.role {
            Role::Pointer => {
                if !cancelled {
                    out.push(Event::PointerMoved(pos));
                    out.push(Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: frame.modifiers });
                }
                out.push(Event::PointerGone);
            }
            Role::Pending if !cancelled => click(c.start, PointerButton::Primary, frame.modifiers, out),
            Role::Pan => {
                self.pan = None;
                let still_panning = self.contacts.iter().any(|c| c.role == Role::Pan);
                let recent = self.last_pan_at.is_some_and(|t| frame.now - t < 0.08);
                if !still_panning && recent && self.velocity.length() > FLING_MIN_SPEED {
                    self.fling = Some((self.velocity, frame.now, c.pos));
                } else if !still_panning {
                    out.push(Event::PointerGone);
                }
                if !still_panning {
                    self.velocity = Vec2::ZERO;
                }
            }
            Role::Pending | Role::Ignored => {}
        }
    }

    /// Release the pointer a finger holds (the Pencil is taking over).
    fn release_pointer(&mut self, out: &mut Vec<Event>) {
        if let Some(c) = self.contacts.iter().find(|c| c.role == Role::Pointer) {
            out.push(Event::PointerButton { pos: c.pos, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
            out.push(Event::PointerGone);
        }
    }

    /// Scroll by the motion of the panning fingers' centroid, and zoom by the change in their
    /// spread, since the last frame.
    fn pan_motion(&mut self, frame: &Frame, out: &mut Vec<Event>) {
        let fingers: Vec<Pos2> = self.contacts.iter().filter(|c| c.role == Role::Pan).map(|c| c.pos).collect();
        if fingers.is_empty() {
            self.pan = None;
            return;
        }
        let n = fingers.len();
        let centroid = fingers.iter().fold(Vec2::ZERO, |a, p| a + p.to_vec2()) / n as f32;
        let centroid = centroid.to_pos2();
        let spread = fingers.iter().map(|p| (*p - centroid).length()).sum::<f32>() / n as f32;
        if let Some((last, last_spread, last_n)) = self.pan
            && last_n == n
        {
            let delta = centroid - last;
            if delta != Vec2::ZERO {
                // Scroll areas only take the wheel under the pointer: hover it between the fingers.
                out.push(Event::PointerMoved(centroid));
                out.push(Event::MouseWheel { unit: MouseWheelUnit::Point, delta, phase: TouchPhase::Move, modifiers: Modifiers::NONE });
                if let Some(t) = self.last_pan_at {
                    let dt = (frame.now - t) as f32;
                    if dt > 0.0 && dt.is_finite() {
                        // Smoothed, so one jittery frame doesn't decide the glide.
                        self.velocity = self.velocity * 0.4 + (delta / dt) * 0.6;
                    }
                }
                self.last_pan_at = Some(frame.now);
            }
            if n >= 2 && last_spread > 4.0 && spread > 4.0 {
                let factor = spread / last_spread;
                if factor.is_finite() && (factor - 1.0).abs() > 1e-4 {
                    // The canvas zooms around the hovering pointer: put it between the fingers.
                    out.push(Event::PointerMoved(centroid));
                    out.push(Event::Zoom(factor));
                }
            }
        } else {
            self.last_pan_at = Some(frame.now);
        }
        self.pan = Some((centroid, spread, n));
    }

    /// Right-click under a finger held still on the page. Returns whether one is still being
    /// timed (so the caller asks for a frame when it is due).
    fn long_presses(&mut self, frame: &Frame, out: &mut Vec<Event>) -> bool {
        let mut timing = false;
        for c in &mut self.contacts {
            if c.role != Role::Pending {
                continue;
            }
            if frame.now - c.started_at >= LONG_PRESS {
                c.role = Role::Ignored;
                click(c.start, PointerButton::Secondary, frame.modifiers, out);
            } else {
                timing = true;
            }
        }
        timing
    }

    /// Keep scrolling after a flick, slowing down. Returns whether the glide goes on.
    fn glide(&mut self, frame: &Frame, out: &mut Vec<Event>) -> bool {
        let Some((v, at, pos)) = self.fling else { return false };
        let dt = ((frame.now - at) as f32).clamp(0.0, 0.1);
        let v = v * (-FLING_DECAY * dt).exp();
        if v.length() < FLING_STOP_SPEED || !v.x.is_finite() || !v.y.is_finite() {
            self.fling = None;
            out.push(Event::PointerGone);
            return false;
        }
        if dt > 0.0 {
            out.push(Event::PointerMoved(pos));
            out.push(Event::MouseWheel { unit: MouseWheelUnit::Point, delta: v * dt, phase: TouchPhase::Move, modifiers: Modifiers::NONE });
        }
        self.fling = Some((v, frame.now, pos));
        true
    }
}

fn press(pos: Pos2, button: PointerButton, modifiers: Modifiers, out: &mut Vec<Event>) {
    out.push(Event::PointerMoved(pos));
    out.push(Event::PointerButton { pos, button, pressed: true, modifiers });
}

/// Press and release in one frame: egui reports it as a click.
fn click(pos: Pos2, button: PointerButton, modifiers: Modifiers, out: &mut Vec<Event>) {
    press(pos, button, modifiers, out);
    out.push(Event::PointerButton { pos, button, pressed: false, modifiers });
    out.push(Event::PointerGone);
}

impl crate::PdfCraftApp {
    /// Tablet mode: touch and Apple Pencil input, touch-sized controls. Call before
    /// [`Self::restore`], which then restores whether a Pencil has been used.
    pub fn enable_tablet(&mut self, _tablet: Tablet) {
        if self.touch.is_none() {
            self.touch = Some(TouchRouter::default());
        }
    }

    /// Whether only the Pencil draws on the page, fingers scrolling and zooming (see the module
    /// docs); automatic until the user chooses.
    pub fn pencil_only_draws(&self) -> bool {
        self.pencil_only.unwrap_or_else(|| self.touch.as_ref().is_some_and(|t| t.pencil_seen))
    }

    /// The pressure of the Pencil drawing right now, 0–1 (tablet mode).
    pub fn pencil_pressure(&self) -> Option<f32> {
        self.touch.as_ref().and_then(TouchRouter::pencil_pressure)
    }

    /// Rewrite the frame's touches (tablet mode only; called from `raw_input_hook`).
    pub(crate) fn route_touches(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        if self.touch.is_none() {
            return;
        }
        let fingers_draw = !self.pencil_only_draws();
        let page = self.touch_page;
        // On the page, and not under a window, menu or popup drawn over it (last frame's layers).
        let on_page = |p: Pos2| page.is_some_and(|r| r.contains(p)) && ctx.layer_id_at(p).is_none_or(|l| l.order == egui::Order::Background);
        let frame = Frame { now: raw.time.unwrap_or(0.0), on_page: &on_page, fingers_draw, modifiers: ctx.input(|i| i.modifiers) };
        let Some(router) = self.touch.as_mut() else { return };
        if router.route(&mut raw.events, &frame) {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }
}

/// The height of a list row the user taps (larger in tablet mode).
pub fn row_height(ctx: &egui::Context) -> f32 {
    (ctx.global_style().spacing.interact_size.y + 12.0).max(30.0)
}

/// Make the interface touch-sized: larger controls and a slightly larger scale. Cheap to call
/// every frame (it changes nothing once applied), which also keeps it applied after a theme
/// change resets the style.
pub fn ensure_tablet_style(ctx: &egui::Context) {
    if ctx.global_style().spacing.interact_size.y < TABLET_INTERACT_HEIGHT {
        ctx.global_style_mut(|s| {
            s.spacing.interact_size.y = TABLET_INTERACT_HEIGHT;
            s.spacing.button_padding = egui::vec2(12.0, 7.0);
            s.spacing.item_spacing = egui::vec2(10.0, 8.0);
            s.spacing.scroll.bar_width = 10.0;
            // A finger's drag starts later than a mouse's.
            s.interaction.interact_radius = 8.0;
        });
    }
    if (ctx.zoom_factor() - TABLET_ZOOM).abs() > f32::EPSILON {
        ctx.set_zoom_factor(TABLET_ZOOM);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: egui::Rect = egui::Rect { min: egui::pos2(100.0, 100.0), max: egui::pos2(900.0, 1100.0) };

    fn on_page(p: Pos2) -> bool {
        PAGE.contains(p)
    }

    fn frame(now: f64, fingers_draw: bool) -> Frame<'static> {
        Frame { now, on_page: &on_page, fingers_draw, modifiers: Modifiers::NONE }
    }

    fn touch(id: u64, phase: TouchPhase, x: f32, y: f32, force: Option<f32>) -> Event {
        Event::Touch { device_id: egui::TouchDeviceId(1), id: TouchId(id), phase, pos: egui::pos2(x, y), force }
    }

    fn run(r: &mut TouchRouter, events: Vec<Event>, f: &Frame) -> Vec<Event> {
        let mut events = events;
        r.route(&mut events, f);
        events
    }

    fn presses(events: &[Event]) -> Vec<(PointerButton, bool)> {
        events.iter().filter_map(|e| if let Event::PointerButton { button, pressed, .. } = e { Some((*button, *pressed)) } else { None }).collect()
    }

    fn scrolled(events: &[Event]) -> Vec2 {
        events.iter().fold(Vec2::ZERO, |a, e| if let Event::MouseWheel { delta, .. } = e { a + *delta } else { a })
    }

    #[test]
    fn the_pencil_presses_drags_and_releases_with_pressure() {
        let mut r = TouchRouter::default();
        let down = run(&mut r, vec![touch(1, TouchPhase::Start, 200.0, 200.0, Some(0.3))], &frame(0.0, false));
        assert_eq!(presses(&down), vec![(PointerButton::Primary, true)]);
        assert!(r.pencil_seen);
        assert_eq!(r.pencil_pressure(), Some(0.3));
        let drag = run(&mut r, vec![touch(1, TouchPhase::Move, 260.0, 230.0, Some(0.8))], &frame(0.02, false));
        assert!(drag.iter().any(|e| matches!(e, Event::PointerMoved(p) if *p == egui::pos2(260.0, 230.0))));
        assert_eq!(r.pencil_pressure(), Some(0.8));
        let up = run(&mut r, vec![touch(1, TouchPhase::End, 260.0, 230.0, Some(0.0))], &frame(0.04, false));
        assert_eq!(presses(&up), vec![(PointerButton::Primary, false)]);
        assert_eq!(r.pencil_pressure(), None);
    }

    #[test]
    fn egui_winits_mouse_emulation_is_replaced() {
        let mut r = TouchRouter::default();
        let events = vec![
            touch(1, TouchPhase::Start, 200.0, 200.0, None),
            Event::PointerMoved(egui::pos2(200.0, 200.0)),
            Event::PointerButton { pos: egui::pos2(200.0, 200.0), button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
            Event::Text("kept".into()),
        ];
        let out = run(&mut r, events, &frame(0.0, false));
        assert!(presses(&out).is_empty(), "a finger on the page doesn't press: {out:?}");
        assert!(out.iter().any(|e| matches!(e, Event::Text(t) if t == "kept")));
        assert!(!out.iter().any(|e| matches!(e, Event::Touch { .. })), "egui's own multi-touch never sees the touches");
    }

    #[test]
    fn a_finger_on_the_page_scrolls_and_glides_instead_of_selecting() {
        let mut r = TouchRouter::default();
        run(&mut r, vec![touch(1, TouchPhase::Start, 400.0, 800.0, None)], &frame(0.0, false));
        let mut total = Vec2::ZERO;
        for i in 1..=10 {
            let out = run(&mut r, vec![touch(1, TouchPhase::Move, 400.0, 800.0 - 30.0 * i as f32, None)], &frame(0.016 * i as f64, false));
            assert!(presses(&out).is_empty());
            total += scrolled(&out);
        }
        // The first move past the slop starts the pan; motion after it scrolls, content follows the finger.
        assert!(total.y < -200.0 && total.x == 0.0, "{total:?}");
        let up = run(&mut r, vec![touch(1, TouchPhase::End, 400.0, 500.0, None)], &frame(0.17, false));
        assert!(presses(&up).is_empty(), "a pan is not a click");
        let mut f = frame(0.2, false);
        let mut out = Vec::new();
        assert!(r.route(&mut out, &f), "a flick keeps gliding");
        assert!(scrolled(&out).y < 0.0);
        f.now = 5.0;
        let mut out = Vec::new();
        for _ in 0..200 {
            out.clear();
            if !r.route(&mut out, &f) {
                break;
            }
            f.now += 0.1;
        }
        assert!(!r.route(&mut Vec::new(), &f), "and stops");
    }

    #[test]
    fn a_tap_clicks_and_a_long_press_right_clicks() {
        let mut r = TouchRouter::default();
        run(&mut r, vec![touch(1, TouchPhase::Start, 300.0, 300.0, None)], &frame(0.0, false));
        let tap = run(&mut r, vec![touch(1, TouchPhase::End, 303.0, 302.0, None)], &frame(0.1, false));
        assert_eq!(presses(&tap), vec![(PointerButton::Primary, true), (PointerButton::Primary, false)]);

        run(&mut r, vec![touch(2, TouchPhase::Start, 300.0, 300.0, None)], &frame(1.0, false));
        let mut wait = Vec::new();
        assert!(r.route(&mut wait, &frame(1.2, false)), "the long press is being timed");
        let held = run(&mut r, vec![], &frame(1.6, false));
        assert_eq!(presses(&held), vec![(PointerButton::Secondary, true), (PointerButton::Secondary, false)]);
        let up = run(&mut r, vec![touch(2, TouchPhase::End, 300.0, 300.0, None)], &frame(2.0, false));
        assert!(presses(&up).is_empty(), "lifting after a long press doesn't click too");
    }

    #[test]
    fn pinching_zooms_between_the_fingers() {
        let mut r = TouchRouter::default();
        run(&mut r, vec![touch(1, TouchPhase::Start, 400.0, 500.0, None), touch(2, TouchPhase::Start, 600.0, 500.0, None)], &frame(0.0, false));
        run(&mut r, vec![], &frame(0.01, false));
        let out =
            run(&mut r, vec![touch(1, TouchPhase::Move, 350.0, 500.0, None), touch(2, TouchPhase::Move, 650.0, 500.0, None)], &frame(0.02, false));
        let zoom: f32 = out.iter().filter_map(|e| if let Event::Zoom(z) = e { Some(*z) } else { None }).product();
        assert!((zoom - 1.5).abs() < 1e-3, "{zoom}");
        assert!(out.iter().any(|e| matches!(e, Event::PointerMoved(p) if *p == egui::pos2(500.0, 500.0))));
        assert!(scrolled(&out).length() < 1e-3, "spreading evenly doesn't scroll");
        assert!(presses(&out).is_empty());
    }

    #[test]
    fn palms_are_ignored_while_the_pencil_draws() {
        let mut r = TouchRouter::default();
        run(&mut r, vec![touch(1, TouchPhase::Start, 200.0, 200.0, Some(0.5))], &frame(0.0, false));
        let palm =
            run(&mut r, vec![touch(2, TouchPhase::Start, 500.0, 700.0, None), touch(2, TouchPhase::Move, 560.0, 640.0, None)], &frame(0.02, false));
        assert!(presses(&palm).is_empty() && scrolled(&palm) == Vec2::ZERO);
        let up = run(&mut r, vec![touch(2, TouchPhase::End, 560.0, 640.0, None)], &frame(0.03, false));
        assert!(presses(&up).is_empty(), "the palm lifting isn't a tap");
    }

    #[test]
    fn a_palm_that_landed_first_gives_way_to_the_pencil() {
        let mut r = TouchRouter::default();
        run(&mut r, vec![touch(1, TouchPhase::Start, 500.0, 700.0, None)], &frame(0.0, false));
        let pen = run(&mut r, vec![touch(2, TouchPhase::Start, 200.0, 200.0, Some(0.4))], &frame(0.05, false));
        assert_eq!(presses(&pen), vec![(PointerButton::Primary, true)]);
        let palm_up = run(&mut r, vec![touch(1, TouchPhase::End, 500.0, 700.0, None)], &frame(0.1, false));
        assert!(presses(&palm_up).is_empty(), "no stray click from the palm");
    }

    #[test]
    fn with_fingers_drawing_one_finger_drags_from_where_it_landed() {
        let mut r = TouchRouter::default();
        run(&mut r, vec![touch(1, TouchPhase::Start, 300.0, 300.0, None)], &frame(0.0, true));
        let out = run(&mut r, vec![touch(1, TouchPhase::Move, 340.0, 300.0, None)], &frame(0.02, true));
        assert_eq!(presses(&out), vec![(PointerButton::Primary, true)]);
        assert!(out.iter().any(|e| matches!(e, Event::PointerButton { pos, .. } if *pos == egui::pos2(300.0, 300.0))));
        // Two fingers still scroll.
        let mut r = TouchRouter::default();
        run(&mut r, vec![touch(1, TouchPhase::Start, 300.0, 300.0, None), touch(2, TouchPhase::Start, 400.0, 300.0, None)], &frame(0.0, true));
        run(&mut r, vec![], &frame(0.01, true));
        let out =
            run(&mut r, vec![touch(1, TouchPhase::Move, 300.0, 260.0, None), touch(2, TouchPhase::Move, 400.0, 260.0, None)], &frame(0.02, true));
        assert!(presses(&out).is_empty());
        assert!((scrolled(&out).y + 40.0).abs() < 1e-3);
    }

    #[test]
    fn fingers_on_the_interface_are_the_pointer() {
        let mut r = TouchRouter::default();
        let out = run(&mut r, vec![touch(1, TouchPhase::Start, 20.0, 20.0, None)], &frame(0.0, false));
        assert_eq!(presses(&out), vec![(PointerButton::Primary, true)]);
        let out = run(&mut r, vec![touch(1, TouchPhase::End, 20.0, 20.0, None)], &frame(0.1, false));
        assert_eq!(presses(&out), vec![(PointerButton::Primary, false)]);
    }

    #[test]
    fn hostile_touches_are_harmless() {
        let mut r = TouchRouter::default();
        let mut events = vec![
            touch(1, TouchPhase::Start, f32::NAN, 1.0, None),
            touch(2, TouchPhase::Move, 1.0, 1.0, Some(f32::INFINITY)),
            touch(3, TouchPhase::End, 1.0, 1.0, None),
            touch(4, TouchPhase::Start, 200.0, 200.0, Some(f32::NAN)),
        ];
        for id in 10..40 {
            events.push(touch(id, TouchPhase::Start, 500.0, 500.0, None));
        }
        r.route(&mut events, &frame(0.0, false));
        assert!(r.contacts.len() <= MAX_CONTACTS);
        r.route(&mut vec![touch(4, TouchPhase::Cancel, 0.0, 0.0, None)], &frame(f64::NAN, false));
        r.route(&mut Vec::new(), &frame(f64::INFINITY, false));
    }
}
