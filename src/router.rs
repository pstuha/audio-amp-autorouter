//! Router: grid-based A* with audio-aware costs, obstacle and clearance handling.
//!
//! The router is a diagnostic prototype, not a certified manufacturing tool.
//! It deliberately treats copper as an obstacle: pads of other nets and already
//! routed tracks are blocked out to track width plus LUT clearance, so a result
//! never relies on two nets sharing a grid cell. A net is reported as routed
//! only when *every* terminal reaches the connected group; partially connected
//! nets are reported as failed and contribute no copper.

use crate::board::{Board, NetClass};
use crate::occupancy::Occupancy;
use serde::Serialize;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashSet};

/// Copper layer of a routed segment. Single-layer boards use `FCu`; the
/// two-layer engine will emit `BCu` where it crosses with a via.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CopperLayer {
    FCu,
    BCu,
}

impl Default for CopperLayer {
    fn default() -> Self {
        Self::FCu
    }
}

impl CopperLayer {
    pub fn kicad_name(self) -> &'static str {
        match self {
            CopperLayer::FCu => "F.Cu",
            CopperLayer::BCu => "B.Cu",
        }
    }
}

/// A routed track segment.
#[derive(Debug, Clone, Serialize)]
pub struct RouteSegment {
    #[serde(default)]
    pub layer: CopperLayer,
    pub x1_mm: f64,
    pub y1_mm: f64,
    pub x2_mm: f64,
    pub y2_mm: f64,
    pub net: String,
    pub class: NetClass,
    /// Actual copper width: the class width, narrowed only where the class
    /// width does not fit between existing pads or tracks.
    pub width_mm: f64,
}

impl RouteSegment {
    pub fn new(x1: f64, y1: f64, x2: f64, y2: f64, net: &str, class: NetClass) -> Self {
        Self {
            layer: CopperLayer::default(),
            x1_mm: x1,
            y1_mm: y1,
            x2_mm: x2,
            y2_mm: y2,
            net: net.into(),
            class,
            width_mm: class.track_width_mm(),
        }
    }

    /// Same segment, drawn at `width_mm` instead of the class width.
    pub fn with_width(mut self, width_mm: f64) -> Self {
        self.width_mm = width_mm;
        self
    }

    /// Same segment, assigned to a copper layer.
    pub fn on_layer(mut self, layer: CopperLayer) -> Self {
        self.layer = layer;
        self
    }

    pub fn length_mm(&self) -> f64 {
        ((self.x2_mm - self.x1_mm).powi(2) + (self.y2_mm - self.y1_mm).powi(2)).sqrt()
    }
}

/// A plated transition between F.Cu and B.Cu.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Via {
    pub x_mm: f64,
    pub y_mm: f64,
    pub net: String,
    pub class: NetClass,
    /// Outer diameter and drill, in millimetres.
    pub diameter_mm: f64,
    pub drill_mm: f64,
}

/// Result of a routing run.
#[derive(Debug, Default, Clone, Serialize)]
pub struct RoutingResult {
    pub segments: Vec<RouteSegment>,
    pub routed_nets: Vec<String>,
    pub failed_nets: Vec<String>,
    pub total_length_mm: f64,
    /// Count, kept in sync with `via_objects` when the two-layer router runs.
    pub vias: usize,
    #[serde(default)]
    pub via_objects: Vec<Via>,
}

impl RoutingResult {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reason for each failed net, in the same order as `failed_nets`.
    pub fn total_nets(&self) -> usize {
        self.routed_nets.len() + self.failed_nets.len()
    }
}

/// Grid-based router with soft penalty zones (thermal, star-ground).
///
/// Hard obstacles — pads of other nets, routed copper, mounting holes and the
/// board edge — live in [`Occupancy`], which test them with exact millimetre
/// geometry instead of stamping them into whole grid cells.
pub struct Router {
    pub grid_mm: f64,
    /// Soft zones: passable, but expensive.
    pub penalty_zones: HashSet<(i32, i32)>,
    /// How strongly a cell inside a soft zone is avoided. A moderate default:
    /// strong enough to prefer free board, weak enough that a zone never
    /// wrecks the routing of a dense single-layer board.
    pub penalty_weight: f32,
    /// Clearance enforced by the routing rules; may be stricter than the
    /// board's own preset after clamping to the manufacturable band.
    pub clearance_mm: f64,
    /// Optional 8-connected search with octile heuristic. Off by default:
    /// the engine compares its result with the stable 4-connected one.
    pub diagonal_search: bool,
    pub bounds: (i32, i32, i32, i32),
    /// Fixed power/ground copper, as hard obstacles: no other net may cross it.
    /// Tuples are `(x1, y1, x2, y2, half_width, net_index)`.
    pub fixed_tracks: Vec<(f64, f64, f64, f64, f64, usize)>,
    /// Wall-clock ceiling for one `route_in_order` call. Checked between nets
    /// and between width attempts within a net, so a caller with a deadline
    /// gets its copper back instead of waiting for a search that will not
    /// finish. `None` runs to completion.
    pub deadline: Option<std::time::Instant>,
}

