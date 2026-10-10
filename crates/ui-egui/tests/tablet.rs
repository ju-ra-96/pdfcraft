//! Tablet mode (PdfCraft for iPad) in the real shell: the Apple Pencil draws with pressure,
//! fingers scroll instead of drawing, and "Only the Pencil draws" can be turned off.
//!
//! Touches go through the app's `raw_input_hook`, as eframe sends them on iPadOS: the hook
//! rewrites them, and the rewritten events are what egui sees.

use eframe::App as _;
use egui::{Event, Pos2, TouchDeviceId, TouchId, TouchPhase, pos2};
use egui_kittest::Harness;
use pdfcraft_ui_egui::touch::Tablet;
use pdfcraft_ui_egui::{PdfCraftApp, QuickTool};

/// Two 300×600 pages: taller than the window, so there is something to scroll.
const FIXTURE: &[u8] = b"%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 300 600] >> endobj
4 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 300 600] >> endobj
trailer << /Root 1 0 R >>
%%EOF";

struct Tab {
    h: Harness<'static, PdfCraftApp>,
    time: f64,
}

impl Tab {
    fn new(setup: impl FnOnce(&mut PdfCraftApp) + 'static) -> Self {
        let mut h = Harness::builder().with_size(egui::vec2(1000.0, 1300.0)).build_eframe(move |_cc| {
            let mut app = PdfCraftApp::new();
            app.enable_tablet(Tablet::IPad);
            app.set_option("language", "en").unwrap();
            app.open_bytes("pages.pdf", None, FIXTURE.to_vec()).expect("opens");
            app.set_option("left", "closed").unwrap();
            app.set_option("author", "Tester").unwrap();
            app.set_option("quick", "ink").unwrap();
            setup(&mut app);
            app
        });
        h.run_steps(6);
        Tab { h, time: 1.0 }
    }

    /// A point on page 1 in PDF user space → screen.
    fn at(&self, x: f32, y: f32) -> Pos2 {
        let r = self.h.state().views[0].page_screen_rect(0).expect("page 1 on screen");
        pos2(r.left() + x / 300.0 * r.width(), r.top() + (600.0 - y) / 600.0 * r.height())
    }

    /// One frame with these touches, routed by the app as on iPadOS.
    fn frame(&mut self, touches: &[(u64, TouchPhase, Pos2, Option<f32>)]) {
        self.time += 1.0 / 60.0;
        let events =
            touches.iter().map(|&(id, phase, pos, force)| Event::Touch { device_id: TouchDeviceId(7), id: TouchId(id), phase, pos, force }).collect();
        let mut raw = egui::RawInput { events, time: Some(self.time), ..Default::default() };
        let ctx = self.h.ctx.clone();
        self.h.state_mut().raw_input_hook(&ctx, &mut raw);
        for e in raw.events {
            self.h.event(e);
        }
        self.h.run_steps(1);
    }

    /// Drag one touch from `a` to `b` in `steps` moves.
    fn stroke(&mut self, id: u64, a: Pos2, b: Pos2, force: impl Fn(usize) -> Option<f32>) {
        let steps = 12;
        self.frame(&[(id, TouchPhase::Start, a, force(0))]);
        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            self.frame(&[(id, TouchPhase::Move, a + (b - a) * t, force(i))]);
        }
        self.frame(&[(id, TouchPhase::End, b, force(steps))]);
        self.h.run_steps(4);
    }

    fn kinds(&self) -> Vec<String> {
        let s = self.h.state();
        s.session.get(s.views[0].id).unwrap().info.annotations.iter().map(|a| a.subtype.clone()).collect()
    }

    /// How many points of pressure each Ink annotation carries in the document (`/PCPressure`).
    fn pressures(&self) -> Vec<usize> {
        let s = self.h.state();
        let bytes = s.session.get(s.views[0].id).unwrap().bytes.as_ref().clone();
        let doc = pdfcraft_cos::Document::open(std::sync::Arc::new(bytes)).unwrap();
        doc.object_numbers()
            .into_iter()
            .filter_map(|n| doc.try_get(n).ok())
            .filter_map(|o| o.as_dict().cloned())
            .filter(|d| d.name(b"Subtype") == Some(b"Ink"))
            .map(|d| {
                d.get(b"PCPressure").and_then(|p| p.as_array().cloned()).map_or(0, |a| a.iter().filter_map(|s| s.as_array()).map(|s| s.len()).sum())
            })
            .collect()
    }
}

