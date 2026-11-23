//! Module 5: pre-route power and ground before the router runs.
//!
//! Power and ground nets (`NetClass::Power`, `NetClass::Ground`) are low
//! impedance and do not tolerate detours through the general signal routing
//! pass, so they are connected up front. Module 5:
//!
//! * radiates one wide ground trace from every ground terminal to the
//!   ground-star point ([`Board::star_ground_point`]);
//! * radiates one wide power trace from every power terminal to the same
//!   star point;
//! * returns a board with the power/ground nets removed, so the router
//!   receives only signal nets.
//!
//! The wide traces are frozen as data ([`PreRoute::traces`]) — they become
//! fixed copper later, after routing.

use crate::board::{Board, NetClass};
use crate::occupancy::Occupancy;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// One fixed wide power/ground trace.
#[derive(Debug, Clone, PartialEq)]
pub struct PreRouteTrace {
    pub net: String,
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
    /// Trace width taken from the net class: power 0.8 mm, ground 0.6 mm.
    pub width_mm: f64,
    pub layer: String,
}

/// What module 5 hands to the router and to later stages.
#[derive(Debug)]
pub struct PreRoute {
    /// Board with power/ground nets removed — what the router receives.
    pub board: Board,
    /// Ground-star point the wide ground traces radiate to.
    pub star_point: Option<(f64, f64)>,
    /// Power-star point. Kept separate from ground: tying both nets to one
    /// point would short them in the exported copper.
    pub power_star_point: Option<(f64, f64)>,
    /// Frozen wide power/ground traces, ready to become fixed copper.
    pub traces: Vec<PreRouteTrace>,
}

/// True for the nets module 5 fixes up front: power and ground.
pub fn is_pre_routed_class(class: NetClass) -> bool {
    matches!(class, NetClass::Power | NetClass::Ground)
}

#[derive(Clone, Copy)]
struct Step {
    f: f32,
    g: f32,
    cell: usize,
}
impl PartialEq for Step {
    fn eq(&self, other: &Self) -> bool {
        self.cell == other.cell
    }
}
impl Eq for Step {}
impl PartialOrd for Step {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Step {
    fn cmp(&self, other: &Self) -> Ordering {
        other.f.partial_cmp(&self.f).unwrap_or(Ordering::Equal)
    }
}

/// A star spoke that walks around foreign pads and earlier fixed rails.
/// Falls back to the straight spoke only when no grid path exists.
fn spoke_path(
    board: &Board,
    from: (f64, f64),
    to: (f64, f64),
    width: f64,
    net: usize,
    occupancy: &Occupancy,
    grid: f64,
) -> Vec<(f64, f64)> {
    let nx = (board.width_mm / grid).floor() as usize + 1;
    let ny = (board.height_mm / grid).floor() as usize + 1;
    let to_cell = |p: (f64, f64)| {
        let x = (p.0 / grid).round().clamp(0.0, (nx - 1) as f64) as usize;
        let y = (p.1 / grid).round().clamp(0.0, (ny - 1) as f64) as usize;
        y * nx + x
    };
    let to_point = |c: usize| ((c % nx) as f64 * grid, (c / nx) as f64 * grid);
    let start = to_cell(from);
    let goal = to_cell(to);
    if start == goal {
        return vec![from, to];
    }
    // Keep a margin: the delivered spoke ends at exact pad/star coordinates,
    // which are not grid-aligned and can eat a few tenths of a millimetre.
    let clearance = board.preset.min_clearance_mm + 0.2;
    let allowed = |c: usize| occupancy.allows(to_point(c), width / 2.0, clearance, net);
    let heuristic = |c: usize| {
        (((c % nx) as f32 - (goal % nx) as f32).abs()
            + ((c / nx) as f32 - (goal / nx) as f32).abs())
            * grid as f32
    };

    let mut best = vec![f32::INFINITY; nx * ny];
    let mut parent = vec![usize::MAX; nx * ny];
    let mut heap = BinaryHeap::new();
    best[start] = 0.0;
    heap.push(Step { f: heuristic(start), g: 0.0, cell: start });
    let mut found = false;
    while let Some(step) = heap.pop() {
        if step.g > best[step.cell] + 1e-6 {
            continue;
        }
        if step.cell == goal {
            found = true;
            break;
        }
        let x = (step.cell % nx) as i32;
        let y = (step.cell / nx) as i32;
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let (nx2, ny2) = (x + dx, y + dy);
            if nx2 < 0 || ny2 < 0 || nx2 as usize >= nx || ny2 as usize >= ny {
                continue;
            }
            let c = ny2 as usize * nx + nx2 as usize;
            if !allowed(c) {
                continue;
            }
            let g = step.g + grid as f32;
            if g + 1e-6 >= best[c] {
                continue;
            }
            best[c] = g;
            parent[c] = step.cell;
            heap.push(Step { f: g + heuristic(c), g, cell: c });
        }
    }
    if !found {
        return vec![from, to];
    }
    let mut cells = Vec::new();
    let mut cursor = goal;
    loop {
        cells.push(cursor);
        let next = parent[cursor];
        if next == usize::MAX {
            break;
        }
        cursor = next;
    }
    cells.reverse();
    let mut points: Vec<(f64, f64)> = cells.into_iter().map(to_point).collect();
    if let Some(first) = points.first_mut() {
        *first = from;
    }
    if let Some(last) = points.last_mut() {
        *last = to;
    }
    // Merge collinear runs: an A* path is a staircase of one-cell steps, but
    // the delivered copper is a handful of long, straight segments.
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for point in points {
        let n = merged.len();
        if n >= 2 {
            let (a, b) = (merged[n - 2], merged[n - 1]);
            let cross = (b.0 - a.0) * (point.1 - b.1) - (b.1 - a.1) * (point.0 - b.0);
            let forward = (b.0 - a.0) * (point.0 - b.0) + (b.1 - a.1) * (point.1 - b.1);
            if cross.abs() < 1e-9 && forward > 0.0 {
                merged[n - 1] = point;
                continue;
            }
        }
        merged.push(point);
    }
    merged
}