#[derive(Clone)]
struct Node {
    /// Cost from the start to this cell. Kept so a stale heap entry can be
    /// recognised and dropped instead of expanded a second time.
    g: f32,
    /// `g` plus the distance heuristic: the priority.
    total: f32,
    /// Flat index into the search grid.
    cell: usize,
}
impl PartialEq for Node {
    fn eq(&self, o: &Self) -> bool {
        self.total == o.total && self.cell == o.cell
    }
}
impl Eq for Node {}
impl PartialOrd for Node {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Node {
    fn cmp(&self, o: &Self) -> Ordering {
        o.total.partial_cmp(&self.total).unwrap_or(Ordering::Equal)
    }
}

fn mdist(a: (i32, i32), b: (i32, i32)) -> i32 {
    (a.0 - b.0).abs() + (a.1 - b.1).abs()
}

/// Which cells one net may occupy at one track width, and what entering each
/// costs.
///
/// Every clearance test runs once per cell here instead of once per neighbour
/// expansion in the search, and the result is reused for every terminal of the
/// net: a net's own copper never blocks itself, so the map stays valid while
/// that net is being routed. This is what makes trying many net orders and
/// reroutes affordable.
struct CostGrid {
    w: usize,
    h: usize,
    minx: i32,
    miny: i32,
    /// Cost of entering each cell; infinite where the cell is blocked.
    step: Vec<f32>,
}

impl CostGrid {
    #[inline]
    fn flat(&self, c: (i32, i32)) -> usize {
        (c.1 - self.miny) as usize * self.w + (c.0 - self.minx) as usize
    }

    #[inline]
    fn point(&self, i: usize) -> (i32, i32) {
        (
            self.minx + (i % self.w) as i32,
            self.miny + (i / self.w) as i32,
        )
    }

    #[inline]
    fn inside(&self, c: (i32, i32)) -> bool {
        c.0 >= self.minx
            && c.1 >= self.miny
            && ((c.0 - self.minx) as usize) < self.w
            && ((c.1 - self.miny) as usize) < self.h
    }
}

/// A routing attempt and the net order that produced it.
struct Attempt {
    order: Vec<usize>,
    run: OrderRun,
}

/// One net that could not be routed, and the nets whose copper walls it in.
struct StuckNet {
    net: usize,
    blockers: Vec<usize>,
}

/// The outcome of routing the nets in one particular order.
struct OrderRun {
    result: RoutingResult,
    stuck: Vec<StuckNet>,
}

/// Extra cost for running next to a net of another class (ТЗ 5.2).
fn proximity_cost(own: NetClass, other: NetClass) -> f64 {
    0.05 * own.proximity_penalty(other)
}

impl Router {
    pub fn new(grid_mm: f64, width_mm: f64, height_mm: f64) -> Self {
        let grid_mm = if grid_mm > 0.0 { grid_mm } else { 0.2 };
        Self {
            grid_mm,
            penalty_zones: HashSet::new(),
            penalty_weight: 1.5,
            clearance_mm: 0.3,
            diagonal_search: false,
            fixed_tracks: Vec::new(),
            bounds: (
                0,
                0,
                (width_mm / grid_mm).ceil() as i32,
                (height_mm / grid_mm).ceil() as i32,
            ),
            deadline: None,
        }
    }

    /// `true` once the caller's deadline has passed.
    fn out_of_time(&self) -> bool {
        self.deadline
            .is_some_and(|d| std::time::Instant::now() >= d)
    }

    /// Add a soft zone around a thermal hotspot or the star-ground point.
    /// Zones cost more but stay passable.
    pub fn add_penalty_circle(&mut self, cx: f64, cy: f64, radius_mm: f64) {
        let cx_g = (cx / self.grid_mm).round() as i32;
        let cy_g = (cy / self.grid_mm).round() as i32;
        let r_g = (radius_mm / self.grid_mm).ceil() as i32;
        for dy in -r_g..=r_g {
            for dx in -r_g..=r_g {
                if dx * dx + dy * dy <= r_g * r_g {
                    self.penalty_zones.insert((cx_g + dx, cy_g + dy));
                }
            }
        }
    }

    fn cell(&self, p: (f64, f64)) -> (i32, i32) {
        (
            (p.0 / self.grid_mm).round() as i32,
            (p.1 / self.grid_mm).round() as i32,
        )
    }

    /// Centre of a cell in millimetres.
    fn point(&self, c: (i32, i32)) -> (f64, f64) {
        (c.0 as f64 * self.grid_mm, c.1 as f64 * self.grid_mm)
    }

    /// Route every net of `board`.
    ///
    /// Net ordering decides whether a single-layer board closes: a wide power
    /// rail laid first can box in a thin signal net that would have been trivial
    /// on its own. So a fixed set of deterministic orders is tried (see
    /// [`Router::net_orders`]) and the run that routes the most nets wins, ties
    /// broken by shorter total copper.
    ///
    /// If a net still fails, the successful orders are searched again with that
    /// net moved earlier: ahead of each net whose copper walls it in when that is
    /// known, and otherwise simply first. That is what a human does with a
    /// marker — change the order of the conflicting traces, not the rules.
    pub fn route_board(&self, board: &Board) -> RoutingResult {
        let mut current = self.best_of(self.net_orders(board), board);
        if current.run.result.failed_nets.is_empty() {
            return current.run.result;
        }

        // (net moved, net it was moved in front of; `usize::MAX` = to the front).
        let mut tried: HashSet<(usize, usize)> = HashSet::new();
        let budget = (4 * board.nets.len()).clamp(8, 40);

        for _ in 0..budget {
            if current.run.result.failed_nets.is_empty() {
                break;
            }

            // Best next move: promote a stuck net ahead of one of its blockers,
            // and failing any known blocker, to the front of the order.
            let Some((stuck, anchor)) = current.run.stuck.iter().find_map(|s| {
                s.blockers
                    .iter()
                    .map(|&b| (s.net, b))
                    .chain(std::iter::once((s.net, usize::MAX)))
                    .find(|pair| !tried.contains(pair))
            }) else {
                break;
            };
            tried.insert((stuck, anchor));

            let mut order = current.order.clone();
            order.retain(|&i| i != stuck);
            let at = if anchor == usize::MAX {
                0
            } else {
                order
                    .iter()
                    .position(|&i| i == anchor)
                    .unwrap_or(order.len())
            };
            order.insert(at, stuck);

            let run = self.route_with_order(board, &order);
            // Keep any gain: more nets connected, or the same nets with less
            // copper. Equal results are dropped so the budget is not spent
            // re-finding the plan we already have.
            let better = run.result.routed_nets.len() > current.run.result.routed_nets.len()
                || (run.result.routed_nets.len() == current.run.result.routed_nets.len()
                    && run.result.failed_nets != current.run.result.failed_nets);
            if better {
                let gained = run.result.routed_nets.len() > current.run.result.routed_nets.len();
                current = Attempt { order, run };
                if gained {
                    // The blockers of the new plan are new information.
                    tried.clear();
                }
            }
        }

        current.run.result
    }

