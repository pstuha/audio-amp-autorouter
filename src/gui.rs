//! AmpRoute Studio — KiCad-like 2-D board viewer and autorouter front-end.
//!
//! The layout follows a familiar EDA application: menu bar, toolbars, an
//! appearance/net/component tree on the left, a rotation-capable 2-D canvas in
//! the middle, an inspector and route report on the right, and a status bar.
//! The board is drawn as a real object — substrate, pads with holes, component
//! bodies, silkscreen, mounting holes and copper on named layers — rather than
//! as an abstract schematic.

use crate::board::{Board, Footprint, LutPreset, NetClass};
use crate::export;
use crate::import;
use crate::kicad;
use crate::pipeline::{PipelineConfig, PipelineResult};
use crate::router::{CopperLayer, RouteSegment, RoutingResult};
use crate::thermal::{self, ThermalField, ThermalParams};
use eframe::egui;
use egui::{Color32, FontId, Pos2, Rect, RichText, Stroke, Vec2};
use std::path::PathBuf;
use std::sync::mpsc::Receiver;

const CANVAS_BG: Color32 = Color32::from_rgb(11, 18, 36);
const SUBSTRATE: Color32 = Color32::from_rgb(18, 66, 43);
const SUBSTRATE_EDGE: Color32 = Color32::from_rgb(120, 205, 145);
const GRID: Color32 = Color32::from_rgb(28, 48, 68);
const PAD: Color32 = Color32::from_rgb(216, 178, 78);
const PAD_EDGE: Color32 = Color32::from_rgb(250, 226, 140);
const HOLE: Color32 = Color32::from_rgb(8, 12, 22);
const BODY: Color32 = Color32::from_rgb(46, 52, 58);
const BODY_EDGE: Color32 = Color32::from_rgb(150, 162, 172);
const SILK: Color32 = Color32::from_rgb(220, 226, 230);
const FRONT_CU: Color32 = Color32::from_rgb(226, 88, 68);
const BACK_CU: Color32 = Color32::from_rgb(74, 145, 226);
const SELECTED: Color32 = Color32::from_rgb(255, 236, 118);

fn layer_color(layer: CopperLayer) -> Color32 {
    match layer {
        CopperLayer::FCu => FRONT_CU,
        CopperLayer::BCu => BACK_CU,
    }
}

#[derive(Clone, Copy)]
struct View {
    zoom: f32,
    pan: Vec2,
    /// View rotation in KiCad convention (positive = counter-clockwise).
    angle: f32,
    /// Mirror the board horizontally, for inspecting the bottom side.
    flip: bool,
}

impl Default for View {
    fn default() -> Self {
        Self {
            zoom: 5.0,
            pan: Vec2::ZERO,
            angle: 0.0,
            flip: false,
        }
    }
}

#[derive(Clone, Debug)]
enum Selection {
    Net(String),
    Footprint(String),
    Pad(String, usize),
    Segment(usize),
    Hole(usize),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Ready,
    Routing,
    Done,
    Incomplete,
}

struct Appearance {
    grid: bool,
    components: bool,
    pads: bool,
    copper: bool,
    silk: bool,
    keepouts: bool,
    heatmap: bool,
    front: bool,
    back: bool,
}

enum Dialog {
    About,
    Hotkeys,
    Message(String),
}

/// Persistent application state.
pub struct AmpRouteApp {
    board: Board,
    file_path: Option<PathBuf>,
    /// User-chosen destination for all export files.
    export_dir: Option<PathBuf>,
    copper: RoutingResult,
    pipe: Option<PipelineResult>,
    stage: Stage,
    status: String,
    route_job: Option<Receiver<anyhow::Result<PipelineResult>>>,
    route_started: Option<std::time::Instant>,
    view: View,
    selection: Option<Selection>,
    hover: Option<(f64, f64)>,
    grid_mm: f64,
    time_budget_s: f64,
    run_kicad_cli_drc: bool,
    two_layer: bool,
    show: Appearance,
    dialog: Option<Dialog>,
    thermal_field: Option<ThermalField>,
    /// Undo history of routing runs.
    history: History,
    /// Size of the central panel, used by zoom-to-fit.
    canvas_size: Vec2,
}

/// Per-net summary row for the GUI statistics panel.
struct StatsRow {
    name: String,
    length_mm: f64,
    segments: usize,
    width_min: f64,
}

/// Board totals for the GUI statistics panel.
struct BoardStats {
    copper_mm: f64,
    segments: usize,
    widths: Option<(f64, f64, f64)>,
    routed: usize,
    total: usize,
    nets: Vec<StatsRow>,
}

/// Undo history of routing runs: `past` holds earlier `RoutingResult`
/// states, `future` the states undone but not yet redone.
#[derive(Default)]
struct History {
    past: Vec<RoutingResult>,
    future: Vec<RoutingResult>,
}

impl History {
    fn clear(&mut self) {
        self.past.clear();
        self.future.clear();
    }

    fn push(&mut self, before: RoutingResult) {
        self.future.clear();
        self.past.push(before);
    }

    fn undo(&mut self, current: &mut RoutingResult) -> bool {
        let Some(state) = self.past.pop() else { return false; };
        self.future.push(std::mem::take(current));
        *current = state;
        true
    }

    fn redo(&mut self, current: &mut RoutingResult) -> bool {
        let Some(state) = self.future.pop() else { return false; };
        self.past.push(std::mem::take(current));
        *current = state;
        true
    }
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            grid: true,
            components: true,
            pads: true,
            copper: true,
            silk: true,
            keepouts: true,
            heatmap: false,
            front: true,
            back: true,
        }
    }
}