/// Build the pre-route data for `board` — no side effects, pure conversion
/// of the board's existing connectivity into the star-radiated wide traces.
pub fn preroute(board: &Board) -> PreRoute {
    let mut signal_board = board.clone();
    signal_board
        .nets
        .retain(|n| !is_pre_routed_class(n.class));

    let star_point = board.star_ground_point().or_else(|| {
        if board.width_mm > 0.0 && board.height_mm > 0.0 {
            Some((board.width_mm / 2.0, board.height_mm / 2.0))
        } else {
            None
        }
    });

    // Each power net gets its own star at the centroid of its terminals.
    // One common power star would short distinct supplies (V+ and V−), and a
    // power star at the ground star would short every rail to ground.
    let power_stars: std::collections::HashMap<&str, (f64, f64)> = board
        .nets
        .iter()
        .filter(|n| n.class == NetClass::Power)
        .filter_map(|n| {
            let points = board.net_terminals(n);
            if points.is_empty() {
                return None;
            }
            let count = points.len() as f64;
            Some((
                n.name.as_str(),
                (
                    points.iter().map(|p| p.0).sum::<f64>() / count,
                    points.iter().map(|p| p.1).sum::<f64>() / count,
                ),
            ))
        })
        .collect();
    let power_star_point = board
        .nets
        .iter()
        .filter(|n| n.class == NetClass::Power)
        .find_map(|n| power_stars.get(n.name.as_str()).copied());

    let mut traces = Vec::new();
    let mut occupancy = Occupancy::new(board, &[]);
    for net in board.nets.iter().filter(|n| is_pre_routed_class(n.class)) {
        let star = if net.class == NetClass::Ground {
            star_point
        } else {
            power_stars.get(net.name.as_str()).copied()
        };
        let Some(star) = star else { continue };
        let width = net.class.track_width_mm();
        let net_index = board.nets.iter().position(|n| n.name == net.name).unwrap_or(usize::MAX);
        for (x, y) in board.net_terminals(net) {
            // Terminals at the star are already on it; nothing to freeze.
            if (x - star.0).hypot(y - star.1) < 0.2 {
                continue;
            }
            let path = spoke_path(board, (x, y), star, width, net_index, &occupancy, 0.25);
            for pair in path.windows(2) {
                let (x1, y1) = pair[0];
                let (x2, y2) = pair[1];
                if (x1 - x2).hypot(y1 - y2) < 1e-6 {
                    continue;
                }
                traces.push(PreRouteTrace {
                    net: net.name.clone(),
                    x1,
                    y1,
                    x2,
                    y2,
                    width_mm: width,
                    layer: "F.Cu".to_string(),
                });
                occupancy.add_track(x1, y1, x2, y2, width / 2.0, net_index);
            }
        }
    }

    PreRoute {
        board: signal_board,
        star_point,
        power_star_point,
        traces,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{Footprint, LutPreset, Net, PadRef};
    use crate::boards;

    /// A board with two ground pads, one power pad and two signal nets.
    fn board_with_nets() -> Board {
        let mut board = Board::new("preroute_test", 20.0, 15.0, LutPreset::reliable());
        board.add_footprint(Footprint::new("G1", "g1", NetClass::Ground, 2.0, 2.0, 2.0, 2.0));
        board.add_footprint(Footprint::new("G2", "g2", NetClass::Ground, 18.0, 12.0, 2.0, 2.0));
        board.add_footprint(Footprint::new("V1", "v1", NetClass::Power, 10.0, 2.0, 2.0, 2.0));
        board.add_footprint(Footprint::new("V1B", "v1b", NetClass::Power, 14.0, 2.0, 2.0, 2.0));
        board.add_footprint(Footprint::new("V2", "v2", NetClass::Power, 8.0, 12.0, 2.0, 2.0));
        board.add_footprint(Footprint::new("S1", "s1", NetClass::Signal, 4.0, 8.0, 2.0, 2.0));
        board.add_footprint(Footprint::new("S2", "s2", NetClass::Signal, 16.0, 8.0, 2.0, 2.0));
        board.add_net(Net::new(
            "GND",
            NetClass::Ground,
            vec![PadRef::new("G1", 0), PadRef::new("G2", 0)],
        ));
        board.add_net(Net::new(
            "VCC",
            NetClass::Power,
            vec![PadRef::new("V1", 0), PadRef::new("V1B", 0)],
        ));
        board.add_net(Net::new(
            "SIG",
            NetClass::Signal,
            vec![PadRef::new("S1", 0), PadRef::new("S2", 0)],
        ));
        board
    }

    #[test]
    fn router_gets_only_signal_nets() {
        let board = board_with_nets();
        let pre = preroute(&board);
        let names: Vec<&str> = pre.board.nets.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["SIG"]);
        assert!(pre.board.nets.iter().all(|n| !is_pre_routed_class(n.class)));
    }

    #[test]
    fn ground_star_is_fixed_with_wide_ground_traces() {
        let board = board_with_nets();
        let pre = preroute(&board);
        let star = pre.star_point.expect("star point exists");
        let gnd: Vec<&PreRouteTrace> =
            pre.traces.iter().filter(|t| t.net == "GND").collect();
        assert!(gnd.len() >= 2, "both ground pads are connected to the star");
        assert!(
            gnd.iter().any(|t| (t.x2, t.y2) == star),
            "the last spoke reaches the star"
        );
        assert!(gnd.iter().all(|t| t.width_mm == 0.6 && t.layer == "F.Cu"));
    }

    #[test]
    fn power_traces_are_wide() {
        let board = board_with_nets();
        let pre = preroute(&board);
        let vcc: Vec<&PreRouteTrace> =
            pre.traces.iter().filter(|t| t.net == "VCC").collect();
        assert!(!vcc.is_empty());
        assert!(vcc.iter().all(|t| t.width_mm == 0.8 && t.net == "VCC"));
    }

    #[test]
    fn power_and_ground_never_share_a_star_point() {
        let mut board = board_with_nets();
        board.add_footprint(Footprint::new("V2B", "v2b", NetClass::Power, 12.0, 12.0, 2.0, 2.0));
        board.add_net(Net::new(
            "VEE",
            NetClass::Power,
            vec![PadRef::new("V2", 0), PadRef::new("V2B", 0)],
        ));
        let pre = preroute(&board);
        let endpoints: Vec<(&str, (f64, f64))> = pre
            .traces
            .iter()
            .map(|t| (t.net.as_str(), (t.x2, t.y2)))
            .collect();
        for (net, a) in &endpoints {
            for (other, b) in &endpoints {
                if net != other {
                    assert!(
                        (a.0 - b.0).abs() > 1e-9 || (a.1 - b.1).abs() > 1e-9,
                        "{net} and {other} share star {a:?}: {endpoints:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn star_falls_back_to_the_board_center() {
        let mut board = board_with_nets();
        // A board with no ground nets has no gravity point; use the centre.
        board.nets.retain(|n| n.class != NetClass::Ground);
        let pre = preroute(&board);
        assert_eq!(pre.star_point, Some((10.0, 7.5)));
    }

    #[test]
    fn builtin_boards_lose_no_signal_nets() {
        for board in boards::all_boards() {
            let pre = preroute(&board);
            assert!(
                pre.board.nets.iter().all(|n| !is_pre_routed_class(n.class)),
                "{} keeps a power/ground net for the router",
                board.name
            );
        }
    }
}
