//! Router: grid-based A* with audio-aware costs, obstacle and clearance handling.
//!
//! The router is a diagnostic prototype, not a certified manufacturing tool.
//! It deliberately treats copper as an obstacle: pads of other nets and already
//! routed tracks are blocked out to track width plus LUT clearance, so a result
//! never relies on two nets sharing a grid cell. A net is reported as routed
//! only when *every* terminal reaches the connected group; partially connected
//! nets are reported as failed and contribute no copper.

use crate::board::{Board, NetClass};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

/// A routed track segment.
#[derive(Debug, Clone)]
pub struct RouteSegment {
    pub x1_mm: f64,
    pub y1_mm: f64,
    pub x2_mm: f64,
    pub y2_mm: f64,
    pub net: String,
    pub class: NetClass,
}

impl RouteSegment {
    pub fn new(x1: f64, y1: f64, x2: f64, y2: f64, net: &str, class: NetClass) -> Self {
        Self {
            x1_mm: x1,
            y1_mm: y1,
            x2_mm: x2,
            y2_mm: y2,
            net: net.into(),
            class,
        }
    }

    pub fn length_mm(&self) -> f64 {
        ((self.x2_mm - self.x1_mm).powi(2) + (self.y2_mm - self.y1_mm).powi(2)).sqrt()
    }
}

/// Result of a routing run.
#[derive(Debug, Default)]
pub struct RoutingResult {
    pub segments: Vec<RouteSegment>,
    pub routed_nets: Vec<String>,
    pub failed_nets: Vec<String>,
    pub total_length_mm: f64,
    pub vias: usize,
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
pub struct Router {
    pub grid_mm: f64,
    /// Soft zones: passable, but expensive.
    pub penalty_zones: HashSet<(i32, i32)>,
    /// Hard zones: never passable.
    pub blocked: HashSet<(i32, i32)>,
    pub bounds: (i32, i32, i32, i32),
}

#[derive(Clone)]
struct Node {
    total: f64,
    point: (i32, i32),
}
impl PartialEq for Node {
    fn eq(&self, o: &Self) -> bool {
        self.total == o.total && self.point == o.point
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
            blocked: HashSet::new(),
            bounds: (
                0,
                0,
                (width_mm / grid_mm).ceil() as i32,
                (height_mm / grid_mm).ceil() as i32,
            ),
        }
    }

    /// Add a circular keep-out / penalty zone around a thermal hotspot or the
    /// star-ground point. Zones are soft: they cost more but stay passable.
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

    /// Stamp a disc of cells into `owner`, without overwriting existing claims.
    fn stamp(
        &self,
        owner: &mut HashMap<(i32, i32), usize>,
        center: (i32, i32),
        radius_cells: i32,
        net: usize,
    ) {
        for dy in -radius_cells..=radius_cells {
            for dx in -radius_cells..=radius_cells {
                if dx * dx + dy * dy <= radius_cells * radius_cells {
                    owner.entry((center.0 + dx, center.1 + dy)).or_insert(net);
                }
            }
        }
    }

    /// Route every net of `board`.
    ///
    /// Several deterministic net orders are attempted (priority order, reverse,
    /// and by terminal count) and the run that routes the most nets wins, ties
    /// broken by shorter total copper. Only fully connected nets emit copper.
    pub fn route_board(&self, board: &Board) -> RoutingResult {
        let mut orders: Vec<Vec<usize>> = Vec::new();

        let by_priority = {
            let mut o: Vec<usize> = (0..board.nets.len()).collect();
            o.sort_by(|&a, &b| {
                board.nets[a]
                    .class
                    .priority()
                    .cmp(&board.nets[b].class.priority())
                    .then_with(|| board.nets[a].name.cmp(&board.nets[b].name))
            });
            o
        };
        orders.push(by_priority.clone());

        let mut reversed = by_priority.clone();
        reversed.reverse();
        orders.push(reversed);

        // Ground and power last: signal nets claim the short paths first.
        let mut signal_first: Vec<usize> = (0..board.nets.len()).collect();
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

        let mut best: Option<RoutingResult> = None;
        for order in orders {
            let candidate = self.route_with_order(board, &order);
            best = match best {
                None => Some(candidate),
                Some(current) => {
                    let better = candidate.routed_nets.len() > current.routed_nets.len()
                        || (candidate.routed_nets.len() == current.routed_nets.len()
                            && candidate.total_length_mm < current.total_length_mm);
                    Some(if better { candidate } else { current })
                }
            };
        }
        best.unwrap_or_default()
    }