#[test]
fn the_pencil_draws_ink_that_follows_its_pressure() {
    let mut tab = Tab::new(|_| {});
    let (a, b) = (tab.at(40.0, 560.0), tab.at(240.0, 520.0));
    tab.stroke(1, a, b, |i| Some(0.1 + i as f32 * 0.07));
    assert_eq!(tab.kinds(), ["Ink"]);
    assert_eq!(tab.pressures(), [13], "a pressure per point is saved with the stroke");
    assert!(tab.h.state().touch.as_ref().is_some_and(|t| t.pencil_seen));
    assert!(tab.h.state().pencil_only_draws(), "after the Pencil, fingers stop drawing by themselves");
    assert_eq!(tab.h.state().quick_tool, QuickTool::Comment(pdfcraft_ui_egui::comments::CommentTool::Ink), "the pen stays on");
}

#[test]
fn a_finger_scrolls_the_page_instead_of_drawing() {
    let mut tab = Tab::new(|app| app.pencil_only = Some(true));
    let top = tab.h.state().views[0].page_screen_rect(0).unwrap().top();
    let (a, b) = (tab.at(150.0, 450.0), tab.at(150.0, 450.0) - egui::vec2(0.0, 250.0));
    tab.stroke(2, a, b, |_| None);
    for _ in 0..30 {
        tab.frame(&[]);
    }
    assert!(tab.kinds().is_empty(), "nothing drawn: {:?}", tab.kinds());
    let after = tab.h.state().views[0].page_screen_rect(0).map_or(f32::NEG_INFINITY, |r| r.top());
    assert!(after < top - 100.0, "the page moved up with the finger: {top} → {after}");
}

#[test]
fn a_palm_on_the_page_while_the_pencil_draws_changes_nothing() {
    let mut tab = Tab::new(|_| {});
    let (a, b) = (tab.at(40.0, 560.0), tab.at(240.0, 520.0));
    let palm = tab.at(200.0, 480.0);
    tab.frame(&[(1, TouchPhase::Start, a, Some(0.3))]);
    tab.frame(&[(9, TouchPhase::Start, palm, None)]);
    for i in 1..=10 {
        let t = i as f32 / 10.0;
        tab.frame(&[(1, TouchPhase::Move, a + (b - a) * t, Some(0.3)), (9, TouchPhase::Move, palm + egui::vec2(0.0, -4.0 * i as f32), None)]);
    }
    tab.frame(&[(9, TouchPhase::End, palm, None)]);
    tab.frame(&[(1, TouchPhase::End, b, Some(0.3))]);
    tab.h.run_steps(4);
    assert_eq!(tab.kinds(), ["Ink"], "one stroke, from the Pencil only");
}

#[test]
fn with_only_the_pencil_drawing_off_a_finger_draws_too() {
    let mut tab = Tab::new(|app| app.pencil_only = Some(false));
    let (a, b) = (tab.at(40.0, 560.0), tab.at(240.0, 520.0));
    tab.stroke(3, a, b, |_| None);
    assert_eq!(tab.kinds(), ["Ink"]);
    assert_eq!(tab.pressures(), [0], "a finger has no pressure: an even stroke");
}

#[test]
fn the_choice_and_the_pencil_survive_a_restart() {
    let mut app = PdfCraftApp::new();
    app.enable_tablet(Tablet::IPad);
    app.pencil_only = Some(false);
    if let Some(t) = app.touch.as_mut() {
        t.pencil_seen = true;
    }
    let saved = app.persist();
    let mut again = PdfCraftApp::new();
    again.enable_tablet(Tablet::IPad);
    again.restore(&saved);
    assert_eq!(again.pencil_only, Some(false));
    assert!(again.touch.as_ref().is_some_and(|t| t.pencil_seen));
}

#[test]
fn only_the_pencil_draws_is_an_option_for_scripts() {
    let mut app = PdfCraftApp::new();
    app.enable_tablet(Tablet::IPad);
    assert!(!app.pencil_only_draws(), "automatic: fingers draw until a Pencil is used");
    app.set_option("pencil_only", "on").unwrap();
    assert!(app.pencil_only_draws());
    app.set_option("pencil_only", "off").unwrap();
    assert_eq!(app.pencil_only, Some(false));
    app.set_option("pencil_only", "auto").unwrap();
    assert_eq!(app.pencil_only, None);
    assert!(app.set_option("pencil_only", "sometimes").unwrap_err().contains("auto, on or off"));
}
