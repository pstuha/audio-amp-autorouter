//! AmpRoute Studio — professional 2-D board viewer and router front-end.
//!
//! Layout: left panel (status + controls), central board canvas with zoom/pan,
//! right statistics/DRC panel, bottom status bar.

use crate::board::Board;
use crate::board::NetClass;
use crate::export;
use crate::import;
use crate::kicad;
use crate::router::{RouteSegment, Router, RoutingResult};
use crate::thermal::{self, ThermalParams};
use eframe::egui;
use egui::{Color32, FontId, Pos2, Rect, RichText, Stroke, Vec2};
use std::path::PathBuf;

/// Persistent application state.
pub struct AmpRouteApp {
    board: Board,
    result: RoutingResult,
    drc: Vec<String>,
    zoom: f32,
    pan: Vec2,
    show_components: bool,
    show_pads: bool,
    show_grid: bool,
    show_copper: bool,
    show_thermal: bool,
    show_heatmap: bool,
    selected_net: Option<String>,
    status: String,
    grid_mm: f64,
    routing_ms: f64,
    thermal_field: Option<thermal::ThermalField>,
    file_path: Option<PathBuf>,
    /// A routing pass running on a worker thread, if any.
    route_job: Option<std::sync::mpsc::Receiver<RouteOutcome>>,
    /// When the current pass started, for the elapsed-time readout.
    route_started: Option<std::time::Instant>,
    stage: RouteStage,
}

/// Everything a background routing pass produces.
struct RouteOutcome {
    result: RoutingResult,
    thermal: thermal::ThermalField,
    drc: Vec<String>,
    elapsed_ms: f64,
}

/// What the router is doing right now, so the window never looks frozen.
#[derive(Clone, Copy, PartialEq)]
enum RouteStage {
    /// No board loaded yet.
    Idle,
    /// Board loaded, nothing routed.
    Ready,
    /// A routing pass is in progress.
    Routing,
    /// Routing finished.
    Done,
    /// Routing finished with nets left over.
    Incomplete,
}

/// Run the thermal simulation for a board with its own heat sources.
fn simulate(board: &Board) -> thermal::ThermalField {
    let sources = thermal::sources_for(board);
    thermal::solve(board, &sources, &ThermalParams::default())
}