impl AmpRouteApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = Color32::from_rgb(38, 40, 44);
        visuals.window_fill = Color32::from_rgb(44, 47, 52);
        visuals.extreme_bg_color = Color32::from_rgb(18, 20, 24);
        visuals.faint_bg_color = Color32::from_rgb(48, 51, 56);
        visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(44, 47, 52);
        visuals.widgets.inactive.bg_fill = Color32::from_rgb(60, 64, 70);
        visuals.widgets.inactive.fg_stroke =
            Stroke::new(1.0_f32, Color32::from_rgb(222, 226, 232));
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(76, 92, 112);
        visuals.widgets.active.bg_fill = Color32::from_rgb(58, 118, 164);
        visuals.selection.bg_fill = Color32::from_rgb(46, 102, 148);
        visuals.selection.stroke = Stroke::new(1.0_f32, Color32::from_rgb(150, 205, 245));
        cc.egui_ctx.set_visuals(visuals);

        // No preloaded boards: built-in designs exist only as test fixtures.
        let board = Board::new("No board", 100.0, 80.0, LutPreset::reliable());
        let thermal_field = Some(simulate(&board));
        let mut app = Self {
            board,
            file_path: None,
            export_dir: None,
            copper: RoutingResult::new(),
            history: History::default(),
            pipe: None,
            stage: Stage::Ready,
            status: "Open a board to begin".into(),
            route_job: None,
            route_started: None,
            view: View::default(),
            selection: None,
            hover: None,
            grid_mm: 0.5,
            time_budget_s: 300.0,
            run_kicad_cli_drc: false,
            two_layer: false,
            show: Appearance::default(),
            dialog: None,
            thermal_field,
            canvas_size: Vec2::new(1000.0, 700.0),
        };
        app.fit_view();
        if let Some(path) = std::env::var_os("AMPROUTE_OPEN_FILE") {
            app.load_path(PathBuf::from(path));
        }
        app
    }

    // -----------------------------------------------------------------------
    // Actions
    // -----------------------------------------------------------------------

    fn open_file(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("KiCad PCB / JSON", &["kicad_pcb", "json"])
            .pick_file()
        else {
            return;
        };
        self.load_path(path);
    }

    fn load_path(&mut self, path: PathBuf) {
        let data = match std::fs::read_to_string(&path) {
            Ok(d) => d,
            Err(e) => {
                self.set_status(format!("Cannot read {}: {e}", path.display()));
                return;
            }
        };
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "Imported board".into());
        let board = if data.trim_start().starts_with('{') {
            Board::from_json(&data)
        } else {
            import::import_kicad_pcb_str_named(&data, &stem).map(|r| r.board)
        };
        match board {
            Ok(board) => {
                self.grid_mm = board.preset.grid_mm;
                self.board = board;
                self.file_path = Some(path);
                self.reset_result();
                self.fit_view();
            }
            Err(e) => self.set_status(format!("Cannot parse board: {e}")),
        }
    }

    fn choose_export_dir(&mut self) {
        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
            self.export_dir = Some(dir);
            self.set_status(format!(
                "Export folder: {}",
                self.export_dir.as_ref().unwrap().display()
            ));
        }
    }

    fn output_dir(&self) -> PathBuf {
        self.export_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from("output"))
    }

    fn reset_result(&mut self) {
        self.copper = RoutingResult::new();
        self.history.clear();
        self.pipe = None;
        self.selection = None;
        self.stage = Stage::Ready;
        self.thermal_field = Some(simulate(&self.board));
        self.status = format!(
            "Loaded {} — {} nets, {} pads. Press F5 to route.",
            self.board.name,
            self.board.nets.len(),
            self.board.all_pads().len()
        );
    }

    fn set_status(&mut self, text: impl Into<String>) {
        self.status = text.into();
    }

    fn route(&mut self) {
        if self.route_job.is_some() {
            return;
        }
        if self.board.nets.is_empty() {
            self.set_status("Nothing to route: no net is assigned to a pad");
            self.stage = Stage::Incomplete;
            return;
        }
        let mut board = self.board.clone();
        if self.two_layer {
            board.two_layer = true;
        }
        let config = PipelineConfig {
            grid_mm: self.grid_mm,
            time_budget_s: self.time_budget_s,
            run_kicad_cli_drc: self.run_kicad_cli_drc,
            ..Default::default()
        };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let res = crate::pipeline::run_pipeline(board, config, "output");
            let _ = tx.send(res);
        });
        self.route_job = Some(rx);
        self.route_started = Some(std::time::Instant::now());
        self.stage = Stage::Routing;
        self.set_status("Routing…");
    }

    fn poll_route(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.route_job else { return };
        let result = match rx.try_recv() {
            Ok(r) => r,
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
                return;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.route_job = None;
                self.set_status("Routing worker stopped unexpectedly");
                self.stage = Stage::Incomplete;
                return;
            }
        };
        self.route_job = None;
        self.route_started = None;
        match result {
            Ok(pipe) => {
                self.history.push(self.copper.clone());
                self.copper = pipe.final_copper.clone();
                self.thermal_field = Some(pipe.thermal.clone());
                let nets = pipe.routing.result.routed_nets.len();
                let failed = pipe.routing.result.failed_nets.len();
                let elapsed = pipe.elapsed_ms;
                let drc = pipe.drc.violations.len();
                self.stage = if failed == 0 && drc == 0 {
                    Stage::Done
                } else {
                    Stage::Incomplete
                };
                self.set_status(format!(
                    "{} nets routed, {} failed, {} DRC violations, {:.1} s",
                    nets,
                    failed,
                    drc,
                    elapsed as f64 / 1000.0
                ));
                self.pipe = Some(pipe);
            }
            Err(e) => {
                self.stage = Stage::Incomplete;
                self.set_status(format!("Pipeline error: {e}"));
            }
        }
    }

    fn stop_route(&mut self) {
        if self.route_job.take().is_some() {
            self.stage = Stage::Ready;
            self.set_status("Routing stopped");
        }
    }

    fn export_all(&mut self) {
        if self.copper.failed_nets.iter().any(|n| !n.is_empty()) {
            self.set_status("Export refused: routing is incomplete");
            return;
        }
        if self.copper.segments.is_empty() {
            self.set_status("Export refused: route the board first (F5)");
            return;
        }
        let dir = self.output_dir();
        let report = match export::write_all(&self.board, &self.copper, dir.to_str().unwrap_or("output")) {
            Ok(files) => format!("Written to {}:\n{}", dir.display(), files.join("\n")),
            Err(e) => format!("Export refused: {e}"),
        };
        self.dialog = Some(Dialog::Message(report));
    }

    fn export_kicad(&mut self) {
        let stem = self.board.name.replace(' ', "_").to_lowercase();
        let path = self.output_dir().join(format!("{stem}.kicad_pcb"));
        let report = match kicad::write_kicad_pcb(
            &self.board,
            &self.copper,
            &kicad::FabOptions::default(),
            path.to_str().unwrap_or("board.kicad_pcb"),
        ) {
            Ok(()) => format!("KiCad board written to {}", path.display()),
            Err(e) => format!("KiCad export failed: {e}"),
        };
        self.dialog = Some(Dialog::Message(report));
    }

    // -----------------------------------------------------------------------
    // View transform
    // -----------------------------------------------------------------------

    fn board_center(&self) -> (f64, f64) {
        (self.board.width_mm / 2.0, self.board.height_mm / 2.0)
    }

    /// Board millimetres -> screen pixels. Rotation and mirroring happen
    /// around the board centre so the view stays centred on any angle.
    fn to_screen(&self, rect: Rect, p: (f64, f64)) -> Pos2 {
        let (cx, cy) = self.board_center();
        let dx = p.0 - cx;
        let dy = p.1 - cy;
        let a = self.view.angle.to_radians() as f64;
        let mut rx = dx * a.cos() + dy * a.sin();
        let ry = -dx * a.sin() + dy * a.cos();
        if self.view.flip {
            rx = -rx;
        }
        let c = rect.center() + self.view.pan;
        Pos2::new(c.x + (rx * self.view.zoom as f64) as f32, c.y + (ry * self.view.zoom as f64) as f32)
    }

    fn to_board(&self, rect: Rect, p: Pos2) -> (f64, f64) {
        let c = rect.center() + self.view.pan;
        let mut rx = (p.x - c.x) as f64 / self.view.zoom as f64;
        let ry = (p.y - c.y) as f64 / self.view.zoom as f64;
        if self.view.flip {
            rx = -rx;
        }
        let a = self.view.angle.to_radians() as f64;
        let dx = rx * a.cos() - ry * a.sin();
        let dy = rx * a.sin() + ry * a.cos();
        let (cx, cy) = self.board_center();
        (cx + dx, cy + dy)
    }

    fn fit_view(&mut self) {
        // After rotation the board's bounding box is symmetric around its
        // centre, so zoom-to-fit only needs the rotated half-extents.
        let a = self.view.angle.to_radians().abs() as f64;
        let hw = self.board.width_mm / 2.0;
        let hh = self.board.height_mm / 2.0;
        let bw = 2.0 * (hw * a.cos() + hh * a.sin());
        let bh = 2.0 * (hw * a.sin() + hh * a.cos());
        let available = self.canvas_size;
        self.view.zoom = (available.x / (bw as f32 + 12.0))
            .min(available.y / (bh as f32 + 12.0))
            .clamp(0.5, 80.0);
        self.view.pan = Vec2::ZERO;
    }

    fn rotate(&mut self, degrees: f32) {
        self.view.angle = (self.view.angle + degrees).rem_euclid(360.0);
        self.fit_view();
    }

    // -----------------------------------------------------------------------
    // Canvas drawing
    // -----------------------------------------------------------------------

    fn draw_canvas(&mut self, ui: &mut egui::Ui) {
        let (resp, painter) = ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
        let rect = resp.rect;
        self.canvas_size = rect.size();

        if resp.hovered() {
            let scroll = ui.input(|i| i.raw_scroll_delta.y);
            if scroll != 0.0 {
                let factor = if scroll > 0.0 { 1.12 } else { 1.0 / 1.12 };
                let pivot = resp.hover_pos().unwrap_or(rect.center());
                let rel = pivot - rect.min;
                self.view.zoom = (self.view.zoom * factor).clamp(0.4, 120.0);
                self.view.pan = rel - (rel - self.view.pan) * factor;
            }
        }
        if resp.dragged() {
            self.view.pan += resp.drag_delta();
        }
        self.hover = resp.hover_pos().map(|p| self.to_board(rect, p));
        if resp.clicked() {
            if let Some(pos) = resp.hover_pos() {
                self.selection = self.pick(rect, pos);
            }
        }

        painter.rect_filled(rect, 0.0, CANVAS_BG);

        let w = self.board.width_mm;
        let h = self.board.height_mm;
        let corners = [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)];

        // Substrate as a rotated polygon.
        let pts: Vec<Pos2> = corners.iter().map(|p| self.to_screen(rect, *p)).collect();
        painter.add(egui::Shape::convex_polygon(
            pts,
            SUBSTRATE,
            Stroke::new(2.0_f32, SUBSTRATE_EDGE),
        ));

        if self.show.grid {
            let step = if self.view.zoom > 12.0 { 1.0 } else { 5.0 };
            let mut x = 0.0;
            while x <= w {
                painter.line_segment(
                    [self.to_screen(rect, (x, 0.0)), self.to_screen(rect, (x, h))],
                    Stroke::new(0.5_f32, GRID),
                );
                x += step;
            }
            let mut y = 0.0;
            while y <= h {
                painter.line_segment(
                    [self.to_screen(rect, (0.0, y)), self.to_screen(rect, (w, y))],
                    Stroke::new(0.5_f32, GRID),
                );
                y += step;
            }
        }

        if self.show.keepouts {
            for (x, y, r) in self.board.thermal_hotspots() {
                let c = self.to_screen(rect, (x, y));
                painter.circle_filled(c, (r as f32) * self.view.zoom, Color32::from_rgba_unmultiplied(230, 96, 72, 26));
                painter.circle_stroke(c, (r as f32) * self.view.zoom, Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(230, 96, 72, 130)));
            }
            if let Some((x, y)) = self.board.star_ground_point() {
                let c = self.to_screen(rect, (x, y));
                painter.circle_stroke(c, 8.0 * self.view.zoom, Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(90, 220, 140, 150)));
            }
        }

        if self.show.heatmap {
            if let Some(field) = &self.thermal_field {
                self.draw_heatmap(&painter, rect, field);
            }
        }

        // Copper, on named layers.
        if self.show.copper {
            for (i, seg) in self.copper.segments.iter().enumerate() {
                let dim = match &self.selection {
                    Some(Selection::Net(net)) => net != &seg.net,
                    Some(Selection::Segment(idx)) => *idx != i,
                    _ => false,
                };
                let layer = seg.layer;
                if !match layer {
                    CopperLayer::FCu => self.show.front,
                    CopperLayer::BCu => self.show.back,
                } {
                    continue;
                }
                let mut col = layer_color(layer);
                if dim {
                    col = col.gamma_multiply(0.22);
                }
                let width = (seg.width_mm as f32 * self.view.zoom).max(1.0);
                let a = self.to_screen(rect, (seg.x1_mm, seg.y1_mm));
                let b = self.to_screen(rect, (seg.x2_mm, seg.y2_mm));
                painter.line_segment([a, b], Stroke::new(width, col));
                // Round caps/joins: egui strokes meet at sharp corners, a disc
                // at each vertex makes the rendered copper look like real
                // circular-aperture Gerber rather than a pixel staircase.
                painter.circle_filled(a, width / 2.0, col);
                painter.circle_filled(b, width / 2.0, col);
            }
        }

        // Component bodies, then pads and references.
        if self.show.components {
            for fp in &self.board.footprints {
                self.draw_footprint(&painter, rect, fp);
            }
        }
        if self.show.pads {
            for ((x, y), size, net) in self.board.all_pads() {
                let c = self.to_screen(rect, (x, y));
                let r = ((size / 2.0) as f32 * self.view.zoom).max(1.5);
                let dim = match &self.selection {
                    Some(Selection::Net(n)) => net.as_deref() != Some(n.as_str()),
                    _ => false,
                };
                let fill = if dim { PAD.gamma_multiply(0.35) } else { PAD };
                painter.circle_filled(c, r, fill);
                painter.circle_stroke(c, r, Stroke::new(0.8_f32, PAD_EDGE.gamma_multiply(if dim { 0.35 } else { 1.0 })));
                painter.circle_filled(c, (r * 0.38).max(0.8), HOLE);
            }
        }

        // Plated vias (two-layer boards).
        if self.show.copper {
            for via in &self.copper.via_objects {
                let c = self.to_screen(rect, (via.x_mm, via.y_mm));
                let outer = (via.diameter_mm as f32 / 2.0 * self.view.zoom).max(2.0);
                painter.circle_filled(c, outer, Color32::from_rgb(220, 226, 232));
                painter.circle_stroke(c, outer, Stroke::new(1.0_f32, layer_color(CopperLayer::FCu)));
                painter.circle_filled(c, (via.drill_mm as f32 / 2.0 * self.view.zoom).max(1.0), HOLE);
            }
        }

        // Mounting holes.
        for hole in &self.board.mounting_holes {
            let c = self.to_screen(rect, (hole.x, hole.y));
            let r = ((hole.drill_mm / 2.0 + 1.2) as f32 * self.view.zoom).max(2.0);
            painter.circle_filled(c, r, SUBSTRATE);
            painter.circle_stroke(c, r, Stroke::new(1.0_f32, Color32::from_rgb(184, 148, 86)));
            painter.circle_filled(c, (hole.drill_mm as f32 / 2.0 * self.view.zoom).max(1.0), HOLE);
        }

        if self.show.silk {
            for fp in &self.board.footprints {
                let (x, y) = fp.center();
                let c = self.to_screen(rect, (x, y));
                painter.text(
                    c,
                    egui::Align2::CENTER_CENTER,
                    &fp.refdes,
                    FontId::proportional(10.0),
                    SILK,
                );
            }
        }

        // Selection halo and hover readout.
        if let Some(sel) = &self.selection {
            self.draw_selection(&painter, rect, sel);
        }
        if let Some((x, y)) = self.hover {
            if x >= 0.0 && y >= 0.0 && x <= w && y <= h {
                painter.text(
                    rect.left_bottom() + Vec2::new(10.0, -10.0),
                    egui::Align2::LEFT_BOTTOM,
                    format!("X {x:.2}  Y {y:.2} mm"),
                    FontId::monospace(12.0),
                    Color32::from_rgb(160, 205, 235),
                );
            }
        }
        painter.text(
            rect.right_top() + Vec2::new(-10.0, 10.0),
            egui::Align2::RIGHT_TOP,
            format!("{}°{}  ·  {:.0} px/mm", self.view.angle as i32, if self.view.flip { " · flipped" } else { "" }, self.view.zoom),
            FontId::monospace(11.0),
            Color32::from_rgb(150, 170, 195),
        );
    }

    fn stats(&self) -> BoardStats {
        let mut map: std::collections::BTreeMap<String, (f64, usize, f64, f64, f64)> =
            Default::default();
        for seg in &self.copper.segments {
            let e = map.entry(seg.net.clone()).or_insert((0.0, 0, f64::MAX, 0.0, f64::MIN));
            e.0 += seg.length_mm();
            e.1 += 1;
            e.2 = e.2.min(seg.width_mm);
            e.3 += seg.width_mm;
            e.4 = e.4.max(seg.width_mm);
        }
        let segments: usize = map.values().map(|v| v.1).sum();
        let widths = if segments == 0 {
            None
        } else {
            let min = map.values().map(|v| v.2).fold(f64::MAX, f64::min);
            let max = map.values().map(|v| v.4).fold(f64::MIN, f64::max);
            Some((min, map.values().map(|v| v.3).sum::<f64>() / segments as f64, max))
        };
        let mut nets: Vec<StatsRow> = map
            .iter()
            .map(|(name, v)| StatsRow {
                name: name.clone(),
                length_mm: v.0,
                segments: v.1,
                width_min: v.2,
            })
            .collect();
        nets.sort_by(|a, b| b.length_mm.total_cmp(&a.length_mm));
        BoardStats {
            copper_mm: nets.iter().map(|n| n.length_mm).sum(),
            segments,
            widths,
            routed: self.copper.routed_nets.len(),
            total: self.board.nets.len(),
            nets,
        }
    }

    fn draw_heatmap(&self, painter: &egui::Painter, rect: Rect, field: &ThermalField) {
        if field.peak_rise_c <= 0.0 {
            return;
        }
        let cell = field.cell_mm;
        for row in 0..field.rows {
            for col in 0..field.cols {
                let rise = field.temps[row * field.cols + col];
                if rise <= 0.0 {
                    continue;
                }
                let (r, g, b) = thermal::colour_for_rise(rise as f64);
                let x = col as f64 * cell;
                let y = row as f64 * cell;
                let pts: Vec<Pos2> = [(x, y), (x + cell, y), (x + cell, y + cell), (x, y + cell)]
                    .iter()
                    .map(|p| self.to_screen(rect, *p))
                    .collect();
                painter.add(egui::Shape::convex_polygon(
                    pts,
                    Color32::from_rgba_unmultiplied(r, g, b, 90),
                    Stroke::NONE,
                ));
            }
        }
    }

    fn draw_footprint(&self, painter: &egui::Painter, rect: Rect, fp: &Footprint) {
        let selected = matches!(&self.selection, Some(Selection::Footprint(r)) if r == &fp.refdes);
        let edge = if selected {
            Stroke::new(2.0_f32, SELECTED)
        } else {
            Stroke::new(1.0_f32, BODY_EDGE)
        };
        let poly = |corners: &[(f64, f64)], fill: Color32| {
            let pts: Vec<Pos2> = corners.iter().map(|p| self.to_screen(rect, *p)).collect();
            painter.add(egui::Shape::convex_polygon(pts, fill, edge));
        };

        if fp.pads.len() >= 2 {
            if let (Some(a), Some(b)) = (fp.pad_world(0), fp.pad_world(1)) {
                let dx = b.0 - a.0;
                let dy = b.1 - a.1;
                let len = dx.hypot(dy);
                if len > 1e-6 {
                    let (ux, uy) = (dx / len, dy / len);
                    let (px, py) = (-uy, ux);
                    let mid = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
                    let kind = fp.refdes.chars().next().unwrap_or('?').to_ascii_uppercase();
                    let body_w = match kind {
                        'R' => 2.0,
                        'C' => 3.2,
                        'L' => 3.0,
                        'D' => 2.6,
                        _ => 3.0,
                    };
                    let body_len = (len - 2.0).max(1.0);
                    let body_fill = match kind {
                        'C' => Color32::from_rgb(38, 74, 132),
                        'R' => Color32::from_rgb(180, 156, 112),
                        'L' => Color32::from_rgb(124, 92, 62),
                        'D' => Color32::from_rgb(36, 36, 42),
                        _ => BODY,
                    };
                    let corner = |along: f64, across: f64| {
                        (
                            mid.0 + ux * along + px * across,
                            mid.1 + uy * along + py * across,
                        )
                    };
                    poly(
                        &[
                            corner(body_len / 2.0, body_w / 2.0),
                            corner(-body_len / 2.0, body_w / 2.0),
                            corner(-body_len / 2.0, -body_w / 2.0),
                            corner(body_len / 2.0, -body_w / 2.0),
                        ],
                        body_fill,
                    );
                    // Leads and the polarity stripe of a diode/electrolytic.
                    painter.line_segment([self.to_screen(rect, a), self.to_screen(rect, b)], Stroke::new(1.0_f32, Color32::from_rgb(176, 181, 186)));
                    if kind == 'D' || kind == 'C' {
                        let s = self.to_screen(rect, corner(-body_len / 2.0 + 0.4, 0.0));
                        painter.circle_filled(s, 2.0, Color32::from_rgb(210, 214, 220));
                    }
                    return;
                }
            }
        }

        // ICs, transistors and multi-lead parts: a black package over the pads.
        let (x0, y0, x1, y1) = fp.rect();
        poly(
            &[(x0, y0), (x1, y0), (x1, y1), (x0, y1)],
            if selected { Color32::from_rgb(58, 64, 72) } else { Color32::from_rgb(30, 33, 38) },
        );
        if fp.pads.len() > 3 {
            let dot = self.to_screen(rect, (x0 + 1.5, y0 + 1.5));
            painter.circle_filled(dot, 2.0, Color32::from_rgb(120, 128, 138));
        }
    }

    fn draw_selection(&self, painter: &egui::Painter, rect: Rect, sel: &Selection) {
        let highlight = |p: (f64, f64), r: f32| {
            let c = self.to_screen(rect, p);
            painter.circle_stroke(c, r, Stroke::new(1.5_f32, SELECTED));
        };
        match sel {
            Selection::Pad(refdes, index) => {
                if let Some(fp) = self.board.footprint(refdes) {
                    if let Some(p) = fp.pad_world(*index) {
                        highlight(p, 5.0);
                    }
                }
            }
            Selection::Hole(i) => {
                if let Some(h) = self.board.mounting_holes.get(*i) {
                    highlight((h.x, h.y), 6.0);
                }
            }
            Selection::Footprint(refdes) => {
                if let Some(fp) = self.board.footprint(refdes) {
                    let (x0, y0, x1, y1) = fp.rect();
                    let pts: Vec<Pos2> = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
                        .iter()
                        .map(|p| self.to_screen(rect, *p))
                        .collect();
                    painter.add(egui::Shape::convex_polygon(
                        pts,
                        Color32::TRANSPARENT,
                        Stroke::new(2.0_f32, SELECTED),
                    ));
                }
            }
            Selection::Segment(i) => {
                if let Some(seg) = self.copper.segments.get(*i) {
                    painter.line_segment(
                        [self.to_screen(rect, (seg.x1_mm, seg.y1_mm)), self.to_screen(rect, (seg.x2_mm, seg.y2_mm))],
                        Stroke::new((seg.width_mm as f32 * self.view.zoom).max(2.0) + 3.0, SELECTED.gamma_multiply(0.5)),
                    );
                }
            }
            Selection::Net(_) => {}
        }
    }

    fn pick(&self, rect: Rect, screen: Pos2) -> Option<Selection> {
        let p = self.to_board(rect, screen);
        let tolerance = 3.0 / self.view.zoom as f64;
        // Pads first — they are the smallest and most precise target.
        for fp in &self.board.footprints {
            for i in 0..fp.pads.len() {
                if let Some(w) = fp.pad_world(i) {
                    let size = fp.pads[i].size_mm / 2.0;
                    if (p.0 - w.0).hypot(p.1 - w.1) <= size + tolerance {
                        return Some(Selection::Pad(fp.refdes.clone(), i));
                    }
                }
            }
        }
        for (i, hole) in self.board.mounting_holes.iter().enumerate() {
            if (p.0 - hole.x).hypot(p.1 - hole.y) <= hole.drill_mm / 2.0 + tolerance {
                return Some(Selection::Hole(i));
            }
        }
        for (i, seg) in self.copper.segments.iter().enumerate() {
            if distance_to_segment(p, seg) <= seg.width_mm / 2.0 + tolerance {
                return Some(Selection::Segment(i));
            }
        }
        for fp in &self.board.footprints {
            if fp.contains(p.0, p.1) {
                return Some(Selection::Footprint(fp.refdes.clone()));
            }
        }
        None
    }

    // -----------------------------------------------------------------------
    // Panels
    // -----------------------------------------------------------------------

    fn menu_bar(&mut self, ctx: &egui::Context) {
        let mut open = false;
        let mut route = false;
        let mut stop = false;
        let mut export_kicad = false;
        let mut export_all = false;
        let mut fit = false;
        let mut rotate_l = false;
        let mut rotate_r = false;
        let mut flip = false;
        let mut undo = false;
        let mut redo = false;

        ctx.input(|i| {
            if i.modifiers.ctrl && i.key_pressed(egui::Key::O) { open = true; }
            if i.modifiers.ctrl && i.key_pressed(egui::Key::K) { export_kicad = true; }
            if i.modifiers.ctrl && i.key_pressed(egui::Key::E) { export_all = true; }
            if i.modifiers.ctrl && i.key_pressed(egui::Key::Z) { undo = true; }
            if i.modifiers.ctrl && i.key_pressed(egui::Key::Y) { redo = true; }
            if i.key_pressed(egui::Key::F5) { route = true; }
            if i.key_pressed(egui::Key::Escape) { stop = true; }
            if i.key_pressed(egui::Key::Home) { fit = true; }
            if i.key_pressed(egui::Key::R) {
                if i.modifiers.shift { rotate_l = true; } else { rotate_r = true; }
            }
            if i.key_pressed(egui::Key::F) { flip = true; }
        });
        if open { self.open_file(); }
        if route { self.route(); }
        if stop { self.stop_route(); }
        // Keyboard actions run here; menu actions set the same flags and are
        // executed after the menu closes. Clear them so nothing runs twice.
        if export_kicad { self.export_kicad(); export_kicad = false; }
        if export_all { self.export_all(); export_all = false; }
        if fit { self.fit_view(); fit = false; }
        if rotate_l { self.rotate(-90.0); rotate_l = false; }
        if rotate_r { self.rotate(90.0); rotate_r = false; }
        if flip {
            self.view.flip = !self.view.flip;
            self.fit_view();
            flip = false;
        }
        if undo && self.history.undo(&mut self.copper) {
            self.set_status("Route undone — Ctrl+Y to redo");
        }
        if redo && self.history.redo(&mut self.copper) {
            self.set_status("Route redone");
        }

        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open board…   Ctrl+O").clicked() { self.open_file(); ui.close_menu(); }
                    if ui.button("Choose export folder…").clicked() { self.choose_export_dir(); ui.close_menu(); }
                    ui.separator();
                    if ui.button("Export KiCad board   Ctrl+K").clicked() { export_kicad = true; ui.close_menu(); }
                    if ui.button("Export Gerber / SVG / PDF   Ctrl+E").clicked() { export_all = true; ui.close_menu(); }
                });
                ui.menu_button("Edit", |ui| {
                    if ui.button("Clear selection").clicked() { self.selection = None; ui.close_menu(); }
                    ui.add_enabled(false, egui::Button::new("Move / delete (planned)"));
                });
                ui.menu_button("View", |ui| {
                    if ui.button("Zoom to fit   Home").clicked() { fit = true; ui.close_menu(); }
                    if ui.button("Rotate 90°   R").clicked() { rotate_r = true; ui.close_menu(); }
                    if ui.button("Rotate −90°   Shift+R").clicked() { rotate_l = true; ui.close_menu(); }
                    if ui.button("Mirror board   F").clicked() { flip = true; ui.close_menu(); }
                    ui.separator();
                    ui.checkbox(&mut self.show.grid, "Grid");
                    ui.checkbox(&mut self.show.components, "Components");
                    ui.checkbox(&mut self.show.pads, "Pads");
                    ui.checkbox(&mut self.show.copper, "Copper");
                    ui.checkbox(&mut self.show.silk, "Silkscreen");
                    ui.checkbox(&mut self.show.keepouts, "Keep-out zones");
                    ui.checkbox(&mut self.show.heatmap, "Thermal heatmap");
                });
                ui.menu_button("Route", |ui| {
                    if ui.button("Route PCB   F5").clicked() { route = true; ui.close_menu(); }
                    if ui.button("Stop   Esc").clicked() { stop = true; ui.close_menu(); }
                });
                ui.menu_button("Inspect", |ui| {
                    if ui.button("DRC report").clicked() {
                        let text = self.pipe.as_ref().map(|p| p.drc.summary()).unwrap_or_else(|| "No run yet".into());
                        self.dialog = Some(Dialog::Message(text));
                        ui.close_menu();
                    }
                    if ui.button("Statistics").clicked() {
                        let text = self.stats_text();
                        self.dialog = Some(Dialog::Message(text));
                        ui.close_menu();
                    }
                });
                ui.menu_button("Help", |ui| {
                    if ui.button("Hotkeys").clicked() { self.dialog = Some(Dialog::Hotkeys); ui.close_menu(); }
                    if ui.button("About").clicked() { self.dialog = Some(Dialog::About); ui.close_menu(); }
                });
            });
        });
        if export_kicad { self.export_kicad(); }
        if export_all { self.export_all(); }
        if fit { self.fit_view(); }
        if rotate_l { self.rotate(-90.0); }
        if rotate_r { self.rotate(90.0); }
        if flip { self.view.flip = !self.view.flip; self.fit_view(); }

        // Toolbar.
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button("📂 Open").clicked() { self.open_file(); }
                ui.separator();
                if ui.button("▶ Route (F5)").clicked() { self.route(); }
                if ui.button("■ Stop").clicked() { self.stop_route(); }
                ui.separator();
                if ui.button("⤢ Fit").clicked() { self.fit_view(); }
                if ui.button("↺").clicked() { self.rotate(-90.0); }
                if ui.button("↻").clicked() { self.rotate(90.0); }
                if ui.button("⇄ Mirror").clicked() { self.view.flip = !self.view.flip; self.fit_view(); }
                ui.separator();
                ui.label("Layer");
                if ui.selectable_label(self.show.front, "F.Cu").clicked() { self.show.front = !self.show.front; }
                if ui.selectable_label(self.show.back, "B.Cu").clicked() { self.show.back = !self.show.back; }
                ui.separator();
                ui.label(format!("Grid {:.2} mm", self.grid_mm));
            });
        });
    }

    fn left_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("left").resizable(true).default_width(232.0).min_width(180.0).show(ctx, |ui| {
            ui.heading("Board");
            let current = self.file_path.as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "No board loaded".into());
            ui.label(RichText::new(current).small());
            if ui.button("Open board…").clicked() { self.open_file(); }
            ui.add_space(4.0);
            ui.label(RichText::new(format!("{} × {} mm", self.board.width_mm, self.board.height_mm)).small());

            ui.separator();
            ui.heading("Statistics");
            let stats = self.stats();
            ui.label(RichText::new(format!(
                "Copper {:.1} mm · segments {} · routed {}/{}",
                stats.copper_mm, stats.segments, stats.routed, stats.total
            )).small());
            if let Some((min_w, avg_w, max_w)) = stats.widths {
                ui.label(RichText::new(format!(
                    "Track width min {:.2} / avg {:.2} / max {:.2} mm",
                    min_w, avg_w, max_w
                )).small());
            }
            if let Some(field) = &self.thermal_field {
                let avg_c = if field.temps.is_empty() {
                    0.0
                } else {
                    field.temps.iter().sum::<f32>() as f64 / field.temps.len() as f64
                };
                ui.label(RichText::new(format!(
                    "Thermal peak {:.1} °C · average {:.1} °C · ambient {:.1} °C",
                    field.peak_rise_c, avg_c, field.ambient_c
                )).small());
            }
            egui::ScrollArea::vertical().id_salt("stats").max_height(140.0).show(ui, |ui| {
                for row in &stats.nets {
                    ui.label(format!(
                        "{}  {:.1} mm · {} seg · {:.2} mm",
                        row.name, row.length_mm, row.segments, row.width_min
                    ));
                }
            });
            if ui.button("Copy stats").clicked() {
                let mut text = format!(
                    "Copper {:.1} mm · segments {} · routed {}/{}",
                    stats.copper_mm, stats.segments, stats.routed, stats.total
                );
                for row in &stats.nets {
                    text.push_str(&format!(
                        "\n{}  {:.1} mm · {} seg · {:.2} mm",
                        row.name, row.length_mm, row.segments, row.width_min
                    ));
                }
                ctx.copy_text(text);
            }

            ui.separator();
            ui.heading("Export");
            let dir = self.output_dir();
            ui.label(RichText::new(dir.display().to_string()).small());
            if ui.button("Choose folder…").clicked() { self.choose_export_dir(); }

            ui.separator();
            ui.heading("Appearance");
            let layers = [
                ("F.Cu", &mut self.show.front, FRONT_CU),
                ("B.Cu", &mut self.show.back, BACK_CU),
                ("Pads", &mut self.show.pads, PAD),
                ("Components", &mut self.show.components, BODY_EDGE),
                ("Silkscreen", &mut self.show.silk, SILK),
                ("Edge.Cuts", &mut self.show.copper, SUBSTRATE_EDGE),
                ("Grid", &mut self.show.grid, GRID),
                ("Keep-outs", &mut self.show.keepouts, Color32::from_rgb(220, 100, 80)),
                ("Heatmap", &mut self.show.heatmap, Color32::from_rgb(230, 160, 60)),
            ];
            for (name, value, color) in layers {
                ui.horizontal(|ui| {
                    ui.checkbox(value, "");
                    ui.colored_label(color, name);
                });
            }
            ui.add_space(4.0);
            ui.label("Rotation");
            ui.add(egui::Slider::new(&mut self.view.angle, 0.0..=360.0).suffix("°"));
            if ui.button("Mirror horizontally").clicked() {
                self.view.flip = !self.view.flip;
                self.fit_view();
            }

            ui.separator();
            ui.heading("Nets");
            egui::ScrollArea::vertical().id_salt("nets").max_height(220.0).show(ui, |ui| {
                for net in &self.board.nets {
                    let selected = matches!(&self.selection, Some(Selection::Net(n)) if n == &net.name);
                    let routed = self.copper.routed_nets.contains(&net.name);
                    let color = net_color(net.class);
                    if ui.selectable_label(selected, RichText::new(&net.name).color(color)).clicked() {
                        self.selection = Some(Selection::Net(net.name.clone()));
                    }
                    if routed {
                        ui.colored_label(Color32::from_rgb(120, 210, 150), "  routed");
                    }
                }
            });

            ui.separator();
            ui.heading("Components");
            egui::ScrollArea::vertical().id_salt("components").show(ui, |ui| {
                for fp in &self.board.footprints {
                    let selected = matches!(&self.selection, Some(Selection::Footprint(r)) if r == &fp.refdes);
                    if ui.selectable_label(selected, format!("{}  {}", fp.refdes, fp.value)).clicked() {
                        self.selection = Some(Selection::Footprint(fp.refdes.clone()));
                    }
                }
            });
        });
    }

    fn right_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::right("inspector").resizable(true).default_width(268.0).min_width(200.0).show(ctx, |ui| {
            ui.heading("Inspector");
            ui.separator();
            match &self.selection {
                None => { ui.label("Select a pad, track, component or net"); }
                Some(Selection::Net(name)) => {
                    ui.label(RichText::new(name).strong());
                    let net = self.board.nets.iter().find(|n| &n.name == name);
                    if let Some(net) = net {
                        ui.label(format!("Class: {:?}", net.class));
                        ui.label(format!("Pads: {}", net.nodes.len()));
                    }
                    let segs: Vec<&RouteSegment> = self.copper.segments.iter().filter(|s| &s.net == name).collect();
                    ui.label(format!("Segments: {}", segs.len()));
                    ui.label(format!("Copper: {:.1} mm", segs.iter().map(|s| s.length_mm()).sum::<f64>()));
                }
                Some(Selection::Footprint(refdes)) => {
                    if let Some(fp) = self.board.footprint(refdes) {
                        ui.label(RichText::new(&fp.refdes).strong());
                        ui.label(format!("Value: {}", fp.value));
                        ui.label(format!("Class: {:?}", fp.class));
                        ui.label(format!("Position: ({:.2}, {:.2}) mm", fp.x, fp.y));
                        ui.label(format!("Rotation: {:.1}°", fp.rotation_deg));
                        ui.label(format!("Pads: {}", fp.pads.len()));
                    }
                }
                Some(Selection::Pad(refdes, index)) => {
                    if let Some(fp) = self.board.footprint(refdes) {
                        ui.label(RichText::new(format!("{refdes} · pad {index}")).strong());
                        if let Some(p) = fp.pad_world(*index) {
                            ui.label(format!("Position: ({:.2}, {:.2}) mm", p.0, p.1));
                        }
                        if let Some(pad) = fp.pads.get(*index) {
                            ui.label(format!("Diameter: {:.2} mm", pad.size_mm));
                            ui.label(format!("Drill: {:.2} mm", pad.drill_mm));
                        }
                        let owner = self.board.nets.iter().find(|n| n.nodes.iter().any(|r| r.refdes == *refdes && r.pad == *index));
                        if let Some(net) = owner { ui.label(format!("Net: {}", net.name)); }
                    }
                }
                Some(Selection::Segment(i)) => {
                    if let Some(seg) = self.copper.segments.get(*i) {
                        ui.label(RichText::new(format!("Track #{i}")).strong());
                        ui.label(format!("Net: {}", seg.net));
                        ui.label(format!("Class: {:?}", seg.class));
                        ui.label(format!("Layer: {}", seg.layer.kicad_name()));
                        ui.label(format!("Width: {:.2} mm", seg.width_mm));
                        ui.label(format!("Length: {:.2} mm", seg.length_mm()));
                    }
                }
                Some(Selection::Hole(i)) => {
                    if let Some(h) = self.board.mounting_holes.get(*i) {
                        ui.label(RichText::new(format!("Mounting hole #{i}")).strong());
                        ui.label(format!("Position: ({:.2}, {:.2}) mm", h.x, h.y));
                        ui.label(format!("Drill: {:.2} mm", h.drill_mm));
                    }
                }
            }

            ui.add_space(8.0);
            ui.heading("Route report");
            ui.separator();
            ui.label(self.stats_text());

            ui.add_space(8.0);
            ui.heading("DRC");
            ui.separator();
            if let Some(pipe) = &self.pipe {
                ui.colored_label(if pipe.drc.is_ok() { Color32::from_rgb(120, 220, 150) } else { Color32::from_rgb(235, 120, 100) }, pipe.drc.summary());
                egui::ScrollArea::vertical().id_salt("drc").max_height(150.0).show(ui, |ui| {
                    for v in &pipe.drc.violations {
                        ui.label(RichText::new(format!("{}: {}", v.rule, v.message)).small().color(Color32::from_rgb(235, 160, 120)));
                    }
                });
            } else {
                ui.label("No run yet");
            }

            ui.add_space(8.0);
            ui.heading("Process");
            ui.separator();
            ui.label(format!("Grid: {:.2} mm", self.grid_mm));
            ui.add(egui::Slider::new(&mut self.grid_mm, 0.2..=1.0).text("Routing grid"));
            ui.checkbox(&mut self.run_kicad_cli_drc, "Run kicad-cli DRC");
            ui.checkbox(&mut self.two_layer, "Two-layer board (vias allowed)");
            ui.label("LUT preset");
            ui.label(format!("{} · min track {:.2} mm · min clearance {:.2} mm",
                self.board.preset.name, self.board.preset.min_track_mm, self.board.preset.min_clearance_mm));
        });
    }

    fn stats_text(&self) -> String {
        if let Some(pipe) = &self.pipe {
            let thermal = &pipe.thermal;
            format!(
                "Nets routed: {}/{}\nCopper: {:.1} mm\nFixed rails: {:.1} mm\nVias: {}\nTime: {:.1} s\nPeak ΔT: {:.1} °C\nModules: {} footprints, {} mounting holes\nResult: {}",
                pipe.routing.result.routed_nets.len(),
                pipe.nets,
                pipe.final_copper.total_length_mm,
                pipe.fixed_copper_mm,
                pipe.final_copper.vias,
                pipe.elapsed_ms as f64 / 1000.0,
                thermal.peak_rise_c,
                pipe.footprints,
                pipe.mounting_holes,
                if pipe.ok { "PASS" } else { "INCOMPLETE" }
            )
        } else {
            format!(
                "Nets: {}\nPads: {}\nFootprints: {}\nCopper: not routed yet",
                self.board.nets.len(),
                self.board.all_pads().len(),
                self.board.footprints.len()
            )
        }
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let stage = match self.stage {
                    Stage::Ready => "READY",
                    Stage::Routing => "ROUTING",
                    Stage::Done => "DONE",
                    Stage::Incomplete => "INCOMPLETE",
                };
                ui.colored_label(match self.stage {
                    Stage::Done => Color32::from_rgb(120, 220, 150),
                    Stage::Incomplete => Color32::from_rgb(235, 120, 100),
                    _ => Color32::from_rgb(160, 190, 220),
                }, stage);
                ui.separator();
                ui.label(&self.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some((x, y)) = self.hover {
                        ui.label(format!("{x:.1}, {y:.1} mm"));
                    }
                    ui.label(format!("{}°", self.view.angle as i32));
                    ui.label(format!("Zoom {:.1}", self.view.zoom));
                });
            });
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        let mut close = false;
        if let Some(dialog) = &self.dialog {
            match dialog {
                Dialog::About => {
                    egui::Window::new("About AmpRoute Studio").collapsible(false).resizable(false).show(ctx, |ui| {
                        ui.label("AmpRoute Studio 2.2.0 — agent-first autorouter and 2-D board editor.");
                        ui.label("Statistics panel, Ctrl+Z / Ctrl+Y undo of routing runs — 15 reference boards · 100% routing · zero KiCad DRC violations.");
                        ui.label("LUT Gerber/1:1 template, IPC-2581, ODB++, CLI --json, MCP stdio.");
                        ui.label("MIT license · Rust + egui · github.com/pstuha/audio-amp-autorouter");
                        if ui.button("Close").clicked() { close = true; }
                    });
                }
                Dialog::Hotkeys => {
                    egui::Window::new("Hotkeys").collapsible(false).resizable(false).show(ctx, |ui| {
                        for (key, action) in [
                            ("F5", "Route the board"),
                            ("Esc", "Stop routing"),
                            ("Ctrl+O", "Open a board"),
                            ("Ctrl+K", "Export .kicad_pcb"),
                            ("Ctrl+E", "Export Gerber / SVG / PDF / LUT template"),
                            ("Ctrl+Z / Ctrl+Y", "Undo / redo the last routing run"),
                            ("R / Shift+R", "Rotate the view ±90°"),
                            ("F", "Mirror the board"),
                            ("Home", "Zoom to fit"),
                            ("Wheel / drag", "Zoom / pan"),
                        ] {
                            ui.horizontal(|ui| { ui.label(RichText::new(key).monospace().strong()); ui.label(action); });
                        }
                        if ui.button("Close").clicked() { close = true; }
                    });
                }
                Dialog::Message(text) => {
                    let text = text.clone();
                    egui::Window::new("Result").collapsible(false).resizable(false).show(ctx, |ui| {
                        ui.label(&text);
                        if ui.button("Close").clicked() { close = true; }
                    });
                }
            }
        }
        if close { self.dialog = None; }
    }
}

