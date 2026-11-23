//! Module 7 — Jumper/Bridge Handler.
//!
//! Runs *after* the router. Where a single-layer board would need a via, this
//! module breaks the copper and bridges the gap with a pair of through-hole
//! pads and an insulated jumper wire instead:
//!
//! ```text
//! ────────o        o────────      o────o = physical jumper (not copper)
//!         A        B
//! ```
//!
//! The wire is a manufacturing instruction recorded in [`Jumper`], deliberately
//! *not* a copper segment: it crosses over other traces precisely because it
//! is insulated. Both jumper pads are added as nodes of the original net, so
//! the net stays connected in the board model while the router's via budget
//! drops by one per replaced via.
//!
//! The handler is pure: it clones the board and the routing result, validates
//! every requested break against real millimetre geometry and module 4's
//! profile, and records the module 4 verdict on the modified copper.

use crate::board::{Board, Footprint, NetClass, Pad, PadRef};
use crate::router::{RouteSegment, RoutingResult};
use crate::rules::{LutRules, RuleReport};
use serde::{Deserialize, Serialize};

/// Default THT pad size for a hand-soldered jumper.
pub const DEFAULT_PAD_SIZE_MM: f64 = 1.6;
/// Default drill of a jumper pad.
pub const DEFAULT_DRILL_MM: f64 = 0.8;
/// Default distance between the two pad centres (0.1 inch).
pub const DEFAULT_SPAN_MM: f64 = 2.54;
/// Default diameter of the insulated wire.
pub const DEFAULT_WIRE_DIAMETER_MM: f64 = 0.6;

const EPS: f64 = 1e-9;
const TOUCH_EPS: f64 = 1e-6;
const FOOTPRINT_MARGIN_MM: f64 = 0.4;

/// Physical geometry of the jumpers this handler is allowed to generate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JumperConfig {
    pub pad_size_mm: f64,
    pub drill_mm: f64,
    pub span_mm: f64,
    pub wire_diameter_mm: f64,
}

impl Default for JumperConfig {
    fn default() -> Self {
        Self {
            pad_size_mm: DEFAULT_PAD_SIZE_MM,
            drill_mm: DEFAULT_DRILL_MM,
            span_mm: DEFAULT_SPAN_MM,
            wire_diameter_mm: DEFAULT_WIRE_DIAMETER_MM,
        }
    }
}

impl JumperConfig {
    pub fn new() -> Self {
        Self::default()
    }

    /// All four dimensions must be finite, positive and physically possible.
    pub fn is_valid(&self) -> bool {
        [
            self.pad_size_mm,
            self.drill_mm,
            self.span_mm,
            self.wire_diameter_mm,
        ]
        .iter()
        .all(|v| v.is_finite() && *v > 0.0)
            && self.drill_mm < self.pad_size_mm
            && self.wire_diameter_mm < self.pad_size_mm
    }

    /// The two pads must also be far enough apart to keep the LUT clearance
    /// between their copper rings.
    pub fn is_valid_for(&self, rules: &LutRules) -> bool {
        self.is_valid() && self.span_mm + EPS >= self.pad_size_mm + rules.min_clearance_mm
    }
}

/// Plan an insulated-wire jumper for a net that has no routed copper at all.
/// The two existing terminals become the jumper pads; no extra copper is
/// added, so the board stays manufacturable by the single-layer LUT process.
pub fn bridge_failed_net(
    board: &Board,
    net_name: &str,
    config: &JumperConfig,
) -> Option<Jumper> {
    let net = board.nets.iter().find(|n| n.name == net_name)?;
    let terminals = board.net_terminals(net);
    if terminals.len() != 2 {
        return None;
    }
    let (a, b) = (terminals[0], terminals[1]);
    let length = (a.0 - b.0).hypot(a.1 - b.1);
    if length < config.span_mm {
        return None;
    }
    Some(Jumper {
        refdes: format!("JP_{}", net_name.replace(['+', '-', '/'], "_")),
        net: net_name.into(),
        class: net.class,
        pad_a: a,
        pad_b: b,
        pad_size_mm: config.pad_size_mm,
        wire_length_mm: length,
        wire_diameter_mm: config.wire_diameter_mm,
        replaces_via: false,
    })
}

