//! Geometry-based EMI proxy for routed amplifier boards.
//!
//! This is not a field solver. It extracts two things the router already
//! knows — parallel copper runs and the loop made by a net with its return —
//! and turns them into comparable numbers: estimated crosstalk in dB and loop
//! area in mm². Both are explicitly labelled `proxy` in the JSON report.

use crate::board::{Board, NetClass};
use crate::measure::NetExcitation;
use crate::router::{CopperLayer, RoutingResult};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmiConfig {
    /// Centre-line gap below which two same-layer runs count as coupled.
    #[serde(default = "default_gap")]
    pub coupling_gap_mm: f64,
    /// Coupling ratio above which a warning is raised.
    #[serde(default = "default_crosstalk")]
    pub max_crosstalk_db: f64,
    /// Loop area above which a warning is raised.
    #[serde(default = "default_loop")]
    pub max_loop_area_mm2: f64,
}

fn default_gap() -> f64 {
    1.2
}
fn default_crosstalk() -> f64 {
    -40.0
}
fn default_loop() -> f64 {
    100.0
}

impl Default for EmiConfig {
    fn default() -> Self {
        Self {
            coupling_gap_mm: default_gap(),
            max_crosstalk_db: default_crosstalk(),
            max_loop_area_mm2: default_loop(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Coupling {
    pub net_a: String,
    pub net_b: String,
    pub layer: String,
    pub overlap_mm: f64,
    pub gap_mm: f64,
    /// Rough capacitance between the two runs, in picofarads.
    pub capacitance_pf: f64,
    /// Estimated crosstalk (negative dB; closer to zero is worse).
    pub crosstalk_db: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Loop {
    pub net: String,
    pub return_net: String,
    pub area_mm2: f64,
    pub encloses_foreign_copper: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EmiReport {
    pub couplings: Vec<Coupling>,
    pub loops: Vec<Loop>,
    pub violations: Vec<String>,
    /// Congestion map: number of 2 mm cells touched by more than one net.
    pub congestion_cells: usize,
    pub peak_owners: usize,
    pub model: String,
    pub confidence: String,
    pub assumptions: Vec<String>,
}

impl EmiReport {
    pub fn is_ok(&self) -> bool {
        self.violations.is_empty()
    }
}

fn projection_overlap(
    a: (f64, f64, f64, f64),
    b: (f64, f64, f64, f64),
) -> Option<(f64, f64)> {
    let (ax, ay, bx, by) = a;
    let (cx, cy, dx, dy) = b;
    let la = (bx - ax).hypot(by - ay);
    let lb = (dx - cx).hypot(dy - cy);
    if la < 1e-6 || lb < 1e-6 {
        return None;
    }
    let ux = (bx - ax) / la;
    let uy = (by - ay) / la;
    let dot = ux * (dx - cx) / lb + uy * (dy - cy) / lb;
    if dot.abs() < 0.996 {
        return None; // ~5° tolerance
    }
    let t0 = ((cx - ax) * ux + (cy - ay) * uy) / la;
    let t1 = ((dx - ax) * ux + (dy - ay) * uy) / la;
    let lo = t0.min(t1).max(0.0);
    let hi = t0.max(t1).min(1.0);
    if hi <= lo {
        return None;
    }
    // Perpendicular distance at the midpoint of the overlap.
    let mid = (lo + hi) / 2.0;
    let px = ax + (bx - ax) * mid;
    let py = ay + (by - ay) * mid;
    let qx = cx + (dx - cx) * ((mid - t0) / (t1 - t0)).clamp(0.0, 1.0);
    let qy = cy + (dy - cy) * ((mid - t0) / (t1 - t0)).clamp(0.0, 1.0);
    Some(((hi - lo) * la, (px - qx).hypot(py - qy)))
}

fn hull(mut points: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    points.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap().then(a.1.partial_cmp(&b.1).unwrap()));
    points.dedup();
    if points.len() < 3 {
        return points;
    }
    let cross = |o: (f64, f64), a: (f64, f64), b: (f64, f64)| {
        (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
    };
    let mut lower = Vec::new();
    for p in &points {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], *p) <= 0.0 {
            lower.pop();
        }
        lower.push(*p);
    }
    let mut upper = Vec::new();
    for p in points.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], *p) <= 0.0 {
            upper.pop();
        }
        upper.push(*p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

fn area(poly: &[(f64, f64)]) -> f64 {
    if poly.len() < 3 {
        return 0.0;
    }
    let mut sum = 0.0;
    for i in 0..poly.len() {
        let p = poly[i];
        let q = poly[(i + 1) % poly.len()];
        sum += p.0 * q.1 - q.0 * p.1;
    }
    (sum / 2.0).abs()
}

fn inside(poly: &[(f64, f64)], p: (f64, f64)) -> bool {
    let mut hit = false;
    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[(i + 1) % poly.len()];
        if ((a.1 > p.1) != (b.1 > p.1))
            && (p.0 < (b.0 - a.0) * (p.1 - a.1) / (b.1 - a.1) + a.0)
        {
            hit = !hit;
        }
    }
    hit
}

/// Extract crosstalk and loop-area proxies from the delivered copper.
pub fn analyze(
    board: &Board,
    routing: &RoutingResult,
    excitations: Option<&[NetExcitation]>,
    config: &EmiConfig,
) -> EmiReport {
    let mut couplings = Vec::new();
    let mut violations = Vec::new();

    for i in 0..routing.segments.len() {
        for j in (i + 1)..routing.segments.len() {
            let a = &routing.segments[i];
            let b = &routing.segments[j];
            if a.net == b.net || a.layer != b.layer {
                continue;
            }
            let Some((overlap, gap)) = projection_overlap(
                (a.x1_mm, a.y1_mm, a.x2_mm, a.y2_mm),
                (b.x1_mm, b.y1_mm, b.x2_mm, b.y2_mm),
            ) else {
                continue;
            };
            if overlap < 0.5 || gap > config.coupling_gap_mm {
                continue;
            }
            let width = a.width_mm.min(b.width_mm);
            // Parallel-plate approximation in free space, scaled to pF/mm.
            let capacitance_pf = 0.008854 * overlap * width / gap.max(0.05);
            let ratio = capacitance_pf / 10.0;
            let crosstalk_db = -20.0 * (1.0 + ratio).log10();
            if crosstalk_db > config.max_crosstalk_db {
                violations.push(format!(
                    "crosstalk {}↔{}: {:.1} dB over {:.1} mm at {:.2} mm gap",
                    a.net, b.net, crosstalk_db, overlap, gap
                ));
            }
            couplings.push(Coupling {
                net_a: a.net.clone(),
                net_b: b.net.clone(),
                layer: match a.layer {
                    CopperLayer::FCu => "F.Cu".into(),
                    CopperLayer::BCu => "B.Cu".into(),
                },
                overlap_mm: overlap,
                gap_mm: gap,
                capacitance_pf,
                crosstalk_db,
            });
        }
    }

    let mut loops = Vec::new();
    for net in &board.nets {
        let return_name = excitations
            .and_then(|list| list.iter().find(|e| e.net == net.name).and_then(|e| e.return_net.clone()))
            .or_else(|| {
                if matches!(net.class, NetClass::Input | NetClass::Output | NetClass::Signal) {
                    board
                        .nets
                        .iter()
                        .find(|n| n.class == NetClass::Ground)
                        .map(|n| n.name.clone())
                } else {
                    None
                }
            });
        let Some(return_name) = return_name else { continue };
        let mut points: Vec<(f64, f64)> = Vec::new();
        for seg in routing.segments.iter().filter(|s| s.net == net.name || s.net == return_name) {
            points.push((seg.x1_mm, seg.y1_mm));
            points.push((seg.x2_mm, seg.y2_mm));
        }
        if points.len() < 3 {
            continue;
        }
        let boundary = hull(points);
        let loop_area = area(&boundary);
        let encloses = routing
            .segments
            .iter()
            .any(|s| {
                s.net != net.name
                    && s.net != return_name
                    && matches!(net.class, NetClass::Input)
                    && matches!(
                        board.nets.iter().find(|n| n.name == s.net).map(|n| n.class),
                        Some(NetClass::Output | NetClass::Power)
                    )
                    && inside(&boundary, ((s.x1_mm + s.x2_mm) / 2.0, (s.y1_mm + s.y2_mm) / 2.0))
            });
        if loop_area > config.max_loop_area_mm2 {
            violations.push(format!(
                "{} return loop {:.0} mm² exceeds {:.0} mm²",
                net.name, loop_area, config.max_loop_area_mm2
            ));
        }
        if encloses {
            violations.push(format!("{} loop encloses output or rail copper", net.name));
        }
        loops.push(Loop {
            net: net.name.clone(),
            return_net: return_name,
            area_mm2: loop_area,
            encloses_foreign_copper: encloses,
        });
    }

    // Congestion map on a 2 mm grid: cells touched by several nets are where
    // a later routing pass would struggle most.
    let mut cells: HashMap<(i32, i32), HashSet<&str>> = HashMap::new();
    for seg in &routing.segments {
        let x = ((seg.x1_mm + seg.x2_mm) / 4.0).floor() as i32;
        let y = ((seg.y1_mm + seg.y2_mm) / 4.0).floor() as i32;
        cells.entry((x, y)).or_default().insert(seg.net.as_str());
    }
    let congestion_cells = cells.values().filter(|nets| nets.len() > 1).count();
    let peak_owners = cells.values().map(HashSet::len).max().unwrap_or(0);

    EmiReport {
        couplings,
        loops,
        violations,
        congestion_cells,
        peak_owners,
        model: "proxy".into(),
        confidence: "low".into(),
        assumptions: vec![
            "parallel-plate capacitance proxy, no field solve".into(),
            "loop area uses the convex hull of net + return copper".into(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{Footprint, LutPreset, Net, NetClass, Pad, PadRef};
    use crate::router::RouteSegment;

    fn board() -> Board {
        let mut b = Board::new("emi-test", 60.0, 40.0, LutPreset::reliable());
        for (r, x) in [("A", 10.0), ("B", 10.0), ("G", 40.0)] {
            b.add_footprint(
                Footprint::new(r, "x", NetClass::Signal, x, 10.0, 6.0, 4.0)
                    .with_pads(vec![Pad::new(1.0, 2.0, 1.6, 0.8), Pad::new(5.0, 2.0, 1.6, 0.8)]),
            );
        }
        b.add_net(Net::new("IN", NetClass::Input, vec![PadRef::new("A", 0)]));
        b.add_net(Net::new("OUT", NetClass::Output, vec![PadRef::new("B", 0)]));
        b.add_net(Net::new("GND", NetClass::Ground, vec![PadRef::new("G", 0)]));
        b
    }

    #[test]
    fn parallel_runs_are_detected_as_coupling() {
        let routing = RoutingResult {
            segments: vec![
                RouteSegment::new(2.0, 5.0, 40.0, 5.0, "IN", NetClass::Input),
                RouteSegment::new(2.0, 6.0, 40.0, 6.0, "OUT", NetClass::Output),
            ],
            ..Default::default()
        };
        let report = analyze(&board(), &routing, None, &EmiConfig::default());
        assert_eq!(report.couplings.len(), 1);
        assert!((report.couplings[0].overlap_mm - 38.0).abs() < 1e-6);
        assert!(!report.violations.is_empty());
    }

    #[test]
    fn distant_runs_are_not_coupled() {
        let routing = RoutingResult {
            segments: vec![
                RouteSegment::new(2.0, 5.0, 40.0, 5.0, "IN", NetClass::Input),
                RouteSegment::new(2.0, 20.0, 40.0, 20.0, "OUT", NetClass::Output),
            ],
            ..Default::default()
        };
        let report = analyze(&board(), &routing, None, &EmiConfig::default());
        assert!(report.couplings.is_empty());
        assert!(report.is_ok());
    }
}