    /// Route a board with one explicit net order (indices into `board.nets`).
    ///
    /// Exposed so a caller can reproduce or override the automatic ordering;
    /// `route_board` calls this internally.
    pub fn route_in_order(&self, board: &Board, order: &[usize]) -> RoutingResult {
        self.route_with_order(board, order).result
    }

    /// Best result among several net orders, keeping the order that produced it.
    fn best_of(&self, orders: Vec<Vec<usize>>, board: &Board) -> Attempt {
        let mut best: Option<Attempt> = None;
        for order in orders {
            let run = self.route_with_order(board, &order);
            let attempt = Attempt { order, run };
            best = match best {
                None => Some(attempt),
                Some(current) => {
                    let better = attempt.run.result.routed_nets.len()
                        > current.run.result.routed_nets.len()
                        || (attempt.run.result.routed_nets.len()
                            == current.run.result.routed_nets.len()
                            && attempt.run.result.total_length_mm
                                < current.run.result.total_length_mm);
                    Some(if better { attempt } else { current })
                }
            };
            // A complete route cannot be beaten on net count, and reruns cost
            // time, so stop as soon as everything is connected.
            if best
                .as_ref()
                .is_some_and(|b| b.run.result.failed_nets.is_empty())
            {
                break;
            }
        }
        best.unwrap_or_else(|| Attempt {
            order: (0..board.nets.len()).collect(),
            run: OrderRun {
                result: RoutingResult::new(),
                stuck: Vec::new(),
            },
        })
    }


    /// Deterministic net orderings to try, most useful first.
    ///
    /// No single order is right for every board: a wide power track laid first
    /// can box in a signal net, while routing signals first can leave a ground
    /// star unreachable. Trying a fixed, reproducible set and keeping the best
    /// result is the honest alternative to tuning one order per board.
    fn net_orders(&self, board: &Board) -> Vec<Vec<usize>> {
        let n = board.nets.len();
        let all: Vec<usize> = (0..n).collect();
        let mut orders: Vec<Vec<usize>> = Vec::new();

        // 1. Class priority, then name — the original ordering.
        let mut by_priority = all.clone();
        by_priority.sort_by(|&a, &b| {
            board.nets[a]
                .class
                .priority()
                .cmp(&board.nets[b].class.priority())
                .then_with(|| board.nets[a].name.cmp(&board.nets[b].name))
        });
        orders.push(by_priority.clone());

        // 2. The reverse.
        let mut reversed = by_priority.clone();
        reversed.reverse();
        orders.push(reversed);

        // 3. Signals first, then power, ground last.
        let mut signal_first = all.clone();
        signal_first.sort_by(|&a, &b| {
            let rank = |i: usize| match board.nets[i].class {
                NetClass::Ground => 2,
                NetClass::Power => 1,
                _ => 0,
            };
            rank(a)
                .cmp(&rank(b))
                .then_with(|| {
                    board.nets[a]
                        .class
                        .priority()
                        .cmp(&board.nets[b].class.priority())
                })
                .then_with(|| board.nets[a].name.cmp(&board.nets[b].name))
        });
        orders.push(signal_first);

        // 4. Most constrained net first: a net with many terminals has the least
        //    freedom, so it must claim its copper before the open board fills up.
        let mut most_terminals_first = all.clone();
        most_terminals_first.sort_by(|&a, &b| {
            let ta = board.net_terminals(&board.nets[a]).len();
            let tb = board.net_terminals(&board.nets[b]).len();
            tb.cmp(&ta)
                .then_with(|| board.nets[a].name.cmp(&board.nets[b].name))
        });
        orders.push(most_terminals_first.clone());

        // 5. Ground and power first, signals last: the opposite of order 3, and
        //    the right choice when the supply rails have the fewest options.
        let mut rails_first = all.clone();
        rails_first.sort_by(|&a, &b| {
            let rank = |i: usize| match board.nets[i].class {
                NetClass::Ground => 0,
                NetClass::Power => 1,
                _ => 2,
            };
            rank(a)
                .cmp(&rank(b))
                .then_with(|| {
                    board.nets[a]
                        .class
                        .priority()
                        .cmp(&board.nets[b].class.priority())
                })
                .then_with(|| board.nets[a].name.cmp(&board.nets[b].name))
        });
        orders.push(rails_first);

        // 6. Terminals descending, but rails before signals among equals.
        let mut mixed = most_terminals_first;
        mixed.sort_by(|&a, &b| {
            let ta = board.net_terminals(&board.nets[a]).len();
            let tb = board.net_terminals(&board.nets[b]).len();
            let rank = |i: usize| match board.nets[i].class {
                NetClass::Ground => 0,
                NetClass::Power => 1,
                _ => 2,
            };
            tb.cmp(&ta)
                .then_with(|| rank(a).cmp(&rank(b)))
                .then_with(|| board.nets[a].name.cmp(&board.nets[b].name))
        });
        orders.push(mixed);

        orders
    }