/// Insert the planned jumpers as real footprints and pads into the exported
/// board. The insulated wire is not copper, so the JP pads intentionally stay
/// on net 0: KiCad must not expect a copper connection between them.
pub fn apply_jumpers(board: &Board, jumpers: &[Jumper]) -> Board {
    let mut out = board.clone();
    // A jumper is an insulated wire, not copper. Remove the corresponding net
    // from the exported netlist so KiCad does not report an unconnected copper
    // item; the physical connection is documented in jumpers.csv.
    for jumper in jumpers {
        out.nets.retain(|net| net.name != jumper.net);
    }
    for jumper in jumpers {
        let margin = jumper.pad_size_mm;
        let x0 = jumper.pad_a.0.min(jumper.pad_b.0) - margin;
        let y0 = jumper.pad_a.1.min(jumper.pad_b.1) - margin;
        let width = (jumper.pad_a.0 - jumper.pad_b.0).abs() + 2.0 * margin;
        let height = (jumper.pad_a.1 - jumper.pad_b.1).abs() + 2.0 * margin;
        let pads = [jumper.pad_a, jumper.pad_b]
            .iter()
            .map(|p| Pad::new(p.0 - x0, p.1 - y0, jumper.pad_size_mm, 0.8))
            .collect();
        out.add_footprint(
            Footprint::new(&jumper.refdes, "WIRE-JUMPER", NetClass::Signal, x0, y0, width, height)
                .with_pads(pads),
        );
    }
    out
}

/// One requested break in a routed segment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BreakRequest {
    pub net: String,
    /// Index into the routing result the handler was called with.
    pub segment: usize,
    /// Fraction along that segment, strictly between 0 and 1.
    pub at: f64,
    /// Set when this jumper takes the place of a via.
    #[serde(default)]
    pub replaces_via: bool,
}

impl BreakRequest {
    pub fn new(net: &str, segment: usize, at: f64) -> Self {
        Self {
            net: net.into(),
            segment,
            at,
            replaces_via: false,
        }
    }

    pub fn replacing_via(mut self) -> Self {
        self.replaces_via = true;
        self
    }
}

/// A finished jumper: two pads on the board plus the insulated wire between.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Jumper {
    pub refdes: String,
    pub net: String,
    pub class: NetClass,
    /// Board coordinates of the two pad centres.
    pub pad_a: (f64, f64),
    pub pad_b: (f64, f64),
    pub pad_size_mm: f64,
    /// Length of the physical wire, equal to the pad spacing.
    pub wire_length_mm: f64,
    pub wire_diameter_mm: f64,
    pub replaces_via: bool,
}

impl Jumper {
    pub fn midpoint(&self) -> (f64, f64) {
        (
            (self.pad_a.0 + self.pad_b.0) / 2.0,
            (self.pad_a.1 + self.pad_b.1) / 2.0,
        )
    }
}

/// A request that could not be placed, with the reason it was refused.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct JumperRejection {
    pub net: String,
    pub segment: usize,
    pub reason: String,
}

impl std::fmt::Display for JumperRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} segment {}: {}", self.net, self.segment, self.reason)
    }
}

/// Everything the rest of the pipeline needs after the jumpers are inserted.
#[derive(Debug, Serialize)]
pub struct JumperResult {
    /// Copy of the board with the JP footprints and their net nodes added.
    pub board: Board,
    /// Copy of the routing with the broken segments split and vias removed.
    pub routing: RoutingResult,
    pub jumpers: Vec<Jumper>,
    pub rejected: Vec<JumperRejection>,
    /// Module 4 verdict on the board before any change.
    pub pre_rules: RuleReport,
    /// Module 4 verdict on the final copper.
    pub post_rules: RuleReport,
    pub log: Vec<String>,
    /// Via count reported by the router, kept for transparency.
    pub vias_before: usize,
}

impl JumperResult {
    pub fn is_ok(&self) -> bool {
        self.rejected.is_empty()
    }

    /// True only when every request was placed, module 4 is clean on both sides
    /// and no via survives in the copper plan.
    pub fn is_acceptable(&self) -> bool {
        self.is_ok() && self.pre_rules.is_ok() && self.post_rules.is_ok() && self.routing.vias == 0
    }

    pub fn vias_avoided(&self) -> usize {
        self.vias_before.saturating_sub(self.routing.vias)
    }

    pub fn total_wire_mm(&self) -> f64 {
        self.jumpers.iter().map(|j| j.wire_length_mm).sum()
    }

    pub fn summary(&self) -> String {
        format!(
            "{} jumper(s), {} via(s) avoided, {} rejected, wire {:.2} mm — {}",
            self.jumpers.len(),
            self.vias_avoided(),
            self.rejected.len(),
            self.total_wire_mm(),
            self.post_rules.summary()
        )
    }
}