fn simulate(board: &Board) -> ThermalField {
    thermal::solve(board, &thermal::sources_for(board), &ThermalParams::default())
}

fn net_color(class: NetClass) -> Color32 {
    let (r, g, b) = class.rgb();
    Color32::from_rgb(r, g, b)
}

fn distance_to_segment(p: (f64, f64), seg: &RouteSegment) -> f64 {
    let (x1, y1, x2, y2) = (seg.x1_mm, seg.y1_mm, seg.x2_mm, seg.y2_mm);
    let dx = x2 - x1;
    let dy = y2 - y1;
    let len2 = dx * dx + dy * dy;
    if len2 <= 1e-12 {
        return (p.0 - x1).hypot(p.1 - y1);
    }
    let t = (((p.0 - x1) * dx + (p.1 - y1) * dy) / len2).clamp(0.0, 1.0);
    (p.0 - (x1 + t * dx)).hypot(p.1 - (y1 + t * dy))
}

impl eframe::App for AmpRouteApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.menu_bar(ctx);
        self.left_panel(ctx);
        self.right_panel(ctx);
        self.status_bar(ctx);
        egui::CentralPanel::default().show(ctx, |ui| self.draw_canvas(ui));
        self.poll_route(ctx);
        self.dialogs(ctx);
    }
}