    fn route_with_order(&self, board: &Board, order: &[usize]) -> OrderRun {
        let mut result = RoutingResult::new();
        let mut stuck: Vec<StuckNet> = Vec::new();
        let clearance = self.clearance_mm;

        // Copper must be judged with exact millimetre geometry: stamping
        // clearance into whole grid cells rounds every obstacle up on four
        // sides and leaves no channel at a 2.54 mm pad pitch.
        let mut occupancy = Occupancy::new(board, &self.fixed_tracks);

        for &index in order {
            if self.out_of_time() {
                // Report what was not attempted so the caller sees the truth
                // rather than a silently short plan.
                for &rest in &order[order.iter().position(|&i| i == index).unwrap_or(0)..] {
                    let net = &board.nets[rest];
                    if board.net_terminals(net).len() > 1
                        && !result.routed_nets.iter().any(|n| n == &net.name)
                    {
                        result.failed_nets.push(format!("{}: deadline reached", net.name));
                    }
                }
                break;
            }
            let net = &board.nets[index];
            let terminals: Vec<(f64, f64)> = board.net_terminals(net);
            if terminals.len() < 2 {
                continue;
            }

            let mut goals: Vec<(i32, i32)> = Vec::new();
            for t in &terminals {
                let c = self.cell(*t);
                if !goals.contains(&c) {
                    goals.push(c);
                }
            }
            if goals.len() < 2 {
                continue;
            }

            // A power rail is drawn 1 mm wide, but at a squeeze between two pads
            // that width simply does not fit. So the class width is tried first
            // and the net is narrowed only when it cannot pass at all, never
            // below the process minimum. A narrower track still carries the
            // current over a short gap.
            let class_width = net.class.track_width_mm();
            let min_width = board.preset.min_track_mm.clamp(0.05, class_width);
            let mut widths = vec![class_width];
            let mut w = class_width;
            while w > min_width + 1e-9 {
                w = (w * 0.7).max(min_width);
                widths.push(w);
            }
            widths.dedup_by(|a, b| (*a - *b).abs() < 1e-6);

            let mut net_segs: Vec<RouteSegment> = Vec::new();
            let mut failure: Option<String> = None;
            let mut last_half = class_width / 2.0;
            let mut last_target: Option<(i32, i32)> = None;

            for width in widths {
                // A width attempt is the expensive unit here: check the clock
                // before starting another one.
                if self.out_of_time() {
                    failure = Some("deadline reached".into());
                    break;
                }
                // Half the track's own width: the centre line plus half a track
                // must clear every obstruction, and the copper edge must clear
                // the board outline by the process edge clearance.
                let half = width / 2.0;
                last_half = half;

                // Clearance tests run once per cell here and are reused for every
                // terminal of this net. The net's own copper is never an obstacle
                // to itself, so the map stays valid while it is routed.
                let grid = self.cost_grid(&occupancy, index, board, half, clearance);

                let mut connected: HashSet<(i32, i32)> = HashSet::new();
                connected.insert(goals[0]);
                let mut pending: Vec<(i32, i32)> = goals[1..].to_vec();
                let mut attempt_segs: Vec<RouteSegment> = Vec::new();
                let mut reason: Option<String> = None;
                let mut passes = 0;

                while !pending.is_empty() {
                    if passes >= goals.len() + 4 {
                        reason = Some(format!("gave up after {passes} passes"));
                        break;
                    }
                    passes += 1;

                    // Nearest pending terminal to the connected group, ties
                    // broken by coordinates so repeated runs are identical.
                    let target = *pending
                        .iter()
                        .min_by(|a, b| {
                            let da = connected
                                .iter()
                                .map(|c| mdist(**a, *c))
                                .min()
                                .unwrap_or(i32::MAX);
                            let db = connected
                                .iter()
                                .map(|c| mdist(**b, *c))
                                .min()
                                .unwrap_or(i32::MAX);
                            da.cmp(&db)
                                .then_with(|| a.0.cmp(&b.0))
                                .then_with(|| a.1.cmp(&b.1))
                        })
                        .expect("pending is non-empty");
                    last_target = Some(target);

                    match self.astar(target, &connected, &grid) {
                        None => {
                            let at = self.point(target);
                            reason = Some(format!(
                                "no path from terminal at ({:.1}, {:.1}) mm",
                                at.0, at.1
                            ));
                            break;
                        }
                        Some(path) => {
                            for cell in &path {
                                connected.insert(*cell);
                            }
                            let path_segs =
                                self.path_to_segments(&path, &net.name, net.class, width);
                            // Geometry stays as the validator approved it.
                            // Corner rounding is visual only (GUI round caps):
                            // a geometric chamfer changed the parallel-run and
                            // clearance verdicts on the shipped boards.
                            attempt_segs.extend(path_segs);
                            pending.retain(|p| !connected.contains(p));
                        }
                    }
                }

                if reason.is_none() {
                    net_segs = attempt_segs;
                    break;
                }
                failure = reason;
            }

            if net_segs.is_empty() {
                // Find out who walled the terminal in, so the caller can rip
                // those nets up and try again.
                if let Some(target) = last_target {
                    let at = self.point(target);
                    let (_, blockers) = occupancy.blocking_nets(
                        at,
                        last_half,
                        clearance,
                        index,
                        self.grid_mm,
                        200_000,
                    );
                    stuck.push(StuckNet {
                        net: index,
                        blockers,
                    });
                }
                // Report the diagnostics but do not emit partially connected copper.
                result.failed_nets.push(format!(
                    "{}: {}",
                    net.name,
                    failure.unwrap_or_else(|| "no path".into())
                ));
            } else {
                // The new copper becomes an obstacle for later nets.
                for seg in &net_segs {
                    occupancy.add_track(
                        seg.x1_mm,
                        seg.y1_mm,
                        seg.x2_mm,
                        seg.y2_mm,
                        seg.width_mm / 2.0,
                        index,
                    );
                }
                result.routed_nets.push(net.name.clone());
                result.total_length_mm += net_segs.iter().map(|s| s.length_mm()).sum::<f64>();
                result.segments.extend(net_segs);
            }
        }

        OrderRun { result, stuck }
    }