/// Post-router bridge handler. Uses module 4's profile for every clearance
/// decision; it never decides routes or rewrites unrelated copper.
#[derive(Debug, Clone)]
pub struct JumperHandler {
    pub rules: LutRules,
    pub config: JumperConfig,
}

impl JumperHandler {
    pub fn new(rules: LutRules, config: JumperConfig) -> Self {
        Self { rules, config }
    }

    pub fn reliable() -> Self {
        Self::new(LutRules::reliable(), JumperConfig::default())
    }

    /// Apply `requests` to copies of `board` and `routing`. No input is mutated.
    pub fn apply(
        &self,
        board: &Board,
        routing: &RoutingResult,
        requests: &[BreakRequest],
    ) -> JumperResult {
        let mut out = JumperResult {
            board: board.clone(),
            routing: RoutingResult {
                segments: routing.segments.clone(),
                routed_nets: routing.routed_nets.clone(),
                failed_nets: routing.failed_nets.clone(),
                total_length_mm: routing.total_length_mm,
                vias: routing.vias,
                via_objects: routing.via_objects.clone(),
            },
            jumpers: Vec::new(),
            rejected: Vec::new(),
            pre_rules: self.rules.check_board(board),
            post_rules: RuleReport::default(),
            log: Vec::new(),
            vias_before: routing.vias,
        };

        if !self.config.is_valid_for(&self.rules) {
            for request in requests {
                out.rejected.push(JumperRejection {
                    net: request.net.clone(),
                    segment: request.segment,
                    reason: "jumper geometry is not manufacturable".into(),
                });
            }
            out.post_rules = self.rules.check_routing(&out.board, &out.routing);
            return out;
        }

        let mut next_id = next_jumper_id(board);
        for request in requests {
            if let Err(reason) = self.apply_one(&mut out, request, &mut next_id, board, routing) {
                out.rejected.push(JumperRejection {
                    net: request.net.clone(),
                    segment: request.segment,
                    reason,
                });
            }
        }

        out.routing.total_length_mm = out.routing.segments.iter().map(|s| s.length_mm()).sum();
        out.post_rules = self.rules.check_routing(&out.board, &out.routing);
        out.log.push(out.summary());
        out
    }