impl AmpRouteApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Dark, quiet interface: the board is the only thing that carries colour.
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = Color32::from_rgb(30, 30, 33);
        visuals.window_fill = Color32::from_rgb(36, 36, 40);
        visuals.extreme_bg_color = Color32::from_rgb(20, 20, 23);
        visuals.faint_bg_color = Color32::from_rgb(40, 40, 45);
        visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(42, 42, 47);
        visuals.widgets.inactive.bg_fill = Color32::from_rgb(52, 52, 58);
        visuals.widgets.inactive.fg_stroke =
            Stroke::new(1.0_f32, Color32::from_rgb(216, 216, 224));
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(66, 66, 74);
        visuals.widgets.active.bg_fill = Color32::from_rgb(84, 148, 112);
        visuals.selection.bg_fill = Color32::from_rgb(56, 132, 94);
        visuals.selection.stroke = Stroke::new(1.0_f32, Color32::from_rgb(190, 235, 210));
        visuals.hyperlink_color = Color32::from_rgb(120, 190, 235);
        cc.egui_ctx.set_visuals(visuals);

        let app = Self {
            result: RoutingResult::new(),
            drc: Vec::new(),
            board: Board::new("Empty", 100.0, 80.0, crate::board::LutPreset::reliable()),
            zoom: 3.2,
            pan: Vec2::new(40.0, 40.0),
            show_components: true,
            show_pads: true,
            show_grid: true,
            show_copper: true,
            show_thermal: false,
            show_heatmap: false,
            selected_net: None,
            status: "Open a .kicad_pcb file to begin".into(),
            grid_mm: 0.5,
            routing_ms: 0.0,
            thermal_field: None,
            file_path: None,
            route_job: None,
            route_started: None,
            stage: RouteStage::Idle,
        };
        app
    }

    fn route(&mut self) {
        if self.route_job.is_some() {
            return;
        }
        if self.board.nets.is_empty() {
            self.status =
                "Nothing to route: no net is assigned to a pad in this file".into();
            self.stage = RouteStage::Incomplete;
            self.drc = vec!["Board has no nets — check that pads carry net assignments".into()];
            return;
        }

        // Routing a large board takes seconds. Run it on a worker so the window
        // keeps drawing and the stage readout can show what is happening.
        let board = self.board.clone();
        let grid_mm = self.grid_mm;
        let (tx, rx) = std::sync::mpsc::channel();

        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            let mut router = Router::new(grid_mm, board.width_mm, board.height_mm);
            for (x, y, r) in board.thermal_hotspots() {
                router.add_penalty_circle(x, y, r);
            }
            if let Some((x, y)) = board.star_ground_point() {
                router.add_penalty_circle(x, y, 8.0);
            }
            let result = router.route_board(&board);

            let mut drc = board.validate();
            drc.extend(
                crate::placement::PlacementResult {
                    board: board.clone(),
                }
                .drc_report(board.preset.min_clearance_mm),
            );

            let sources = thermal::sources_for(&board);
            let thermal = thermal::solve(&board, &sources, &ThermalParams::default());

            let outcome = RouteOutcome {
                result,
                thermal,
                drc,
                elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
            };
            // The receiver is gone only if the app closed; dropping is fine.
            let _ = tx.send(outcome);
        });

        self.route_job = Some(rx);
        self.route_started = Some(std::time::Instant::now());
        self.stage = RouteStage::Routing;
        self.status = "Routing · placing copper on nets".into();
    }

    /// Collect the result of a finished routing pass, if it has arrived.
    fn poll_route(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.route_job else { return };
        // `try_recv` keeps the UI responsive; a repaint is requested below while
        // the job is still running.
        let Ok(outcome) = rx.try_recv() else {
            ctx.request_repaint_after(std::time::Duration::from_millis(60));
            return;
        };

        self.result = outcome.result;
        self.thermal_field = Some(outcome.thermal);
        self.drc = outcome.drc;
        self.route_started = None;
        self.route_job = None;

        if self.result.failed_nets.is_empty() {
            self.stage = RouteStage::Done;
            self.status = format!(
                "Router · done — {}/{} nets in {:.0} ms, {:.1} mm of copper",
                self.result.routed_nets.len(),
                self.result.total_nets(),
                outcome.elapsed_ms,
                self.result.total_length_mm
            );
        } else {
            self.stage = RouteStage::Incomplete;
            self.status = format!(
                "Router · {}/{} nets in {:.0} ms — {} net(s) could not be placed",
                self.result.routed_nets.len(),
                self.result.total_nets(),
                outcome.elapsed_ms,
                self.result.failed_nets.len()
            );
        }
    }

    fn export_kicad(&mut self) {
        let fab = kicad::FabOptions::default();
        let name = self.board.name.replace(' ', "_").to_lowercase();
        let path = format!("{name}.kicad_pcb");
        match kicad::write_kicad_pcb(&self.board, &self.result, &fab, &path) {
            Ok(()) => {
                self.status = format!("Exported KiCad board to {path}");
            }
            Err(e) => {
                self.status = format!("KiCad export failed — {e}");
            }
        }
    }

    fn export_all(&mut self) {
        match export::write_all(&self.board, &self.result, "output") {
            Ok(files) => {
                self.status = format!(
                    "Exported {} files to ./output: {}",
                    files.len(),
                    files.join(", ")
                )
            }
            Err(e) => self.status = format!("Export blocked — {e}"),
        }
    }

    fn load_from_file(&mut self, path: PathBuf) {
        let data = match std::fs::read_to_string(&path) {
            Ok(d) => d,
            Err(e) => {
                self.status = format!("Cannot read {}: {e}", path.display());
                return;
            }
        };
        // Try JSON first (our own format), then KiCad PCB. A KiCad board takes
        // the file's name, so re-exporting it does not lose that.
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "Imported board".into());
        let board = if data.trim_start().starts_with('{') {
            Board::from_json(&data).ok()
        } else {
            None
        }
        .or_else(|| {
            import::import_kicad_pcb_str_named(&data, &stem)
                .ok()
                .map(|r| r.board)
        });

        let Some(board) = board else {
            self.status = "Failed to parse file as JSON or KiCad PCB".into();
            return;
        };

        self.board = board;
        self.grid_mm = self.board.preset.grid_mm;
        self.file_path = Some(path);
        self.selected_net = None;
        self.zoom = 3.2;
        self.pan = Vec2::new(40.0, 40.0);
        // A fresh board has no copper yet: drop whatever the previous one left.
        self.result = RoutingResult::new();
        self.drc.clear();
        self.thermal_field = Some(simulate(&self.board));
        self.route_job = None;
        self.route_started = None;
        self.stage = RouteStage::Ready;
        self.status = format!(
            "Loaded · {} · {} nets · {} pads · press Route PCB",
            self.board.name,
            self.board.nets.len(),
            self.board.all_pads().len()
        );
    }

    fn net_color(&self, name: &str) -> Color32 {
        let class = self
            .board
            .nets
            .iter()
            .find(|n| n.name == name)
            .map(|n| n.class)
            .unwrap_or(NetClass::Signal);
        let (r, g, b) = class.rgb();
        Color32::from_rgb(r, g, b)
    }

    /// Convert board coordinates to screen coordinates.
    fn to_screen(&self, rect: Rect, p: (f64, f64)) -> Pos2 {
        Pos2::new(
            rect.min.x + self.pan.x + p.0 as f32 * self.zoom,
            rect.min.y + self.pan.y + p.1 as f32 * self.zoom,
        )
    }

    fn draw_heatmap(&self, painter: &egui::Painter, rect: Rect) {
        let Some(field) = &self.thermal_field else { return };
        if field.peak_rise_c <= 0.0 {
            return;
        }

        let cell = field.cell_mm as f32 * self.zoom;
        let origin = self.to_screen(rect, (0.0, 0.0));

        // One translucent quad per cell, coloured by the solved temperature rise.
        for row in 0..field.rows {
            for col in 0..field.cols {
                let rise = field.temps[row * field.cols + col] as f64;
                if rise <= 0.0 {
                    continue;
                }
                let (r, g, b) = thermal::colour_for_rise(rise);
                let tint = Color32::from_rgba_unmultiplied(r, g, b, 150);
                let min = Pos2::new(origin.x + col as f32 * cell, origin.y + row as f32 * cell);
                painter.rect_filled(Rect::from_min_size(min, Vec2::splat(cell)), 0.0, tint);
            }
        }
    }

    fn draw_canvas(&mut self, ui: &mut egui::Ui) {
        let (resp, painter) =
            ui.allocate_painter(ui.available_size(), egui::Sense::click_and_drag());
        let rect = resp.rect;

        // Pan with drag, zoom with wheel.
        if resp.dragged() {
            self.pan += resp.drag_delta();
        }
        if resp.hovered() {
            let scroll = ui.input(|i| i.raw_scroll_delta.y);
            if scroll != 0.0 {
                let factor = if scroll > 0.0 { 1.1 } else { 1.0 / 1.1 };
                let pivot = resp.hover_pos().unwrap_or(rect.center());
                let rel = pivot - rect.min;
                self.zoom = (self.zoom * factor).clamp(0.5_f32, 40.0_f32);
                self.pan = rel - (rel - self.pan) * factor;
            }
        }

        // Background.
        painter.rect_filled(rect, 0.0, Color32::from_rgb(10, 16, 13));

        let origin = self.to_screen(rect, (0.0, 0.0));
        let far = self.to_screen(rect, (self.board.width_mm, self.board.height_mm));
        let board_rect = Rect::from_min_max(origin, far);

        // PCB substrate.
        painter.rect_filled(board_rect, 2.0, Color32::from_rgb(14, 48, 32));
        painter.rect_stroke(
            board_rect,
            2.0,
            Stroke::new(1.6_f32, Color32::from_rgb(95, 224, 138)),
        );

        // Grid.
        if self.show_grid {
            let step = 10.0;
            let mut x = 0.0;
            while x <= self.board.width_mm {
                let a = self.to_screen(rect, (x, 0.0));
                let b = self.to_screen(rect, (x, self.board.height_mm));
                painter.line_segment([a, b], Stroke::new(0.5_f32, Color32::from_rgb(26, 68, 44)));
                x += step;
            }
            let mut y = 0.0;
            while y <= self.board.height_mm {
                let a = self.to_screen(rect, (0.0, y));
                let b = self.to_screen(rect, (self.board.width_mm, y));
                painter.line_segment([a, b], Stroke::new(0.5_f32, Color32::from_rgb(26, 68, 44)));
                y += step;
            }
        }

        // Heatmap (real thermal simulation).
        if self.show_heatmap {
            self.draw_heatmap(&painter, rect);
        }

        // Thermal and star-ground keepout zones (soft zones overlay).
        if self.show_thermal {
            for (x, y, r) in self.board.thermal_hotspots() {
                let c = self.to_screen(rect, (x, y));
                painter.circle_filled(
                    c,
                    (r as f32) * self.zoom,
                    Color32::from_rgba_unmultiplied(235, 90, 80, 40),
                );
                painter.circle_stroke(
                    c,
                    (r as f32) * self.zoom,
                    Stroke::new(1.0_f32, Color32::from_rgb(200, 80, 70)),
                );
            }
            if let Some((x, y)) = self.board.star_ground_point() {
                let c = self.to_screen(rect, (x, y));
                painter.circle_filled(
                    c,
                    8.0 * self.zoom,
                    Color32::from_rgba_unmultiplied(60, 200, 110, 36),
                );
                painter.circle_stroke(
                    c,
                    8.0 * self.zoom,
                    Stroke::new(1.0_f32, Color32::from_rgb(60, 200, 110)),
                );
            }
        }

        // Component footprints.
        if self.show_components {
            for fp in &self.board.footprints {
                let (x0, y0, x1, y1) = fp.rect();
                let a = self.to_screen(rect, (x0, y0));
                let b = self.to_screen(rect, (x1, y1));
                let r = Rect::from_min_max(a, b);
                painter.rect_filled(r, 1.5, Color32::from_rgba_unmultiplied(120, 170, 140, 22));
                painter.rect_stroke(
                    r,
                    1.5,
                    Stroke::new(1.0_f32, Color32::from_rgb(120, 185, 150)),
                );
                if self.zoom > 2.0 {
                    painter.text(
                        r.center(),
                        egui::Align2::CENTER_CENTER,
                        &fp.refdes,
                        FontId::monospace(9.0),
                        Color32::from_rgb(205, 235, 215),
                    );
                }
            }
        }

        // Mounting holes.
        if self.show_components {
            for hole in &self.board.mounting_holes {
                let c = self.to_screen(rect, (hole.x, hole.y));
                let r = (hole.drill_mm as f32 / 2.0 + 1.2) * self.zoom;
                painter.circle_stroke(
                    c,
                    r.max(2.0),
                    Stroke::new(1.0_f32, Color32::from_rgb(180, 140, 90)),
                );
                painter.circle_filled(
                    c,
                    (hole.drill_mm as f32 / 2.0 * self.zoom).max(1.0),
                    Color32::from_rgb(10, 16, 13),
                );
            }
        }

        // Tracks.
        if self.show_copper {
            for seg in &self.result.segments {
                let dim = self.selected_net.is_some()
                    && self.selected_net.as_deref() != Some(seg.net.as_str());
                let mut col = self.net_color(&seg.net);
                if dim {
                    col = col.gamma_multiply(0.25);
                }
                let a = self.to_screen(rect, (seg.x1_mm, seg.y1_mm));
                let b = self.to_screen(rect, (seg.x2_mm, seg.y2_mm));
                let w = (seg.width_mm as f32 * self.zoom).max(1.2);
                painter.line_segment([a, b], Stroke::new(w, col));
            }
        }

        // Pads.
        if self.show_pads {
            for ((x, y), size, _net) in self.board.all_pads() {
                let c = self.to_screen(rect, (x, y));
                let r = ((size as f32 / 2.0) * self.zoom).max(1.8);
                painter.circle_filled(c, r, Color32::from_rgb(200, 165, 58));
                painter.circle_stroke(c, r, Stroke::new(0.8_f32, Color32::from_rgb(245, 220, 120)));
                let hole = (r * 0.42).max(0.8);
                painter.circle_filled(c, hole, Color32::from_rgb(10, 16, 13));
            }
        }

        // Hover coordinate readout.
        if let Some(hp) = resp.hover_pos() {
            let bx = (hp.x - origin.x) / self.zoom;
            let by = (hp.y - origin.y) / self.zoom;
            if bx >= 0.0
                && by >= 0.0
                && bx <= self.board.width_mm as f32
                && by <= self.board.height_mm as f32
            {
                painter.text(
                    rect.left_bottom() + Vec2::new(8.0, -8.0),
                    egui::Align2::LEFT_BOTTOM,
                    format!("X {bx:.1}  Y {by:.1} mm"),
                    FontId::monospace(11.0),
                    Color32::from_rgb(150, 200, 170),
                );
            }
        }
    }

    fn left_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);

        // The board comes from a file. There is no built-in board list: the
        // program is a router for your project, not a demo viewer.
        let board_label = self
            .file_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "No board loaded".into());
        ui.label(RichText::new(board_label).strong().size(13.0));
        ui.label(
            RichText::new(if self.file_path.is_some() {
                "Loaded from file"
            } else {
                "Open a .kicad_pcb to begin"
            })
            .size(11.0)
            .color(Color32::from_gray(150)),
        );

        ui.add_space(6.0);
        if ui
            .add_sized([ui.available_width(), 26.0], egui::Button::new("Open board…"))
            .clicked()
        {
            if let Some(path) = rfd::FileDialog::new()
                .add_filter("KiCad PCB", &["kicad_pcb"])
                .add_filter("JSON", &["json"])
                .pick_file()
            {
                self.load_from_file(path);
            }
        }

        ui.add_space(4.0);
        let can_route = !self.board.nets.is_empty();
        if ui
            .add_enabled(
                can_route,
                egui::Button::new(RichText::new("⚡ Route PCB").size(13.0))
                    .min_size(Vec2::new(ui.available_width(), 28.0)),
            )
            .clicked()
        {
            self.route();
        }
        if !can_route {
            ui.label(
                RichText::new("No nets to route — the file has no net assignments")
                    .size(10.0)
                    .color(Color32::from_rgb(220, 150, 100)),
            );
        }

        ui.add_space(8.0);
        ui.heading(RichText::new("Nets").size(14.0).strong());
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("nets")
            .max_height(ui.available_height() * 0.45)
            .show(ui, |ui| {
                let nets = self.board.nets.clone();
                for net in &nets {
                    let (r, g, b) = net.class.rgb();
                    let color = Color32::from_rgb(r, g, b);
                    let selected = self.selected_net.as_deref() == Some(net.name.as_str());
                    let routed = self.result.routed_nets.contains(&net.name);
                    let mark = if routed { "✓" } else { "✗" };
                    let label = format!("{mark}  {}  ·  {}", net.name, net.class.label());
                    let text = RichText::new(label).color(color).size(12.0);
                    if ui.selectable_label(selected, text).clicked() {
                        self.selected_net = if selected {
                            None
                        } else {
                            Some(net.name.clone())
                        };
                    }
                }
            });

        ui.add_space(6.0);
        ui.heading(RichText::new("Components").size(14.0).strong());
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("comps")
            .show(ui, |ui| {
                for fp in &self.board.footprints {
                    let (r, g, b) = fp.class.rgb();
                    ui.label(
                        RichText::new(format!(
                            "{:<6} {:<8} ({:.0},{:.0})",
                            fp.refdes, fp.value, fp.x, fp.y
                        ))
                        .monospace()
                        .size(11.0)
                        .color(Color32::from_rgb(r, g, b)),
                    );
                }
            });
    }

    fn right_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.heading(RichText::new("Statistics").size(14.0).strong());
        ui.separator();
        let total = self.result.routed_nets.len() + self.result.failed_nets.len();
        let pct = if total > 0 {
            self.result.routed_nets.len() as f32 / total as f32 * 100.0
        } else {
            0.0
        };
        ui.label(format!(
            "Nets routed: {}/{}  ({pct:.0}%)",
            self.result.routed_nets.len(),
            total
        ));
        ui.label(format!(
            "Copper:      {:.1} mm",
            self.result.total_length_mm
        ));
        ui.label(format!(
            "Board area:  {:.0} × {:.0} mm",
            self.board.width_mm, self.board.height_mm
        ));
        ui.label(format!("Components:  {}", self.board.footprints.len()));
        ui.label(format!("Mounting holes: {}", self.board.mounting_holes.len()));
        ui.label(format!("Router time: {:.0} ms", self.routing_ms));
        ui.add_space(4.0);
        ui.add(egui::ProgressBar::new(pct / 100.0).text(format!("{pct:.0}% routed")));

        // Peak temperature from simulation.
        if let Some(field) = &self.thermal_field {
            ui.add_space(4.0);
            ui.label(format!("Peak ΔT:  {:.1} °C", field.peak_rise_c));
            ui.label(format!("At:       ({:.1}, {:.1}) mm", field.peak_at.0, field.peak_at.1));
        }

        ui.add_space(8.0);
        ui.heading(RichText::new("Checks").size(14.0).strong());
        ui.separator();
        if self.drc.is_empty() {
            ui.label(
                RichText::new("✓ Model valid, placement clear")
                    .color(Color32::from_rgb(90, 220, 130)),
            );
        } else {
            for issue in &self.drc {
                ui.label(
                    RichText::new(format!("⚠ {issue}"))
                        .color(Color32::from_rgb(235, 180, 90))
                        .size(11.0),
                );
            }
        }
        if !self.result.failed_nets.is_empty() {
            ui.add_space(4.0);
            for failure in &self.result.failed_nets {
                ui.label(
                    RichText::new(format!("✗ {failure}"))
                        .color(Color32::from_rgb(235, 110, 100))
                        .size(11.0),
                );
            }
        }
        ui.label(
            RichText::new("Structural + placement checks only — not a manufacturing DRC.")
                .size(10.0)
                .color(Color32::from_gray(140)),
        );

        ui.add_space(8.0);
        ui.heading(RichText::new("LUT process").size(14.0).strong());
        ui.separator();
        ui.label(format!("Preset:      {}", self.board.preset.name));
        ui.label(format!(
            "Min track:   {:.2} mm",
            self.board.preset.min_track_mm
        ));
        ui.label(format!(
            "Min clear:   {:.2} mm",
            self.board.preset.min_clearance_mm
        ));
        ui.label(format!(
            "Min pad:     {:.2} mm",
            self.board.preset.min_pad_mm
        ));
        ui.label(format!("Max vias:    {}", self.board.preset.max_vias));
        ui.label(format!("Grid:        {:.1} mm", self.grid_mm));

        ui.add_space(8.0);
        ui.heading(RichText::new("Layers").size(14.0).strong());
        ui.separator();
        ui.checkbox(&mut self.show_copper, "Copper tracks");
        ui.checkbox(&mut self.show_components, "Components");
        ui.checkbox(&mut self.show_pads, "Pads");
        ui.checkbox(&mut self.show_grid, "10 mm grid");
        ui.checkbox(&mut self.show_thermal, "Soft zones (thermal / star-ground)");
        ui.checkbox(&mut self.show_heatmap, "Heatmap (real thermal sim)");

        if let Some(net) = &self.selected_net {
            ui.add_space(8.0);
            ui.heading(RichText::new("Selected net").size(14.0).strong());
            ui.separator();
            ui.label(RichText::new(net).monospace().strong());
            if let Some(n) = self.board.nets.iter().find(|n| &n.name == net) {
                ui.label(format!("Class:   {}", n.class.label()));
                ui.label(format!("Nodes:   {}", n.nodes.len()));
                ui.label(format!("Width:   {:.1} mm", n.class.track_width_mm()));
                ui.label(format!("Priority: {}", n.class.priority()));
            }
            let len: f64 = self
                .result
                .segments
                .iter()
                .filter(|s| &s.net == net)
                .map(|s| s.length_mm())
                .sum();
            ui.label(format!("Routed:  {:.1} mm", len));
        }
    }
}