    /// Collapse a grid path into maximal straight 0°/45°/90° segments.
    ///
    /// A 4-neighbour A* path is a staircase; joining the ends of a staircase
    /// diagonal into one 45° line is legal only if the whole new segment clears
    /// every pad, hole and routed copper shape at exact millimetre geometry.
    fn path_to_segments(
        &self,
        path: &[(i32, i32)],
        net: &str,
        class: NetClass,
        width_mm: f64,
    ) -> Vec<RouteSegment> {
        let mut segs = Vec::new();
        if path.len() < 2 {
            return segs;
        }
        let point = |c: (i32, i32)| (c.0 as f64 * self.grid_mm, c.1 as f64 * self.grid_mm);
        // Search geometry stays byte-stable: merge only exact collinear runs.
        // Line-of-sight simplification is applied after routing in
        // `routing_engine::simplify_result`, where it can be validated and
        // rolled back without changing which nets get routed.
        let mut i = 0;
        while i < path.len() - 1 {
            let dir = (path[i + 1].0 - path[i].0, path[i + 1].1 - path[i].1);
            let mut j = i;
            while j + 1 < path.len()
                && (path[j + 1].0 - path[j].0, path[j + 1].1 - path[j].1) == dir
            {
                j += 1;
            }
            let a = point(path[i]);
            let b = point(path[j]);
            segs.push(RouteSegment::new(a.0, a.1, b.0, b.1, net, class).with_width(width_mm));
            i = j;
        }
        segs
    }

    /// Chamfer each legal corner of a routed net.
    ///
    /// Toner-transfer copper should not ship with needle-sharp corners: a
    /// small 45° cut at a 90° turn prints and etches more reliably. Every cut
    /// is checked against the exact-millimetre occupancy field, so a chamfer
    /// is applied only where the new diagonal copper clears other nets.
    pub(crate) fn chamfer_corners(
        &self,
        segments: &[RouteSegment],
        half: f64,
        clearance: f64,
        net: usize,
        occupancy: &Occupancy,
    ) -> Vec<RouteSegment> {
        if segments.len() < 2 {
            return segments.to_vec();
        }
        let clone_between = |seg: &RouteSegment, from: (f64, f64), to: (f64, f64)| RouteSegment {
            layer: seg.layer,
            x1_mm: from.0,
            y1_mm: from.1,
            x2_mm: to.0,
            y2_mm: to.1,
            net: seg.net.clone(),
            class: seg.class,
            width_mm: seg.width_mm,
        };
        let mut out = Vec::new();
        let mut start = (segments[0].x1_mm, segments[0].y1_mm);
        for pair in segments.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            let p = (a.x2_mm, a.y2_mm);
            let a_start = (a.x1_mm, a.y1_mm);
            let b_end = (b.x2_mm, b.y2_mm);
            let len_a = ((a_start.0 - p.0).powi(2) + (a_start.1 - p.1).powi(2)).sqrt();
            let len_b = ((b_end.0 - p.0).powi(2) + (b_end.1 - p.1).powi(2)).sqrt();
            let cross = (p.0 - a_start.0) * (b_end.1 - p.1) - (p.1 - a_start.1) * (b_end.0 - p.0);
            let dot = (p.0 - a_start.0) * (b_end.0 - p.0) + (p.1 - a_start.1) * (b_end.1 - p.1);
            // Only a true 90° corner between two axis-aligned runs gets a
            // chamfer. A 45° turn would produce a 22.5° bridge, which is not
            // a canonical LUT angle.
            let axis_aligned = |s: &RouteSegment| {
                (s.x2_mm - s.x1_mm).abs() < 1e-6 || (s.y2_mm - s.y1_mm).abs() < 1e-6
            };
            if !axis_aligned(a)
                || !axis_aligned(b)
                || cross.abs() < 1e-9
                || dot < 0.0
                || len_a < 1e-6
                || len_b < 1e-6
            {
                out.push(clone_between(a, start, p));
                start = p;
                continue;
            }
            // Largest legal cut wins: a visible 45° chamfer where there is
            // room, a smaller one in tight channels, no cut at all if even
            // 0.25 mm would violate clearance.
            let mut chosen: Option<(f64, (f64, f64), (f64, f64))> = None;
            for wanted in [1.0_f64, 0.75, 0.5, 0.25] {
                let cut = wanted.min(len_a * 0.3).min(len_b * 0.3);
                if cut < 0.2 {
                    continue;
                }
                let p1 = (
                    p.0 + (a_start.0 - p.0) / len_a * cut,
                    p.1 + (a_start.1 - p.1) / len_a * cut,
                );
                let p2 = (
                    p.0 + (b_end.0 - p.0) / len_b * cut,
                    p.1 + (b_end.1 - p.1) / len_b * cut,
                );
                // Exact millimetre geometry along the whole diagonal, with a
                // small process margin: endpoint samples are not enough.
                if occupancy.allows_segment(p1, p2, half, clearance + 0.1, net) {
                    chosen = Some((cut, p1, p2));
                    break;
                }
            }
            let Some((_cut, p1, p2)) = chosen else {
                out.push(clone_between(a, start, p));
                start = p;
                continue;
            };
            out.push(clone_between(a, start, p1));
            out.push(clone_between(a, p1, p2));
            start = p2;
        }
        let last = segments.last().unwrap();
        out.push(clone_between(last, start, (last.x2_mm, last.y2_mm)));
        out
    }