    fn apply_one(
        &self,
        out: &mut JumperResult,
        request: &BreakRequest,
        next_id: &mut usize,
        board: &Board,
        routing: &RoutingResult,
    ) -> Result<(), String> {
        let net = board
            .nets
            .iter()
            .find(|n| n.name == request.net)
            .ok_or_else(|| "unknown net".to_string())?;
        let original = routing
            .segments
            .get(request.segment)
            .ok_or_else(|| "segment index out of range".to_string())?;
        if original.net != request.net {
            return Err("segment belongs to a different net".into());
        }
        if !request.at.is_finite() || request.at <= 0.0 || request.at >= 1.0 {
            return Err("break position must lie inside the segment".into());
        }
        let length = original.length_mm();
        if !length.is_finite() || length <= EPS {
            return Err("segment has no usable length".into());
        }

        let (ux, uy) = (
            (original.x2_mm - original.x1_mm) / length,
            (original.y2_mm - original.y1_mm) / length,
        );
        let (px, py) = (
            original.x1_mm + (original.x2_mm - original.x1_mm) * request.at,
            original.y1_mm + (original.y2_mm - original.y1_mm) * request.at,
        );
        let half = self.config.span_mm / 2.0;

        // A previous break may have split the requested segment. Find the
        // current piece that still contains the requested point.
        let index = out
            .routing
            .segments
            .iter()
            .position(|s| {
                if s.net != request.net {
                    return false;
                }
                let len = s.length_mm();
                if len <= EPS {
                    return false;
                }
                let dx = s.x2_mm - s.x1_mm;
                let dy = s.y2_mm - s.y1_mm;
                let t = ((px - s.x1_mm) * dx + (py - s.y1_mm) * dy) / (len * len);
                let (cx, cy) = (s.x1_mm + t * dx, s.y1_mm + t * dy);
                t > 0.0 && t < 1.0 && (px - cx).hypot(py - cy) <= TOUCH_EPS
            })
            .ok_or_else(|| "break point is not on a segment of this net".to_string())?;
        let current = &out.routing.segments[index];
        let current_len = current.length_mm();
        let dx = current.x2_mm - current.x1_mm;
        let dy = current.y2_mm - current.y1_mm;
        let cut =
            ((px - current.x1_mm) * dx + (py - current.y1_mm) * dy) / (current_len * current_len);
        if cut * current_len <= half + TOUCH_EPS || (1.0 - cut) * current_len <= half + TOUCH_EPS {
            return Err("not enough copper around the break for the jumper span".into());
        }

        let pad_a = (px - ux * half, py - uy * half);
        let pad_b = (px + ux * half, py + uy * half);
        self.check_placement(board, routing, out, request, &[pad_a, pad_b])?;

        let size = self.config.pad_size_mm;
        let width = (ux.abs() * self.config.span_mm + size + FOOTPRINT_MARGIN_MM).max(size);
        let height = (uy.abs() * self.config.span_mm + size + FOOTPRINT_MARGIN_MM).max(size);
        let ox = px - width / 2.0;
        let oy = py - height / 2.0;
        if ox < 0.0 || oy < 0.0 || ox + width > board.width_mm || oy + height > board.height_mm {
            return Err("jumper footprint extends outside the board".into());
        }

        let refdes = format!("JP{next_id}");
        let footprint = Footprint::new(&refdes, "JUMPER", net.class, ox, oy, width, height)
            .with_pads(vec![
                Pad::new(pad_a.0 - ox, pad_a.1 - oy, size, self.config.drill_mm),
                Pad::new(pad_b.0 - ox, pad_b.1 - oy, size, self.config.drill_mm),
            ]);
        out.board.add_footprint(footprint);
        if let Some(modified) = out.board.nets.iter_mut().find(|n| n.name == request.net) {
            modified.nodes.push(PadRef::new(&refdes, 0));
            modified.nodes.push(PadRef::new(&refdes, 1));
        }
        *next_id += 1;

        let (sx, sy, ex, ey) = (current.x1_mm, current.y1_mm, current.x2_mm, current.y2_mm);
        let left = RouteSegment::new(sx, sy, pad_a.0, pad_a.1, &request.net, current.class)
            .with_width(current.width_mm);
        let right = RouteSegment::new(pad_b.0, pad_b.1, ex, ey, &request.net, current.class)
            .with_width(current.width_mm);
        out.routing.segments[index] = left;
        out.routing.segments.insert(index + 1, right);

        out.jumpers.push(Jumper {
            refdes: refdes.clone(),
            net: request.net.clone(),
            class: net.class,
            pad_a,
            pad_b,
            pad_size_mm: size,
            wire_length_mm: self.config.span_mm,
            wire_diameter_mm: self.config.wire_diameter_mm,
            replaces_via: request.replaces_via,
        });
        if request.replaces_via {
            out.routing.vias = out.routing.vias.saturating_sub(1);
        }
        out.log.push(format!(
            "{refdes}: {} broken at ({px:.2}, {py:.2}) mm, {:.2} mm insulated wire",
            request.net, self.config.span_mm
        ));
        Ok(())
    }

    /// Keep the new pads away from every pad and track of another net.
    fn check_placement(
        &self,
        board: &Board,
        routing: &RoutingResult,
        out: &JumperResult,
        request: &BreakRequest,
        pads: &[(f64, f64)],
    ) -> Result<(), String> {
        let radius = self.config.pad_size_mm / 2.0;
        let clearance = self.rules.min_clearance_mm;

        for &(x, y) in pads {
            let edge = board.edge_clearance_mm();
            if x - radius < edge
                || y - radius < edge
                || board.width_mm - x - radius < edge
                || board.height_mm - y - radius < edge
            {
                return Err("jumper pad is too close to the board edge".into());
            }
            for ((ox, oy), osize, owner) in board.all_pads() {
                if owner.as_deref() == Some(request.net.as_str()) {
                    continue;
                }
                if (x - ox).hypot(y - oy) - radius - osize / 2.0 < clearance - TOUCH_EPS {
                    return Err("jumper pad is too close to another pad".into());
                }
            }
            let probe = RouteSegment::new(x, y, x, y, &request.net, NetClass::Signal)
                .with_width(self.config.pad_size_mm);
            for other in &routing.segments {
                if other.net == request.net {
                    continue;
                }
                let (distance, _, _) = crate::rules::segment_distance_mm(&probe, other);
                if distance - radius - other.width_mm / 2.0 < clearance - TOUCH_EPS {
                    return Err("jumper pad is too close to another net's track".into());
                }
            }
            for jumper in &out.jumpers {
                if jumper.net == request.net {
                    continue;
                }
                for &(jx, jy) in &[jumper.pad_a, jumper.pad_b] {
                    if (x - jx).hypot(y - jy) - radius - jumper.pad_size_mm / 2.0
                        < clearance - TOUCH_EPS
                    {
                        return Err("jumper pad is too close to another jumper".into());
                    }
                }
            }
        }
        Ok(())
    }
}

