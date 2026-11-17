//! AmpRoute Studio — professional 2-D board viewer and router front-end.
//!
//! Layout: menu bar, left inspector (nets + components), central board canvas
//! with zoom/pan, right statistics/DRC panel, bottom status bar.

use crate::board::{Board, NetClass};
use crate::export;
use crate::placement::PlacementResult;
use crate::router::{RouteSegment, Router, RoutingResult};
use eframe::egui;
use egui::{Color32, FontId, Pos2, Rect, RichText, Stroke, Vec2};

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
    selected_net: Option<String>,
    status: String,
    grid_mm: f64,
    routing_ms: f64,
}

impl AmpRouteApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Dark, professional theme.
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = Color32::from_rgb(18, 26, 22);
        visuals.window_fill = Color32::from_rgb(22, 32, 27);
        visuals.extreme_bg_color = Color32::from_rgb(12, 18, 15);
        visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(28, 40, 33);
        visuals.widgets.inactive.bg_fill = Color32::from_rgb(34, 48, 40);
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(46, 66, 54);
        visuals.widgets.active.bg_fill = Color32::from_rgb(58, 82, 66);
        visuals.selection.bg_fill = Color32::from_rgb(60, 120, 85);
        cc.egui_ctx.set_visuals(visuals);

        let placement = PlacementResult::auto_amplifier();
        let board = placement.board;
        let grid_mm = board.preset.grid_mm;
        let mut app = Self {
            result: RoutingResult::new(),
            drc: Vec::new(),
            board,
            zoom: 3.2,
            pan: Vec2::new(40.0, 40.0),
            show_components: true,
            show_pads: true,
            show_grid: true,
            show_copper: true,
            show_thermal: false,
            selected_net: None,
            status: "Ready — press Route (F5) to autoroute the board".into(),
            grid_mm,
            routing_ms: 0.0,
        };
        app.route();
        app
    }

    /// Run the router on the current board and update the diagnostics panels.
    fn route(&mut self) {
        let started = std::time::Instant::now();
        let mut router = Router::new(self.grid_mm, self.board.width_mm, self.board.height_mm);
        // Thermal hotspots (ТЗ 5.3).
        for (x, y, r) in self.board.thermal_hotspots() {
            router.add_penalty_circle(x, y, r);
        }
        // Star-ground keepout (ТЗ 5.1).
        if let Some((x, y)) = self.board.star_ground_point() {
            router.add_penalty_circle(x, y, 8.0);
        }
        self.result = router.route_board(&self.board);
        self.routing_ms = started.elapsed().as_secs_f64() * 1000.0;

        // Diagnostics: structural validation plus component placement checks.
        let mut issues = self.board.validate();
        let placement = PlacementResult {
            board: self.board.clone(),
        };
        issues.extend(placement.drc_report(self.board.preset.min_clearance_mm));
        self.drc = issues;

        self.status = if self.result.failed_nets.is_empty() {
            format!(
                "Routed {}/{} nets in {:.0} ms · {:.1} mm of copper · export ready",
                self.result.routed_nets.len(),
                self.result.total_nets(),
                self.routing_ms,
                self.result.total_length_mm
            )
        } else {
            format!(
                "Only {}/{} nets routed in {:.0} ms · {} incomplete — export blocked",
                self.result.routed_nets.len(),
                self.result.total_nets(),
                self.routing_ms,
                self.result.failed_nets.len()
            )
        };
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

        // Thermal and star-ground keepout zones.
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
                let w = (seg.class.track_width_mm() as f32 * self.zoom).max(1.2);
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
        ui.heading(RichText::new("Nets").size(14.0).strong());
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("nets")
            .max_height(ui.available_height() * 0.55)
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
        ui.label(format!("Router time: {:.0} ms", self.routing_ms));
        ui.add_space(4.0);
        ui.add(egui::ProgressBar::new(pct / 100.0).text(format!("{pct:.0}% routed")));

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
        ui.checkbox(&mut self.show_thermal, "Thermal / star-ground");

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

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Export all (Ctrl+E)").clicked() {
                        self.export_all();
                        ui.close_menu();
                    }
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("Board", |ui| {
                    if ui.button("Route all nets (F5)").clicked() {
                        self.route();
                        ui.close_menu();
                    }
                    if ui.button("Reset view").clicked() {
                        self.zoom = 3.2;
                        self.pan = Vec2::new(40.0, 40.0);
                        ui.close_menu();
                    }
                });
                ui.menu_button("View", |ui| {
                    ui.checkbox(&mut self.show_copper, "Copper tracks");
                    ui.checkbox(&mut self.show_components, "Components");
                    ui.checkbox(&mut self.show_pads, "Pads");
                    ui.checkbox(&mut self.show_grid, "Grid");
                    ui.checkbox(&mut self.show_thermal, "Thermal overlay");
                });
                ui.menu_button("Help", |ui| {
                    ui.label(RichText::new("AmpRoute Studio").strong());
                    ui.label(format!("v{}", crate::VERSION));
                    ui.label("Autorouter for LUT audio amplifier PCBs");
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("⬇ Export").clicked() {
                        self.export_all();
                    }
                    if ui.button("⚡ Route (F5)").clicked() {
                        self.route();
                    }
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
            .default_width(220.0)
            .show(ctx, |ui| {
                self.left_panel(ui);
            });

        egui::SidePanel::right("stats")
            .resizable(true)
            .default_width(230.0)
            .show(ctx, |ui| {
                self.right_panel(ui);
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            self.draw_canvas(ui);
        });
    }
}

/// Launch the GUI. Blocks until the window is closed.
pub fn run() -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([900.0, 600.0])
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

    #[test]
    fn auto_placement_routes_most_nets() {
        let board = PlacementResult::auto_amplifier().board;
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
        let board = PlacementResult::auto_amplifier().board;
        let router = Router::new(0.5, board.width_mm, board.height_mm);
        let r = router.route_board(&board);
        assert!(r.segments.iter().all(|s| s.length_mm() > 0.0));
    }
}