    /// Which cells `net_index` may occupy at `half` width, and what entering
    /// each cell costs.
    ///
    /// Built once per net and width and reused for every terminal of that net:
    /// a net's own copper never blocks itself, so the map cannot go stale while
    /// that net is being routed.
    fn cost_grid(
        &self,
        occupancy: &Occupancy,
        net_index: usize,
        board: &Board,
        half: f64,
        clearance: f64,
    ) -> CostGrid {
        let (minx, miny, maxx, maxy) = self.bounds;
        let w = (maxx - minx + 1).max(1) as usize;
        let h = (maxy - miny + 1).max(1) as usize;

        // Distance at which a neighbouring net starts to cost extra: close
        // enough to couple, far enough not to matter for a well-spaced board.
        let isolation_mm = 1.2;
        // Beyond this much free room a cell is "roomy enough" and costs nothing.
        let slack_cap_mm = 1.5;
        let own_class = board.nets[net_index].class;

        let mut step = vec![f32::INFINITY; w * h];
        for y in miny..=maxy {
            for x in minx..=maxx {
                let c = (x, y);
                let i = (y - miny) as usize * w + (x - minx) as usize;
                let mut cost = 1.0_f32;
                if self.penalty_zones.contains(&c) {
                    cost *= self.penalty_weight;
                }
                let p = self.point(c);
                if !occupancy.allows(p, half, clearance, net_index) {
                    continue;
                }
                // A route squeezed through the tightest possible gap is short but
                // leaves nothing for the nets routed after it. Prefer roomy
                // channels: this is what lets a single-layer board close.
                let slack = occupancy.slack(p, half, clearance, net_index, slack_cap_mm);
                cost += (3.0 * (slack_cap_mm - slack) / slack_cap_mm) as f32;
                // Audio isolation: prefer not to run alongside a rail or an
                // output track of a different class (ТЗ 5.2).
                if let Some(other) = occupancy.foreign_within(p, isolation_mm, net_index) {
                    cost += proximity_cost(own_class, board.nets[other].class) as f32;
                }
                step[i] = cost;
            }
        }

        CostGrid {
            w,
            h,
            minx,
            miny,
            step,
        }
    }