    fn route_with_order(&self, board: &Board, order: &[usize]) -> RoutingResult {
        let mut result = RoutingResult::new();
        let (minx, miny, maxx, maxy) = self.bounds;
        let clearance = board.preset.min_clearance_mm;

        // Cells occupied by copper, keyed to the owning net index.
        let mut owner: HashMap<(i32, i32), usize> = HashMap::new();
        let unassigned = usize::MAX;

        // Pads are obstacles for every other net.
        for (pos, size, net_name) in board.all_pads() {
            let net = net_name
                .as_deref()
                .and_then(|n| board.nets.iter().position(|b| b.name == n))
                .unwrap_or(unassigned);
            let radius = (((size / 2.0) + clearance) / self.grid_mm).ceil() as i32;
            self.stamp(&mut owner, self.cell(pos), radius.max(1), net);
        }

        for &index in order {
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

            let mut connected: HashSet<(i32, i32)> = HashSet::new();
            connected.insert(goals[0]);
            let mut pending: Vec<(i32, i32)> = goals[1..].to_vec();
            let mut net_segs: Vec<RouteSegment> = Vec::new();
            let mut failure = None;
            let mut passes = 0;

            while !pending.is_empty() {
                if passes >= goals.len() + 4 {
                    failure = Some(format!("gave up after {passes} passes"));
                    break;
                }
                passes += 1;

                // Nearest pending terminal to the connected group, ties broken by
                // coordinates so repeated runs produce identical output.
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

                match self.astar(
                    target,
                    &connected,
                    &owner,
                    index,
                    board,
                    (minx, miny, maxx, maxy),
                ) {
                    None => {
                        failure = Some(format!(
                            "no path from terminal at ({:.1}, {:.1}) mm",
                            target.0 as f64 * self.grid_mm,
                            target.1 as f64 * self.grid_mm
                        ));
                        break;
                    }
                    Some(path) => {
                        for cell in &path {
                            connected.insert(*cell);
                        }
                        net_segs.extend(self.path_to_segments(&path, &net.name, net.class));

                        // Reserve the copper plus clearance for later nets.
                        let radius = (((net.class.track_width_mm() / 2.0) + clearance)
                            / self.grid_mm)
                            .ceil() as i32;
                        for cell in &path {
                            self.stamp(&mut owner, *cell, radius.max(1), index);
                        }

                        pending.retain(|p| !connected.contains(p));
                    }
                }
            }

            if let Some(reason) = failure {
                // Report the diagnostics but do not emit partially connected copper.
                result.failed_nets.push(format!("{}: {}", net.name, reason));
            } else {
                result.routed_nets.push(net.name.clone());
                result.total_length_mm += net_segs.iter().map(|s| s.length_mm()).sum::<f64>();
                result.segments.extend(net_segs);
            }
        }

        result
    }

    /// Collapse a grid path into maximal straight segments.
    fn path_to_segments(
        &self,
        path: &[(i32, i32)],
        net: &str,
        class: NetClass,
    ) -> Vec<RouteSegment> {
        let mut segs = Vec::new();
        if path.len() < 2 {
            return segs;
        }
        let mut i = 0;
        while i < path.len() - 1 {
            let (x1, y1) = path[i];
            let dir = (path[i + 1].0 - x1, path[i + 1].1 - y1);
            let mut j = i;
            while j + 1 < path.len()
                && (path[j + 1].0 - path[j].0, path[j + 1].1 - path[j].1) == dir
            {
                j += 1;
            }
            let (x2, y2) = path[j];
            segs.push(RouteSegment::new(
                x1 as f64 * self.grid_mm,
                y1 as f64 * self.grid_mm,
                x2 as f64 * self.grid_mm,
                y2 as f64 * self.grid_mm,
                net,
                class,
            ));
            i = j;
        }
        segs
    }

    fn astar(
        &self,
        start: (i32, i32),
        goals: &HashSet<(i32, i32)>,
        owner: &HashMap<(i32, i32), usize>,
        net_index: usize,
        board: &Board,
        bounds: (i32, i32, i32, i32),
    ) -> Option<Vec<(i32, i32)>> {
        let (minx, miny, maxx, maxy) = bounds;
        let own_class = board.nets[net_index].class;
        let heuristic =
            |p: (i32, i32)| goals.iter().map(|g| mdist(p, *g)).min().unwrap_or(0) as f64;

        let mut open = BinaryHeap::new();
        let mut came: HashMap<(i32, i32), (i32, i32)> = HashMap::new();
        let mut cost: HashMap<(i32, i32), f64> = HashMap::new();
        cost.insert(start, 0.0);
        open.push(Node {
            total: heuristic(start),
            point: start,
        });

        let limit = 2_000_000usize;
        for _ in 0..limit {
            let node = open.pop()?;
            if goals.contains(&node.point) {
                let mut path = vec![node.point];
                while let Some(&prev) = came.get(&path[path.len() - 1]) {
                    path.push(prev);
                    if prev == start {
                        break;
                    }
                }
                path.reverse();
                return Some(path);
            }

            let (x, y) = node.point;
            let here = cost[&node.point];
            for next in [(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)] {
                if next.0 < minx || next.1 < miny || next.0 > maxx || next.1 > maxy {
                    continue;
                }
                if self.blocked.contains(&next) {
                    continue;
                }
                // Another net's copper (pads or tracks) is a hard obstacle.
                match owner.get(&next) {
                    Some(&o) if o != net_index && o != usize::MAX => continue,
                    _ => {}
                }
                // An unassigned pad blocks everything except itself being a goal.
                if owner.get(&next) == Some(&usize::MAX) && !goals.contains(&next) {
                    continue;
                }

                let mut step = 1.0;
                if self.penalty_zones.contains(&next) {
                    step *= 5.0;
                }
                // Audio isolation: discourage running alongside incompatible nets.
                for nb in [
                    (next.0 + 1, next.1),
                    (next.0 - 1, next.1),
                    (next.0, next.1 + 1),
                    (next.0, next.1 - 1),
                ] {
                    if let Some(&o) = owner.get(&nb) {
                        if o != net_index && o != usize::MAX {
                            step += proximity_cost(own_class, board.nets[o].class);
                        }
                    }
                }

                let nd = here + step;
                if let Some(&c) = cost.get(&next) {
                    if nd >= c {
                        continue;
                    }
                }
                cost.insert(next, nd);
                came.insert(next, node.point);
                open.push(Node {
                    total: nd + heuristic(next),
                    point: next,
                });
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
        let mut router = Router::new(0.5, b.width_mm, b.height_mm);
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
