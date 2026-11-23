//! Two-layer completion router.
//!
//! The single-layer A* engine routes every board we ship today. When a board is
//! marked `two_layer`, this module takes the nets it could not finish and
//! searches again on a layered grid: each cell exists on F.Cu and B.Cu, pads
//! are reachable from both, and a move to the other layer inserts a plated via
//! at that cell. Copper already routed by the engine is a hard obstacle on its
//! own layer; vias block both layers. The result is appended to the engine's
//! copper, so the single-layer path stays untouched and byte-stable.

use crate::board::{Board, NetClass};
use crate::occupancy::{Occupancy, NO_NET};
use crate::router::{CopperLayer, RouteSegment, RoutingResult, Via};
use crate::rules::LutRules;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

const VIA_COST_MM: f32 = 6.0;
const MAX_VIAS_PER_NET: usize = 8;

/// Copper produced by the two-layer completion pass.
#[derive(Debug, Default, Clone)]
pub struct TwoLayerResult {
    pub segments: Vec<RouteSegment>,
    pub vias: Vec<Via>,
    pub routed_nets: Vec<String>,
    pub failed_nets: Vec<String>,
    pub log: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Layer(CopperLayer);

impl Layer {
    fn index(self) -> usize {
        match self.0 {
            CopperLayer::FCu => 0,
            CopperLayer::BCu => 1,
        }
    }

    fn from_index(i: usize) -> Self {
        match i {
            0 => Layer(CopperLayer::FCu),
            _ => Layer(CopperLayer::BCu),
        }
    }