/// Next unused JP reference, respecting jumpers already on the board.
fn next_jumper_id(board: &Board) -> usize {
    let mut highest = 0;
    for footprint in &board.footprints {
        if let Some(number) = footprint.refdes.strip_prefix("JP") {
            if let Ok(parsed) = number.parse::<usize>() {
                highest = highest.max(parsed);
            }
        }
    }
    highest + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{Footprint, LutPreset, Net};

    fn sample_board() -> Board {
        let mut board = Board::new("jumper-test", 80.0, 60.0, LutPreset::reliable());
        let fp =
            Footprint::new("R1", "10k", NetClass::Signal, 10.0, 30.0, 24.0, 2.0).with_pads(vec![
                Pad::new(0.0, 0.0, 1.6, 0.8),
                Pad::new(20.0, 0.0, 1.6, 0.8),
            ]);
        board.add_footprint(fp);
        board.add_net(Net::new(
            "SIG",
            NetClass::Signal,
            vec![PadRef::new("R1", 0), PadRef::new("R1", 1)],
        ));
        board
    }

    fn routing(segments: Vec<RouteSegment>, vias: usize) -> RoutingResult {
        let total_length_mm = segments_len(&segments);
        RoutingResult {
            segments,
            routed_nets: vec!["SIG".into()],
            failed_nets: Vec::new(),
            total_length_mm,
            vias,
            via_objects: Vec::new(),
        }
    }

    fn segments_len(segments: &[RouteSegment]) -> f64 {
        segments.iter().map(|s| s.length_mm()).sum()
    }

    fn straight() -> RoutingResult {
        routing(
            vec![RouteSegment::new(
                10.0,
                30.0,
                30.0,
                30.0,
                "SIG",
                NetClass::Signal,
            )],
            0,
        )
    }

    // --- configuration -----------------------------------------------------

    #[test]
    fn default_config_is_manufacturable() {
        let config = JumperConfig::new();
        assert!(config.is_valid());
        assert!(config.is_valid_for(&LutRules::reliable()));
        assert_eq!(config.span_mm, 2.54);
    }

    #[test]
    fn config_rejects_bad_dimensions() {
        let mut config = JumperConfig::new();
        config.drill_mm = config.pad_size_mm;
        assert!(!config.is_valid());
        config.drill_mm = DEFAULT_DRILL_MM;
        config.wire_diameter_mm = f64::NAN;
        assert!(!config.is_valid());
        config.wire_diameter_mm = DEFAULT_WIRE_DIAMETER_MM;
        config.span_mm = 1.8;
        assert!(config.is_valid());
        assert!(!config.is_valid_for(&LutRules::reliable()));
    }

    #[test]
    fn invalid_config_refuses_every_request() {
        let mut handler = JumperHandler::reliable();
        handler.config.span_mm = 1.0;
        let result = handler.apply(
            &sample_board(),
            &straight(),
            &[BreakRequest::new("SIG", 0, 0.5)],
        );
        assert_eq!(result.rejected.len(), 1);
        assert!(result.jumpers.is_empty());
        assert!(result.rejected[0].reason.contains("manufacturable"));
    }

    // --- happy path --------------------------------------------------------

    #[test]
    fn inserts_two_pads_and_an_insulated_wire() {
        let result = JumperHandler::reliable().apply(
            &sample_board(),
            &straight(),
            &[BreakRequest::new("SIG", 0, 0.5)],
        );
        assert!(result.is_ok());
        assert!(result.post_rules.is_ok(), "{}", result.post_rules);
        assert!(result.is_acceptable());
        assert_eq!(result.jumpers.len(), 1);

        let jumper = &result.jumpers[0];
        assert_eq!(jumper.refdes, "JP1");
        assert_eq!(jumper.net, "SIG");
        assert_eq!(jumper.class, NetClass::Signal);
        assert!((jumper.wire_length_mm - 2.54).abs() < 1e-9);
        assert_eq!(jumper.wire_diameter_mm, 0.6);
        assert!(!jumper.replaces_via);
        let mid = jumper.midpoint();
        assert!((mid.0 - 20.0).abs() < 1e-9 && (mid.1 - 30.0).abs() < 1e-9);

        let jp = result.board.footprint("JP1").expect("JP footprint");
        assert_eq!(jp.pads.len(), 2);
        assert_eq!(jp.value, "JUMPER");
        assert_eq!(jp.pad_world(0), Some(jumper.pad_a));
        assert_eq!(jp.pad_world(1), Some(jumper.pad_b));
        let net = result.board.nets.iter().find(|n| n.name == "SIG").unwrap();
        assert_eq!(net.nodes.len(), 4);
        assert_eq!(net.nodes[2].refdes, "JP1");
        assert_eq!(net.nodes[3].pad, 1);
    }

    #[test]
    fn splits_copper_and_keeps_class_and_width() {
        let result = JumperHandler::reliable().apply(
            &sample_board(),
            &straight(),
            &[BreakRequest::new("SIG", 0, 0.5)],
        );
        assert_eq!(result.routing.segments.len(), 2);
        let left = &result.routing.segments[0];
        let right = &result.routing.segments[1];
        assert!((left.x1_mm - 10.0).abs() < 1e-9);
        assert!((left.x2_mm - result.jumpers[0].pad_a.0).abs() < 1e-9);
        assert!((right.x1_mm - result.jumpers[0].pad_b.0).abs() < 1e-9);
        assert!((right.x2_mm - 30.0).abs() < 1e-9);
        assert_eq!(left.class, NetClass::Signal);
        assert_eq!(right.width_mm, 0.4);
        assert!((result.routing.total_length_mm - 17.46).abs() < 1e-9);
        // The insulated wire is never emitted as copper.
        assert!(result
            .routing
            .segments
            .iter()
            .all(|s| (s.x2_mm - result.jumpers[0].pad_b.0).abs() > 1e-9
                || (s.x1_mm - result.jumpers[0].pad_a.0).abs() > 1e-9));
    }

    #[test]
    fn a_via_replacing_break_removes_exactly_one_via() {
        let handler = JumperHandler::reliable();
        let via_free = handler.apply(
            &sample_board(),
            &straight(),
            &[BreakRequest::new("SIG", 0, 0.5)],
        );
        assert_eq!(via_free.vias_before, 0);
        assert_eq!(via_free.routing.vias, 0);
        assert_eq!(via_free.vias_avoided(), 0);

        let routed = routing(straight().segments, 1);
        let replaced = handler.apply(
            &sample_board(),
            &routed,
            &[BreakRequest::new("SIG", 0, 0.5).replacing_via()],
        );
        assert_eq!(replaced.vias_before, 1);
        assert_eq!(replaced.routing.vias, 0);
        assert_eq!(replaced.vias_avoided(), 1);
        assert!(replaced.jumpers[0].replaces_via);
        assert!(replaced.is_acceptable());
    }

    // --- refusals ----------------------------------------------------------

    #[test]
    fn refuses_unknown_net_and_bad_segment() {
        let handler = JumperHandler::reliable();
        let board = sample_board();
        let routed = straight();
        let cases = [
            (BreakRequest::new("NOPE", 0, 0.5), "unknown net"),
            (BreakRequest::new("SIG", 7, 0.5), "out of range"),
        ];
        for (request, reason) in cases {
            let result = handler.apply(&board, &routed, std::slice::from_ref(&request));
            assert_eq!(result.rejected.len(), 1);
            assert!(result.rejected[0].reason.contains(reason));
            assert_eq!(result.rejected[0].net, request.net);
        }
    }

    #[test]
    fn refuses_wrong_net_bad_position_and_short_segment() {
        let mut other = routing(straight().segments, 0);
        other.segments[0].net = "OTHER".into();
        let handler = JumperHandler::reliable();
        let board = sample_board();

        let result = handler.apply(&board, &other, &[BreakRequest::new("SIG", 0, 0.5)]);
        assert!(result.rejected[0].reason.contains("different net"));

        for at in [0.0, 1.0, f64::NAN, -0.1] {
            let result = handler.apply(&board, &straight(), &[BreakRequest::new("SIG", 0, at)]);
            assert!(result.rejected[0].reason.contains("inside"));
        }

        let mut short = straight();
        short.segments[0] = RouteSegment::new(10.0, 30.0, 12.0, 30.0, "SIG", NetClass::Signal);
        let result = handler.apply(&board, &short, &[BreakRequest::new("SIG", 0, 0.5)]);
        assert!(result.rejected[0].reason.contains("not enough copper"));
    }

    #[test]
    fn refuses_pad_too_close_to_other_net_pad() {
        let mut board = sample_board();
        let near = Footprint::new("R2", "1k", NetClass::Signal, 18.0, 30.5, 4.0, 2.0)
            .with_pads(vec![Pad::new(2.0, 0.0, 1.6, 0.8)]);
        board.add_footprint(near);
        board.add_net(Net::new(
            "OTHER",
            NetClass::Signal,
            vec![PadRef::new("R2", 0)],
        ));
        // The OTHER net has one terminal, but module 4 is only recorded here.
        let result = JumperHandler::reliable().apply(
            &board,
            &straight(),
            &[BreakRequest::new("SIG", 0, 0.5)],
        );
        assert_eq!(result.rejected.len(), 1);
        assert!(result.rejected[0].reason.contains("another pad"));
    }

    #[test]
    fn allows_same_net_pad_at_the_break() {
        let mut board = sample_board();
        let same = Footprint::new("C1", "100n", NetClass::Signal, 18.0, 30.5, 4.0, 2.0)
            .with_pads(vec![Pad::new(2.0, 0.0, 1.6, 0.8)]);
        board.add_footprint(same);
        board.nets[0].nodes.push(PadRef::new("C1", 0));
        let result = JumperHandler::reliable().apply(
            &board,
            &straight(),
            &[BreakRequest::new("SIG", 0, 0.5)],
        );
        assert!(result.is_ok(), "{}", result.rejected[0]);
    }

    #[test]
    fn refuses_pad_too_close_to_other_net_track() {
        let mut routed = straight();
        routed.segments.push(RouteSegment::new(
            15.0,
            31.0,
            25.0,
            31.0,
            "OTHER",
            NetClass::Signal,
        ));
        let result = JumperHandler::reliable().apply(
            &sample_board(),
            &routed,
            &[BreakRequest::new("SIG", 0, 0.5)],
        );
        assert_eq!(result.rejected.len(), 1);
        assert!(result.rejected[0].reason.contains("track"));
    }

    #[test]
    fn refuses_break_too_close_to_board_edge() {
        let mut routed = straight();
        routed.segments[0] = RouteSegment::new(1.0, 30.0, 20.0, 30.0, "SIG", NetClass::Signal);
        let result = JumperHandler::reliable().apply(
            &sample_board(),
            &routed,
            &[BreakRequest::new("SIG", 0, 0.08)],
        );
        assert_eq!(result.rejected.len(), 1);
        assert!(result.rejected[0].reason.contains("board edge"));
    }

    // --- sequencing and purity --------------------------------------------

    #[test]
    fn refdes_sequence_respects_existing_jumpers() {
        let mut board = sample_board();
        let existing = Footprint::new("JP5", "JUMPER", NetClass::Signal, 5.0, 5.0, 3.0, 2.0)
            .with_pads(vec![Pad::new(0.0, 0.0, 1.6, 0.8)]);
        board.add_footprint(existing);
        let result = JumperHandler::reliable().apply(
            &board,
            &straight(),
            &[BreakRequest::new("SIG", 0, 0.5)],
        );
        assert_eq!(result.jumpers[0].refdes, "JP6");
    }

    #[test]
    fn two_breaks_on_one_segment_are_both_placed() {
        let result = JumperHandler::reliable().apply(
            &sample_board(),
            &straight(),
            &[
                BreakRequest::new("SIG", 0, 0.3),
                BreakRequest::new("SIG", 0, 0.7),
            ],
        );
        assert!(result.is_ok(), "{}", result.rejected[0]);
        assert_eq!(result.jumpers.len(), 2);
        assert_eq!(result.routing.segments.len(), 3);
        assert_eq!(result.jumpers[0].refdes, "JP1");
        assert_eq!(result.jumpers[1].refdes, "JP2");
        assert!((result.total_wire_mm() - 5.08).abs() < 1e-9);
    }

    #[test]
    fn handler_does_not_mutate_its_inputs() {
        let board = sample_board();
        let routed = straight();
        let before = board.to_json().unwrap();
        let before_segments = segments_len(&routed.segments);
        let _ = JumperHandler::reliable().apply(
            &board,
            &routed,
            &[BreakRequest::new("SIG", 0, 0.5).replacing_via()],
        );
        assert_eq!(board.to_json().unwrap(), before);
        assert_eq!(segments_len(&routed.segments), before_segments);
        assert_eq!(routed.vias, 0);
    }

    #[test]
    fn module_four_verdicts_are_recorded() {
        let mut board = sample_board();
        // A GND-named net classed as Signal trips the pre-route gate.
        board.add_net(Net::new(
            "GND",
            NetClass::Signal,
            vec![PadRef::new("R1", 0), PadRef::new("R1", 1)],
        ));
        let result = JumperHandler::reliable().apply(
            &board,
            &straight(),
            &[BreakRequest::new("SIG", 0, 0.5)],
        );
        assert!(!result.pre_rules.is_ok());
        assert!(!result.is_acceptable());
        assert!(result.jumpers.len() == 1);
    }

    #[test]
    fn summary_log_and_helpers_report_the_plan() {
        let result = JumperHandler::reliable().apply(
            &sample_board(),
            &routing(straight().segments, 1),
            &[BreakRequest::new("SIG", 0, 0.5).replacing_via()],
        );
        assert_eq!(result.log.len(), 2);
        assert!(result.log[0].contains("JP1"));
        assert!(result.summary().contains("1 jumper(s)"));
        assert!(result.summary().contains("1 via(s) avoided"));
        assert!(result.summary().contains("wire 2.54 mm"));
        assert!(result.rejected.is_empty());
        let rejection = JumperRejection {
            net: "SIG".into(),
            segment: 3,
            reason: "test".into(),
        };
        assert_eq!(rejection.to_string(), "SIG segment 3: test");
    }

    #[test]
    fn config_rejects_zero_and_inverted_dimensions() {
        let mut config = JumperConfig::new();
        config.span_mm = 0.0;
        assert!(!config.is_valid());
        config.span_mm = DEFAULT_SPAN_MM;
        config.pad_size_mm = 0.0;
        assert!(!config.is_valid());
        config.pad_size_mm = DEFAULT_PAD_SIZE_MM;
        config.wire_diameter_mm = DEFAULT_PAD_SIZE_MM;
        assert!(!config.is_valid());
    }

    #[test]
    fn via_replacement_is_clamped_at_zero() {
        let result = JumperHandler::reliable().apply(
            &sample_board(),
            &straight(),
            &[BreakRequest::new("SIG", 0, 0.5).replacing_via()],
        );
        assert_eq!(result.routing.vias, 0);
        assert_eq!(result.vias_avoided(), 0);
        assert!(result.jumpers[0].replaces_via);
    }

    #[test]
    fn refuses_jumper_too_close_to_a_previous_jumper() {
        let mut board = sample_board();
        let aux =
            Footprint::new("R3", "1k", NetClass::Signal, 10.0, 41.5, 24.0, 2.0).with_pads(vec![
                Pad::new(0.0, 0.0, 1.6, 0.8),
                Pad::new(20.0, 0.0, 1.6, 0.8),
            ]);
        board.add_footprint(aux);
        board.add_net(Net::new(
            "AUX",
            NetClass::Signal,
            vec![PadRef::new("R3", 0), PadRef::new("R3", 1)],
        ));
        let mut routed = straight();
        routed.segments.push(RouteSegment::new(
            10.0,
            31.5,
            30.0,
            31.5,
            "AUX",
            NetClass::Signal,
        ));
        let result = JumperHandler::reliable().apply(
            &board,
            &routed,
            &[
                BreakRequest::new("SIG", 0, 0.5),
                BreakRequest::new("AUX", 1, 0.5),
            ],
        );
        assert_eq!(result.jumpers.len(), 1);
        assert_eq!(result.rejected.len(), 1);
        assert!(result.rejected[0].reason.contains("another jumper"));
    }

    #[test]
    fn post_rules_catch_a_crossing_left_outside_the_break() {
        let mut routed = straight();
        routed.segments.push(RouteSegment::new(
            25.0,
            28.0,
            25.0,
            32.0,
            "OTHER",
            NetClass::Signal,
        ));
        let result = JumperHandler::reliable().apply(
            &sample_board(),
            &routed,
            &[BreakRequest::new("SIG", 0, 0.5)],
        );
        assert!(result.is_ok(), "{}", result.rejected[0]);
        assert!(!result.post_rules.is_ok());
        assert!(!result.is_acceptable());
        assert_eq!(result.post_rules.count(crate::rules::RuleId::Clearance), 1);
    }
}