/// Launch the GUI. Blocks until the window is closed.
pub fn run() -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1500.0, 940.0])
            .with_min_inner_size([1100.0, 700.0])
            .with_title("AmpRoute Studio"),
        ..Default::default()
    };
    eframe::run_native(
        "AmpRoute Studio",
        options,
        Box::new(|cc| Ok(Box::new(AmpRouteApp::new(cc)))),
    )
    .map_err(|e| anyhow::anyhow!("GUI error: {e}"))
}

/// Segments of a net, for tests.
pub fn segments_of<'a>(result: &'a RoutingResult, net: &str) -> Vec<&'a RouteSegment> {
    result.segments.iter().filter(|s| s.net == net).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boards::BOARD_SPECS;
    use crate::router::Router;

    #[test]
    fn auto_placement_routes_most_nets() {
        let board = (BOARD_SPECS[0].build)();
        let router = Router::new(0.5, board.width_mm, board.height_mm);
        let r = router.route_board(&board);
        let total = r.routed_nets.len() + r.failed_nets.len();
        assert!(total > 0);
        assert!(
            r.routed_nets.len() as f32 / total as f32 > 0.7,
            "should route most nets"
        );
    }

    #[test]
    fn segments_have_positive_length() {
        let board = (BOARD_SPECS[0].build)();
        let router = Router::new(0.5, board.width_mm, board.height_mm);
        let r = router.route_board(&board);
        assert!(r.segments.iter().all(|s| s.length_mm() > 0.0));
    }

    #[test]
    fn view_rotation_round_trips_coordinates() {
        let app = AmpRouteApp {
            board: (BOARD_SPECS[0].build)(),
            file_path: None,
            export_dir: None,
            copper: RoutingResult::new(),
            pipe: None,
            stage: Stage::Ready,
            status: String::new(),
            route_job: None,
            route_started: None,
            view: View { zoom: 4.0, pan: Vec2::new(10.0, -5.0), angle: 37.0, flip: true },
            selection: None,
            hover: None,
            grid_mm: 0.5,
            time_budget_s: 300.0,
            run_kicad_cli_drc: false,
            two_layer: false,
            show: Appearance::default(),
            dialog: None,
            thermal_field: None,
            history: History::default(),
            canvas_size: Vec2::new(800.0, 600.0),
        };
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        let point = (12.3, 8.7);
        let screen = app.to_screen(rect, point);
        let back = app.to_board(rect, screen);
        assert!((back.0 - point.0).abs() < 1e-3);
        assert!((back.1 - point.1).abs() < 1e-3);
    }

    #[test]
    fn history_undo_redo_route_runs() {
        let mut h = History::default();
        let mut cur = RoutingResult::new();
        assert!(cur.routed_nets.is_empty());

        // First route run: previous state (empty copper) goes to `past`.
        h.push(cur.clone());
        cur.routed_nets.push("A".to_string());

        assert!(h.undo(&mut cur));
        assert!(cur.routed_nets.is_empty());
        assert!(h.redo(&mut cur));
        assert_eq!(cur.routed_nets, vec!["A".to_string()]);

        // A new run after undo/redo pushes the current state again.
        h.push(cur.clone());
        assert_eq!(h.past.len(), 2);
        cur.routed_nets.clear();

        assert!(h.undo(&mut cur));
        assert_eq!(cur.routed_nets, vec!["A".to_string()]);
        assert!(h.undo(&mut cur));
        assert!(cur.routed_nets.is_empty());
        assert!(h.past.is_empty());
    }
}