    fn other(self) -> Self {
        Layer(match self.0 {
            CopperLayer::FCu => CopperLayer::BCu,
            CopperLayer::BCu => CopperLayer::FCu,
        })
    }
}

#[derive(Clone, Copy)]
struct State {
    f: f32,
    g: f32,
    /// `(layer * cells + cell) * (vias + 1) + vias_used`
    key: usize,
}

impl PartialEq for State {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}
impl Eq for State {}
impl PartialOrd for State {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for State {
    fn cmp(&self, other: &Self) -> Ordering {
        other.f.partial_cmp(&self.f).unwrap_or(Ordering::Equal)
    }
}

fn net_index(board: &Board, net: &str) -> Option<usize> {
    board.nets.iter().position(|n| n.name == net)
}

fn tracks_for(
    board: &Board,
    segments: impl IntoIterator<Item = RouteSegment>,
    layer: CopperLayer,
    vias: &[Via],
) -> Vec<(f64, f64, f64, f64, f64, usize)> {
    let mut tracks = Vec::new();
    for seg in segments {
        if seg.layer != layer {
            continue;
        }
        let net = net_index(board, &seg.net).unwrap_or(NO_NET);
        tracks.push((
            seg.x1_mm,
            seg.y1_mm,
            seg.x2_mm,
            seg.y2_mm,
            seg.width_mm / 2.0,
            net,
        ));
    }
    for via in vias {
        if let Some(net) = net_index(board, &via.net) {
            tracks.push((
                via.x_mm,
                via.y_mm,
                via.x_mm,
                via.y_mm,
                via.diameter_mm / 2.0,
                net,
            ));
        }
    }
    tracks
}

/// Route the nets that the single-layer engine left unfinished, using both
/// layers. `existing` is the engine's copper; `fixed` is module 5's copper.
pub fn route_unfinished(
    board: &Board,
    existing: &RoutingResult,
    fixed_segments: &[RouteSegment],
    rules: &LutRules,
    grid_mm: f64,
) -> TwoLayerResult {
    let mut out = TwoLayerResult::default();
    if !board.two_layer || !rules.allow_vias {
        return out;
    }

    let grid = if grid_mm > 0.0 { grid_mm } else { 0.5 };
    let nx = (board.width_mm / grid).floor() as usize + 1;
    let ny = (board.height_mm / grid).floor() as usize + 1;
    let cells = nx * ny;
    let classes: Vec<(String, NetClass)> = board
        .nets
        .iter()
        .map(|n| (n.name.clone(), n.class))
        .collect();

    // Seed obstacles once; each net copies the layer occupancies and adds its
    // own copper so later nets cannot route through it.
    let seed: Vec<Occupancy> = [CopperLayer::FCu, CopperLayer::BCu]
        .into_iter()
        .map(|layer| {
            let mut segments: Vec<RouteSegment> = existing.segments.clone();
            segments.extend(fixed_segments.iter().cloned());
            Occupancy::new(board, &tracks_for(board, segments, layer, &[]))
        })
        .collect();
    let via_diameter = (rules.min_track_mm * 4.0).max(0.8);
    let via_drill = (via_diameter / 2.0).max(rules.min_track_mm);

    let failed: Vec<String> = existing
        .failed_nets
        .iter()
        .map(|f| f.split(':').next().unwrap_or(f).trim().to_string())
        .filter(|name| net_index(board, name).is_some())
        .collect();

    for net_name in failed {
        let Some(idx) = net_index(board, &net_name) else { continue };
        let class = classes[idx].1;
        let width = class.track_width_mm().max(rules.min_track_mm);
        let half = width / 2.0;
        let clearance = rules.min_clearance_mm;
        let mut occupancy = seed.clone();
        for via in &out.vias {
            // A via blocks both layers; add_track accepts a zero-length track.
            for occ in &mut occupancy {
                occ.add_track(via.x_mm, via.y_mm, via.x_mm, via.y_mm, via.diameter_mm / 2.0, idx);
            }
        }

        let terminals = board.net_terminals(&board.nets[idx]);
        let points: Vec<(i32, i32)> = terminals
            .iter()
            .map(|p| ((p.0 / grid).round() as i32, (p.1 / grid).round() as i32))
            .collect();
        if points.len() < 2 {
            out.failed_nets.push(net_name);
            continue;
        }

        let mut allowed_f = vec![false; cells];
        let mut allowed_b = vec![false; cells];
        for y in 0..ny {
            for x in 0..nx {
                let c = y * nx + x;
                let p = (x as f64 * grid, y as f64 * grid);
                allowed_f[c] = occupancy[0].allows(p, half, clearance, idx);
                allowed_b[c] = occupancy[1].allows(p, half, clearance, idx);
            }
        }
        let allowed = |layer: Layer, c: usize| match layer.0 {
            CopperLayer::FCu => allowed_f[c],
            CopperLayer::BCu => allowed_b[c],
        };

        let start_cell = points[0].1 as usize * nx + points[0].0 as usize;
        // Through-hole terminals are reachable from either layer.
        let goals: Vec<(usize, Layer)> = points[1..]
            .iter()
            .flat_map(|p| {
                let c = p.1 as usize * nx + p.0 as usize;
                [Layer(CopperLayer::FCu), Layer(CopperLayer::BCu)]
                    .into_iter()
                    .filter(move |l| allowed(*l, c))
                    .map(move |l| (c, l))
            })
            .collect();
        let goal_cells: Vec<usize> = points[1..]
            .iter()
            .map(|p| p.1 as usize * nx + p.0 as usize)
            .collect();

        let via_budget = rules.max_vias.min(MAX_VIAS_PER_NET);
        let stride = cells * 2;
        let key_of = |layer: Layer, cell: usize, vias: usize| {
            (layer.index() * cells + cell) * (via_budget + 1) + vias
        };
        let mut best = vec![f32::INFINITY; stride * (via_budget + 1)];
        let mut parent = vec![usize::MAX; stride * (via_budget + 1)];
        let mut heap = BinaryHeap::new();

        let heuristic = |cell: usize| {
            let cx = (cell % nx) as f32;
            let cy = (cell / nx) as f32;
            goal_cells
                .iter()
                .map(|g| {
                    ((cx - (g % nx) as f32).abs() + (cy - (g / nx) as f32).abs()) * grid as f32
                })
                .fold(f32::INFINITY, f32::min)
        };

        for layer in [Layer(CopperLayer::FCu), Layer(CopperLayer::BCu)] {
            if !allowed(layer, start_cell) {
                continue;
            }
            let key = key_of(layer, start_cell, 0);
            best[key] = 0.0;
            heap.push(State { f: heuristic(start_cell), g: 0.0, key });
        }

        let mut found = None;
        while let Some(current) = heap.pop() {
            if current.g > best[current.key] + 1e-6 {
                continue;
            }
            let per_layer = current.key / (via_budget + 1);
            let used = current.key % (via_budget + 1);
            let layer = Layer::from_index(per_layer / cells);
            let cell = per_layer % cells;
            if goals.iter().any(|(g, l)| *g == cell && *l == layer) {
                found = Some(current.key);
                break;
            }
            let cx = (cell % nx) as i32;
            let cy = (cell / nx) as i32;
            let mut neighbours = Vec::with_capacity(5);
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (x, y) = (cx + dx, cy + dy);
                if x < 0 || y < 0 || x as usize >= nx || y as usize >= ny {
                    continue;
                }
                let n = y as usize * nx + x as usize;
                if allowed(layer, n) {
                    neighbours.push((n, layer, used, grid as f32));
                }
            }
            if used < via_budget {
                let other = layer.other();
                if allowed(other, cell) {
                    neighbours.push((cell, other, used + 1, VIA_COST_MM));
                }
            }
            for (n, l, vias_used, step_cost) in neighbours {
                let key = key_of(l, n, vias_used);
                let g = current.g + step_cost;
                if g + 1e-6 >= best[key] {
                    continue;
                }
                best[key] = g;
                parent[key] = current.key;
                heap.push(State { f: g + heuristic(n), g, key });
            }
        }

        let Some(end) = found else {
            out.log.push(format!("{net_name}: no two-layer path"));
            out.failed_nets.push(net_name);
            continue;
        };

        // Reconstruct, then split into per-layer runs and emit vias at changes.
        let mut keys = Vec::new();
        let mut cursor = end;
        loop {
            keys.push(cursor);
            let next = parent[cursor];
            if next == usize::MAX {
                break;
            }
            cursor = next;
        }
        keys.reverse();
        let mut run: Vec<usize> = Vec::new();
        let mut run_layer = Layer::from_index((keys[0] / (via_budget + 1)) / cells);
        let mut net_segments = Vec::new();
        let mut net_vias = Vec::new();
        let mut flush = |run: &mut Vec<usize>, layer: Layer| {
            if run.len() < 2 {
                run.clear();
                return;
            }
            for pair in run.windows(2) {
                let a = pair[0];
                let b = pair[1];
                net_segments.push(
                    RouteSegment::new(
                        (a % nx) as f64 * grid,
                        (a / nx) as f64 * grid,
                        (b % nx) as f64 * grid,
                        (b / nx) as f64 * grid,
                        &net_name,
                        class,
                    )
                    .with_width(width)
                    .on_layer(layer.0),
                );
            }
            run.clear();
        };
        for &key in &keys {
            let per_layer = key / (via_budget + 1);
            let layer = Layer::from_index(per_layer / cells);
            let cell = per_layer % cells;
            if layer != run_layer {
                flush(&mut run, run_layer);
                net_vias.push(Via {
                    x_mm: (cell % nx) as f64 * grid,
                    y_mm: (cell / nx) as f64 * grid,
                    net: net_name.clone(),
                    class,
                    diameter_mm: via_diameter,
                    drill_mm: via_drill,
                });
                run_layer = layer;
            }
            run.push(cell);
        }
        flush(&mut run, run_layer);

        // Final legality check: every emitted cell must still clear other nets.
        let mut legal = true;
        for seg in &net_segments {
            let layer = if seg.layer == CopperLayer::FCu { 0 } else { 1 };
            let mid = ((seg.x1_mm + seg.x2_mm) / 2.0, (seg.y1_mm + seg.y2_mm) / 2.0);
            if !occupancy[layer].allows(mid, half, clearance, idx) {
                legal = false;
                break;
            }
        }
        if !legal || net_vias.len() > via_budget {
            out.log.push(format!("{net_name}: two-layer path failed final check"));
            out.failed_nets.push(net_name);
            continue;
        }

        for seg in &net_segments {
            let layer = if seg.layer == CopperLayer::FCu { 0 } else { 1 };
            occupancy[layer].add_track(
                seg.x1_mm,
                seg.y1_mm,
                seg.x2_mm,
                seg.y2_mm,
                half,
                idx,
            );
        }
        for via in &net_vias {
            for occ in &mut occupancy {
                occ.add_track(
                    via.x_mm,
                    via.y_mm,
                    via.x_mm,
                    via.y_mm,
                    via.diameter_mm / 2.0,
                    idx,
                );
            }
        }
        out.segments.extend(net_segments);
        out.vias.extend(net_vias);
        out.log.push(format!("{net_name}: routed on two layers"));
        out.routed_nets.push(net_name);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{Footprint, LutPreset, Net, Pad, PadRef};

    /// Two signal pads separated by a long wall of another net's copper on
    /// F.Cu: impossible on one layer, reachable with one via on B.Cu.
    fn blocked_board() -> Board {
        let mut board = Board::new("2L-test", 40.0, 30.0, LutPreset::reliable()).with_two_layer();
        for (refdes, x) in [("A", 4.0), ("B", 36.0)] {
            board.add_footprint(
                Footprint::new(refdes, "test", NetClass::Signal, x, 14.0, 4.0, 4.0)
                    .with_pads(vec![Pad::new(2.0, 2.0, 1.8, 0.9)]),
            );
        }
        board.add_net(Net::new(
            "SIG",
            NetClass::Signal,
            vec![PadRef::new("A", 0), PadRef::new("B", 0)],
        ));
        board
    }

    #[test]
    fn two_layer_router_crosses_a_front_copper_wall_with_vias() {
        let board = blocked_board();
        // Extend past both board edges so there is no single-layer detour.
        let front_wall = RouteSegment::new(20.0, -5.0, 20.0, 35.0, "WF", NetClass::Power)
            .with_width(2.0);
        // B.Cu walls near each terminal force the route to start/end on F.Cu
        // and use vias to cross the front wall in the middle.
        let back_wall_a = RouteSegment::new(10.0, -5.0, 10.0, 35.0, "WA", NetClass::Power)
            .with_width(2.0)
            .on_layer(CopperLayer::BCu);
        let back_wall_b = RouteSegment::new(30.0, -5.0, 30.0, 35.0, "WB", NetClass::Power)
            .with_width(2.0)
            .on_layer(CopperLayer::BCu);
        let existing = RoutingResult {
            failed_nets: vec!["SIG: no path".into()],
            ..Default::default()
        };
        let result = route_unfinished(
            &board,
            &existing,
            &[front_wall, back_wall_a, back_wall_b],
            &LutRules::two_layer(),
            1.0,
        );
        assert_eq!(result.routed_nets, ["SIG"], "log: {:?}", result.log);
        assert!(
            result.vias.len() >= 2,
            "a crossing needs two vias: {:?}; segments: {:?}",
            result.vias,
            result.segments.iter().map(|s| (s.layer, s.x1_mm, s.y1_mm, s.x2_mm, s.y2_mm)).collect::<Vec<_>>()
        );
        assert!(result.segments.iter().any(|s| s.layer == CopperLayer::BCu));
    }

    #[test]
    fn single_layer_boards_are_left_alone() {
        let mut board = blocked_board();
        board.two_layer = false;
        let existing = RoutingResult {
            failed_nets: vec!["SIG: no path".into()],
            ..Default::default()
        };
        let result = route_unfinished(&board, &existing, &[], &LutRules::reliable(), 1.0);
        assert!(result.segments.is_empty());
        assert!(result.vias.is_empty());
        assert!(result.routed_nets.is_empty());
    }
}