impl eframe::App for AmpRouteApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Keyboard shortcuts.
        if ctx.input(|i| i.key_pressed(egui::Key::F5)) {
            self.route();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::E) && i.modifiers.ctrl) {
            self.export_all();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::K) && i.modifiers.ctrl) {
            self.export_kicad();
        }

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open .kicad_pcb / JSON…").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("KiCad PCB", &["kicad_pcb"])
                            .add_filter("JSON", &["json"])
                            .pick_file()
                        {
                            self.load_from_file(path);
                        }
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("Help", |ui| {
                    ui.label(RichText::new("AmpRoute Studio").strong());
                    ui.label(format!("v{}", crate::VERSION));
                    ui.label("Autorouter for LUT audio amplifier PCBs");
                });
            });
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(&self.status)
                        .size(11.0)
                        .color(Color32::from_rgb(150, 200, 170)),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(format!("zoom {:.1}×", self.zoom)).size(11.0));
                });
            });
        });

        egui::SidePanel::left("inspector")
            .resizable(true)
            .default_width(240.0)
            .show(ctx, |ui| {
                self.left_panel(ui);
            });

        egui::SidePanel::right("stats")
            .resizable(true)
            .default_width(250.0)
            .show(ctx, |ui| {
                self.right_panel(ui);
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            self.draw_canvas(ui);
        });

        // While a routing pass runs, poll for its result and keep the window alive.
        self.poll_route(ctx);
    }
}

/// Launch the GUI. Blocks until the window is closed.
pub fn run() -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([960.0, 640.0])
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
}