    fn astar(
        &self,
        start: (i32, i32),
        goals: &HashSet<(i32, i32)>,
        grid: &CostGrid,
    ) -> Option<Vec<(i32, i32)>> {
        if !grid.inside(start) {
            return None;
        }

        let cells = grid.step.len();
        let sqrt2 = std::f64::consts::SQRT_2 as f32;

        // Exact Manhattan distance to the nearest goal on an empty board, by
        // BFS from all goals at once. Kept deliberately 4-connected: changing
        // the search graph changes every routing decision on dense LUT boards.
        let mut guess = vec![f32::INFINITY; cells];
        let mut queue: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
        for g in goals {
            if !grid.inside(*g) {
                continue;
            }
            let i = grid.flat(*g);
            if guess[i] == 0.0 {
                continue;
            }
            guess[i] = 0.0;
            queue.push_back(i);
        }
        while let Some(i) = queue.pop_front() {
            let d = guess[i] + 1.0;
            let c = grid.point(i);
            for nb in [(c.0 + 1, c.1), (c.0 - 1, c.1), (c.0, c.1 + 1), (c.0, c.1 - 1)] {
                if !grid.inside(nb) {
                    continue;
                }
                let ni = grid.flat(nb);
                if d < guess[ni] {
                    guess[ni] = d;
                    queue.push_back(ni);
                }
            }
        }
        let unreachable = f32::MAX / 8.0;
        for value in guess.iter_mut() {
            if !value.is_finite() {
                *value = unreachable;
            }
        }
        if self.diagonal_search {
            // Octile distance is admissible for 8-connected movement.
            let goals_xy: Vec<(i32, i32)> = goals.iter().copied().collect();
            for i in 0..cells {
                let c = grid.point(i);
                guess[i] = goals_xy
                    .iter()
                    .map(|g| {
                        let dx = (c.0 - g.0).abs() as f32;
                        let dy = (c.1 - g.1).abs() as f32;
                        dx.max(dy) + (sqrt2 - 1.0) * dx.min(dy)
                    })
                    .fold(f32::INFINITY, f32::min);
            }
        }

        let mut is_goal = vec![false; cells];
        for g in goals {
            if grid.inside(*g) {
                let i = grid.flat(*g);
                is_goal[i] = true;
            }
        }

        let mut distance = vec![f32::INFINITY; cells];
        let mut came = vec![u32::MAX; cells];
        let start_cell = grid.flat(start);
        distance[start_cell] = 0.0;
        let mut open = BinaryHeap::new();
        open.push(Node {
            g: 0.0,
            total: guess[start_cell],
            cell: start_cell,
        });

        // Expanding a cell is the innermost expensive step, so the clock is
        // consulted here too: one net on a dense board can otherwise outlast
        // the caller's whole budget.
        let mut expanded = 0usize;

        while let Some(node) = open.pop() {
            expanded += 1;
            if expanded % 2048 == 0 && self.out_of_time() {
                return None;
            }
            // Stale heap entry: a shorter route to this cell was already found.
            if node.g > distance[node.cell] {
                continue;
            }
            if is_goal[node.cell] {
                let mut path = Vec::new();
                let mut at = node.cell;
                loop {
                    path.push(grid.point(at));
                    if at == start_cell {
                        break;
                    }
                    let prev = came[at];
                    if prev == u32::MAX {
                        return None;
                    }
                    at = prev as usize;
                }
                path.reverse();
                return Some(path);
            }

            let c = grid.point(node.cell);
            let mut candidates = vec![
                (c.0 + 1, c.1, 1.0_f32),
                (c.0 - 1, c.1, 1.0),
                (c.0, c.1 + 1, 1.0),
                (c.0, c.1 - 1, 1.0),
            ];
            if self.diagonal_search {
                let passable = |p: (i32, i32)| {
                    grid.inside(p) && {
                        let pi = grid.flat(p);
                        is_goal[pi] || grid.step[pi].is_finite()
                    }
                };
                for (dx, dy) in [(1, 1), (1, -1), (-1, 1), (-1, -1)] {
                    // Do not cut the corner between two blocked neighbours.
                    if passable((c.0 + dx, c.1)) || passable((c.0, c.1 + dy)) {
                        candidates.push((c.0 + dx, c.1 + dy, sqrt2));
                    }
                }
            }
            for (nx, ny, factor) in candidates {
                let nb = (nx, ny);
                if !grid.inside(nb) {
                    continue;
                }
                let ni = grid.flat(nb);
                // A terminal of this net is always reachable, even if another
                // net's pad sits within clearance of it: refusing it would make a
                // routable net fail for no real reason.
                let step = if is_goal[ni] { 1.0 } else { grid.step[ni] } * factor;
                if !step.is_finite() {
                    continue;
                }
                let reached = node.g + step;
                if reached < distance[ni] {
                    distance[ni] = reached;
                    came[ni] = node.cell as u32;
                    open.push(Node {
                        g: reached,
                        total: reached + guess[ni],
                        cell: ni,
                    });
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{Footprint, LutPreset, Net, Pad, PadRef};

    fn two_pad_board() -> Board {
        let mut b = Board::new("demo", 60.0, 40.0, LutPreset::reliable());
        b.add_footprint(Footprint::new(
            "R1",
            "10k",
            NetClass::Signal,
            5.0,
            15.0,
            6.0,
            4.0,
        ));
        b.add_footprint(Footprint::new(
            "R2",
            "10k",
            NetClass::Signal,
            45.0,
            15.0,
            6.0,
            4.0,
        ));
        b.add_net(Net::new(
            "N1",
            NetClass::Signal,
            vec![PadRef::new("R1", 0), PadRef::new("R2", 0)],
        ));
        b
    }

    #[test]
    fn routes_two_point_net() {
        let b = two_pad_board();
        let router = Router::new(0.5, b.width_mm, b.height_mm);
        let r = router.route_board(&b);
        assert_eq!(r.routed_nets, vec!["N1".to_string()]);
        assert!(
            r.failed_nets.is_empty(),
            "unexpected failures: {:?}",
            r.failed_nets
        );
        assert!(
            (r.total_length_mm - 40.0).abs() < 1.0,
            "length {}",
            r.total_length_mm
        );
    }

    #[test]
    fn corners_are_chamfered_for_lut() {
        let board = Board::new("chamfer", 40.0, 30.0, LutPreset::reliable());
        let router = Router::new(0.5, board.width_mm, board.height_mm);
        let occupancy = Occupancy::new(&board, &[]);
        let raw = vec![
            RouteSegment::new(5.0, 10.0, 20.0, 10.0, "N", NetClass::Signal),
            RouteSegment::new(20.0, 10.0, 20.0, 25.0, "N", NetClass::Signal),
        ];
        let cut = router.chamfer_corners(&raw, 0.2, 0.3, 0, &occupancy);
        assert_eq!(cut.len(), 3, "expected two trimmed legs plus a chamfer: {cut:?}");
        let mid = &cut[1];
        let dx = (mid.x2_mm - mid.x1_mm).abs();
        let dy = (mid.y2_mm - mid.y1_mm).abs();
        assert!((dx - dy).abs() < 1e-9, "chamfer must be 45°: {mid:?}");
        assert!(dx >= 0.4, "chamfer is too small to print: {mid:?}");
    }

    #[test]
    fn connectors_become_straight_segments() {
        let b = two_pad_board();
        let r = Router::new(0.5, b.width_mm, b.height_mm).route_board(&b);
        // A straight connection must collapse to a single segment, not hundreds.
        assert!(
            r.segments.len() <= 3,
            "expected few segments, got {}",
            r.segments.len()
        );
    }

    #[test]
    fn different_nets_never_share_a_cell() {
        let mut b = Board::new("cross", 60.0, 60.0, LutPreset::reliable());
        b.add_footprint(Footprint::new(
            "R1",
            "a",
            NetClass::Signal,
            4.0,
            4.0,
            6.0,
            4.0,
        ));
        b.add_footprint(Footprint::new(
            "R2",
            "b",
            NetClass::Signal,
            48.0,
            4.0,
            6.0,
            4.0,
        ));
        b.add_footprint(Footprint::new(
            "R3",
            "c",
            NetClass::Signal,
            24.0,
            4.0,
            6.0,
            4.0,
        ));
        b.add_footprint(Footprint::new(
            "R4",
            "d",
            NetClass::Signal,
            24.0,
            46.0,
            6.0,
            4.0,
        ));
        b.add_net(Net::new(
            "H",
            NetClass::Signal,
            vec![PadRef::new("R1", 0), PadRef::new("R2", 0)],
        ));
        b.add_net(Net::new(
            "V",
            NetClass::Signal,
            vec![PadRef::new("R3", 0), PadRef::new("R4", 0)],
        ));
        let r = Router::new(0.5, b.width_mm, b.height_mm).route_board(&b);

        // Build the cell set of each routed net and compare.
        let cells = |name: &str| -> HashSet<(i64, i64)> {
            r.segments
                .iter()
                .filter(|s| s.net == name)
                .flat_map(|s| {
                    let x0 = (s.x1_mm / 0.5) as i64;
                    let y0 = (s.y1_mm / 0.5) as i64;
                    let x1 = (s.x2_mm / 0.5) as i64;
                    let y1 = (s.y2_mm / 0.5) as i64;
                    let n = (x1 - x0).abs().max((y1 - y0).abs()) + 1;
                    (0..=n).map(move |i| {
                        let t = if n == 0 { 0.0 } else { i as f64 / n as f64 };
                        (
                            x0 + ((x1 - x0) as f64 * t).round() as i64,
                            y0 + ((y1 - y0) as f64 * t).round() as i64,
                        )
                    })
                })
                .collect()
        };
        let h = cells("H");
        let v = cells("V");
        assert!(!h.is_empty() && !v.is_empty(), "both nets should route");
        assert!(h.intersection(&v).count() == 0, "nets H and V overlap");
    }

    #[test]
    fn partially_connected_net_is_reported_failed() {
        let mut b = Board::new("blocked", 40.0, 40.0, LutPreset::reliable());
        b.add_footprint(Footprint::new(
            "A1",
            "a",
            NetClass::Signal,
            1.0,
            1.0,
            6.0,
            4.0,
        ));
        b.add_footprint(Footprint::new(
            "A2",
            "b",
            NetClass::Signal,
            1.0,
            20.0,
            6.0,
            4.0,
        ));
        b.add_footprint(Footprint::new(
            "A3",
            "c",
            NetClass::Signal,
            34.0,
            20.0,
            6.0,
            4.0,
        ));
        // A wall of foreign copper covering the right half of the board.
        let mut wall = Footprint::new("WALL", "wall", NetClass::Signal, 15.0, 2.0, 4.0, 36.0);
        wall.pads = vec![Pad::new(2.0, 18.0, 30.0, 1.0)];
        b.add_footprint(wall);
        b.add_net(Net::new(
            "A",
            NetClass::Signal,
            vec![
                PadRef::new("A1", 0),
                PadRef::new("A2", 0),
                PadRef::new("A3", 0),
            ],
        ));
        b.add_net(Net::new(
            "W",
            NetClass::Signal,
            vec![PadRef::new("WALL", 0)],
        ));

        let r = Router::new(0.5, b.width_mm, b.height_mm).route_board(&b);
        assert!(
            r.failed_nets.iter().any(|n| n.starts_with("A:")),
            "net A should fail: {:?}",
            r.failed_nets
        );
        assert!(
            !r.routed_nets.iter().any(|n| n == "A"),
            "partially connected net must not count as routed"
        );
        assert!(
            !r.segments.iter().any(|s| s.net == "A"),
            "failed net must not emit copper"
        );
    }

    #[test]
    fn routing_is_deterministic() {
        let b = two_pad_board();
        let a = Router::new(0.5, b.width_mm, b.height_mm).route_board(&b);
        let c = Router::new(0.5, b.width_mm, b.height_mm).route_board(&b);
        assert_eq!(a.segments.len(), c.segments.len());
        for (x, y) in a.segments.iter().zip(c.segments.iter()) {
            assert_eq!(
                (x.x1_mm, x.y1_mm, x.x2_mm, x.y2_mm),
                (y.x1_mm, y.y1_mm, y.x2_mm, y.y2_mm)
            );
        }
    }

    #[test]
    fn penalty_zone_is_passable_but_avoided() {
        let b = two_pad_board();
        let straight = Router::new(0.5, b.width_mm, b.height_mm).route_board(&b);
        // A strong zone so the avoidance is unambiguous at this small scale;
        // the default weight is gentler so dense boards still close.
        let mut router = Router::new(0.5, b.width_mm, b.height_mm);
        router.penalty_weight = 8.0;
        // Cover the direct route with a soft zone.
        router.add_penalty_circle(25.0, 17.0, 6.0);
        let detoured = router.route_board(&b);
        assert_eq!(
            detoured.routed_nets.len(),
            1,
            "zone must not make the net unroutable"
        );
        assert!(
            detoured.total_length_mm > straight.total_length_mm,
            "soft zone should lengthen the path: {} vs {}",
            detoured.total_length_mm,
            straight.total_length_mm
        );
    }

    #[test]
    fn net_class_weights_affect_cost() {
        assert!(NetClass::Ground.cost_weight() < NetClass::Signal.cost_weight());
        assert!(NetClass::Input.cost_weight() < NetClass::Control.cost_weight());
    }

    #[test]
    fn segment_length_is_euclidean() {
        let s = RouteSegment::new(0.0, 0.0, 3.0, 4.0, "N1", NetClass::Signal);
        assert!((s.length_mm() - 5.0).abs() < 1e-9);
    }
}
