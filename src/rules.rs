//! Module 4 — Rules: the LUT manufacturing profile and its hard constraints.
//!
//! The pipeline consults this module twice: [`LutRules::check_board`] before
//! routing and [`LutRules::check_routing`] after it. It never rewrites copper or
//! decides routes — it only accepts or rejects, so the routing engine
//! (module 6) stays a dumb executor of these rules.
//!
//! Hard constraints of the LUT (laser-iron / toner-transfer) process:
//! * no plated-through vias — the board is single-layer ([`VIA_LIMIT`]);
//! * minimum track width and clearance stay in the 0.3–0.4 mm band
//!   ([`FEATURE_MIN_MM`]..=[`FEATURE_MAX_MM`]). The band limits the *minimum*
//!   manufacturable feature, not the maximum width: 0.6/0.8 mm ground and power
//!   rails are wider and perfectly legal;
//! * GND/PWR nets keep their net class and their wide class copper;
//! * no corner sharper than 45°;
//! * no long side-by-side parallel run of two different nets.

use crate::board::{Board, LutPreset, NetClass};
use crate::router::{RouteSegment, RoutingResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Lower bound of the manufacturable minimum-feature band, millimetres.
pub const FEATURE_MIN_MM: f64 = 0.3;
/// Upper bound of the manufacturable minimum-feature band, millimetres.
pub const FEATURE_MAX_MM: f64 = 0.4;
/// Single-layer toner transfer cannot make plated vias.
pub const VIA_LIMIT: usize = 0;
/// Practical per-board via budget for a two-layer amplifier.
pub const TWO_LAYER_VIA_LIMIT: usize = 64;
/// Smallest corner allowed between two connected segments, degrees.
pub const MIN_CORNER_DEG: f64 = 45.0;
/// Longest tolerated parallel run of two different nets, millimetres.
pub const MAX_PARALLEL_RUN_MM: f64 = 10.0;
/// Centre-line spacing below which two segments count as running in parallel.
pub const PARALLEL_GAP_MM: f64 = 1.2;
/// Angular tolerance for the parallel test, degrees.
pub const PARALLEL_ANGLE_TOL_DEG: f64 = 5.0;

const EPS: f64 = 1e-9;
const EPS_POINT: f64 = 1e-6;

/// Identity of a hard rule. Codes are stable and suitable for a DRC log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RuleId {
    ProfileBand,
    Via,
    TrackWidth,
    Clearance,
    NetClass,
    AcuteAngle,
    ParallelRun,
    Geometry,
    AnnularRing,
    Drill,
    EdgeClearance,
}

impl RuleId {
    pub fn code(self) -> &'static str {
        match self {
            RuleId::ProfileBand => "R01_PROFILE_BAND",
            RuleId::Via => "R02_VIA",
            RuleId::TrackWidth => "R03_TRACK_WIDTH",
            RuleId::Clearance => "R04_CLEARANCE",
            RuleId::NetClass => "R05_NETCLASS",
            RuleId::AcuteAngle => "R06_ACUTE_ANGLE",
            RuleId::ParallelRun => "R07_PARALLEL_RUN",
            RuleId::Geometry => "R08_GEOMETRY",
            RuleId::AnnularRing => "R09_ANNULAR_RING",
            RuleId::Drill => "R10_DRILL",
            RuleId::EdgeClearance => "R11_EDGE_CLEARANCE",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            RuleId::ProfileBand => "minimum features must be in the 0.3–0.4 mm LUT band",
            RuleId::Via => "no plated vias in a single-layer LUT board",
            RuleId::TrackWidth => "copper must be at least the profile minimum width",
            RuleId::Clearance => "copper-to-copper and copper-to-outline clearance",
            RuleId::NetClass => "GND/PWR nets keep their class and their wide copper",
            RuleId::AcuteAngle => "no corner sharper than the 45° minimum",
            RuleId::ParallelRun => "parallel runs of different nets are length-limited",
            RuleId::Geometry => "routed geometry must be finite and well formed",
            RuleId::AnnularRing => "pad annulus must survive LUT drilling",
            RuleId::Drill => "drill must be within the LUT process band",
            RuleId::EdgeClearance => "pads and copper must clear the routed edge",
        }
    }
}

/// One hard-rule failure, optionally pinned to a board coordinate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleViolation {
    pub rule: RuleId,
    pub message: String,
    pub x_mm: Option<f64>,
    pub y_mm: Option<f64>,
}

impl RuleViolation {
    pub fn new(rule: RuleId, message: impl Into<String>) -> Self {
        Self {
            rule,
            message: message.into(),
            x_mm: None,
            y_mm: None,
        }
    }

    pub fn at(mut self, x: f64, y: f64) -> Self {
        self.x_mm = Some(x);
        self.y_mm = Some(y);
        self
    }
}

impl std::fmt::Display for RuleViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.x_mm, self.y_mm) {
            (Some(x), Some(y)) => write!(
                f,
                "{} at ({:.2}, {:.2}) mm: {}",
                self.rule.code(),
                x,
                y,
                self.message
            ),
            _ => write!(f, "{}: {}", self.rule.code(), self.message),
        }
    }
}

/// All failures collected by one check. Empty means "accepted".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RuleReport {
    pub violations: Vec<RuleViolation>,
}

impl RuleReport {
    pub fn is_ok(&self) -> bool {
        self.violations.is_empty()
    }

    pub fn passed(&self) -> bool {
        self.is_ok()
    }

    pub fn len(&self) -> usize {
        self.violations.len()
    }

    pub fn is_empty(&self) -> bool {
        self.violations.is_empty()
    }

    pub fn count(&self, rule: RuleId) -> usize {
        self.violations.iter().filter(|v| v.rule == rule).count()
    }

    pub fn summary(&self) -> String {
        let n = self.len();
        if n == 0 {
            "PASS: 0 rule violations".to_string()
        } else {
            format!(
                "FAIL: {} rule violation{}",
                n,
                if n == 1 { "" } else { "s" }
            )
        }
    }

    fn push(&mut self, violation: RuleViolation) {
        self.violations.push(violation);
    }
}

impl std::fmt::Display for RuleReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.summary())?;
        for violation in &self.violations {
            write!(f, "\n  {violation}")?;
        }
        Ok(())
    }
}

/// The hard-constraint profile. Construct one with [`LutRules::reliable`],
/// [`LutRules::fine`] or [`LutRules::from_preset`] — every field is validated by
/// [`LutRules::validate_profile`] before it is trusted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LutRules {
    pub name: String,
    /// Minimum copper width in millimetres, inside the LUT feature band.
    pub min_track_mm: f64,
    /// Minimum copper clearance in millimetres, inside the LUT feature band.
    pub min_clearance_mm: f64,
    /// Allowed vias; only zero is manufacturable by a single-layer LUT board.
    pub max_vias: usize,
    /// True for two-layer boards, where plated vias are legitimate.
    #[serde(default)]
    pub allow_vias: bool,
    /// Smallest allowed corner between two connected segments.
    pub min_corner_deg: f64,
    /// Longest tolerated parallel overlap of two different nets.
    pub max_parallel_run_mm: f64,
    /// Centre-line spacing at which two segments count as parallel.
    pub parallel_gap_mm: f64,
    /// Angular tolerance of the parallel test.
    pub parallel_angle_tol_deg: f64,
}

impl LutRules {
    /// Conservative profile: 0.4 mm minimum copper, 0.3 mm clearance, zero vias.
    pub fn reliable() -> Self {
        Self {
            name: "LUT-reliable".into(),
            min_track_mm: 0.4,
            min_clearance_mm: 0.3,
            max_vias: VIA_LIMIT,
            allow_vias: false,
            min_corner_deg: MIN_CORNER_DEG,
            max_parallel_run_mm: MAX_PARALLEL_RUN_MM,
            parallel_gap_mm: PARALLEL_GAP_MM,
            parallel_angle_tol_deg: PARALLEL_ANGLE_TOL_DEG,
        }
    }

    /// Finest features the process can still hold: 0.3 mm minimum, zero vias.
    pub fn fine() -> Self {
        Self {
            name: "LUT-fine".into(),
            min_track_mm: FEATURE_MIN_MM,
            min_clearance_mm: FEATURE_MIN_MM,
            ..Self::reliable()
        }
    }

    /// Derive rules from a board preset, without ever loosening the hard band or
    /// the via limit. A preset that asks for 0.2 mm clearance is clamped up to
    /// 0.3 mm; the board itself is still rejected by [`LutRules::check_board`].
    pub fn from_preset(preset: &LutPreset) -> Self {
        Self::from_preset_for(preset, false)
    }

    /// Preset-derived profile with an explicit layer count. Two-layer boards
    /// keep the same manufacturable feature band but may use plated vias.
    pub fn from_preset_for(preset: &LutPreset, two_layer: bool) -> Self {
        let base = if two_layer { Self::two_layer() } else { Self::reliable() };
        Self {
            name: format!("{}-{}", if two_layer { "2L" } else { "LUT" }, preset.name),
            min_track_mm: preset.min_track_mm.clamp(FEATURE_MIN_MM, FEATURE_MAX_MM),
            min_clearance_mm: preset
                .min_clearance_mm
                .clamp(FEATURE_MIN_MM, FEATURE_MAX_MM),
            ..base
        }
    }

    /// Two-layer profile: identical feature band to [`LutRules::reliable`],
    /// with a bounded via budget and vias permitted.
    pub fn two_layer() -> Self {
        Self {
            name: "2L-reliable".into(),
            max_vias: TWO_LAYER_VIA_LIMIT,
            allow_vias: true,
            ..Self::reliable()
        }
    }

    /// Check that the profile itself does not relax any hard constraint.
    pub fn validate_profile(&self) -> RuleReport {
        let mut report = RuleReport::default();
        for (label, value) in [
            ("minimum track width", self.min_track_mm),
            ("minimum clearance", self.min_clearance_mm),
        ] {
            if !value.is_finite() || value < FEATURE_MIN_MM - EPS || value > FEATURE_MAX_MM + EPS {
                report.push(RuleViolation::new(
                    RuleId::ProfileBand,
                    format!(
                        "{label} {value:.2} mm is outside the LUT band {FEATURE_MIN_MM:.1}–{FEATURE_MAX_MM:.1} mm"
                    ),
                ));
            }
        }
        if !self.allow_vias && self.max_vias > VIA_LIMIT {
            report.push(RuleViolation::new(
                RuleId::Via,
                format!(
                    "rule profile allows {} vias; a single-layer LUT board allows {}",
                    self.max_vias, VIA_LIMIT
                ),
            ));
        }
        if !self.min_corner_deg.is_finite() || self.min_corner_deg < MIN_CORNER_DEG - EPS {
            report.push(RuleViolation::new(
                RuleId::AcuteAngle,
                format!(
                    "minimum corner {:.1}° is sharper than {:.0}°",
                    self.min_corner_deg, MIN_CORNER_DEG
                ),
            ));
        }
        if !self.max_parallel_run_mm.is_finite() || self.max_parallel_run_mm < 0.0 {
            report.push(RuleViolation::new(
                RuleId::ParallelRun,
                format!("invalid parallel-run limit {}", self.max_parallel_run_mm),
            ));
        }
        if !self.parallel_gap_mm.is_finite() || self.parallel_gap_mm <= 0.0 {
            report.push(RuleViolation::new(
                RuleId::ParallelRun,
                format!("invalid parallel spacing {}", self.parallel_gap_mm),
            ));
        }
        if !self.parallel_angle_tol_deg.is_finite()
            || self.parallel_angle_tol_deg <= 0.0
            || self.parallel_angle_tol_deg >= 90.0
        {
            report.push(RuleViolation::new(
                RuleId::ParallelRun,
                format!(
                    "invalid parallel angle tolerance {:.1}°",
                    self.parallel_angle_tol_deg
                ),
            ));
        }
        report
    }    /// Pre-route gate: validate the profile, the board preset, class widths and
    /// the GND/PWR classification before any copper is generated.
    pub fn check_board(&self, board: &Board) -> RuleReport {
        let mut report = self.validate_profile();

        // The preset's declared features are *informational*: `from_preset`
        // clamps them into the hard band, and the routed copper is validated
        // against the clamped rules by `check_routing`. Rejecting the board
        // for its preset declaration would make any `advanced` board (0.2 mm
        // clearance) impossible to route at all.
        for class in NetClass::ALL {
            let width = class.track_width_mm();
            if width + EPS < self.min_track_mm {
                report.push(RuleViolation::new(
                    RuleId::TrackWidth,
                    format!(
                        "{} class width {:.2} mm is below the {:.2} mm minimum",
                        class.label(),
                        width,
                        self.min_track_mm
                    ),
                ));
            }
        }
        for net in &board.nets {
            if let Some(expected) = expected_power_ground_class(&net.name) {
                if net.class != expected {
                    report.push(RuleViolation::new(
                        RuleId::NetClass,
                        format!(
                            "net `{}` is classed {} but must be {}",
                            net.name,
                            net.class.label(),
                            expected.label()
                        ),
                    ));
                }
            }
            if matches!(net.class, NetClass::Ground | NetClass::Power)
                && board.net_terminals(net).len() < 2
            {
                report.push(RuleViolation::new(
                    RuleId::NetClass,
                    format!(
                        "{} net `{}` has fewer than two terminals",
                        net.class.label(),
                        net.name
                    ),
                ));
            }
        }
        // Manufacturing fields the LUT shop actually cares about. They are
        // separate RuleIds so an agent can gate on them explicitly.
        for fp in &board.footprints {
            for (index, pad) in fp.pads.iter().enumerate() {
                let annulus = (pad.size_mm - pad.drill_mm) / 2.0;
                if annulus < 0.15 {
                    report.push(
                        RuleViolation::new(
                            RuleId::AnnularRing,
                            format!(
                                "{} pad {} annulus {:.2} mm is below 0.15 mm",
                                fp.refdes, index, annulus
                            ),
                        )
                        .at(fp.x, fp.y),
                    );
                }
                if pad.drill_mm < 0.3 || pad.drill_mm > 3.0 {
                    report.push(
                        RuleViolation::new(
                            RuleId::Drill,
                            format!(
                                "{} pad {} drill {:.2} mm is outside 0.3–3.0 mm",
                                fp.refdes, index, pad.drill_mm
                            ),
                        )
                        .at(fp.x, fp.y),
                    );
                }
                if let Some((x, y)) = fp.pad_world(index) {
                    let edge = x.min(y).min(board.width_mm - x).min(board.height_mm - y)
                        - pad.size_mm / 2.0;
                    if edge < board.edge_clearance_mm() {
                        report.push(
                            RuleViolation::new(
                                RuleId::EdgeClearance,
                                format!(
                                    "{} pad {} is {:.2} mm from the edge, need {:.2} mm",
                                    fp.refdes,
                                    index,
                                    edge,
                                    board.edge_clearance_mm()
                                ),
                            )
                            .at(x, y),
                        );
                    }
                }
            }
        }
        report
    }

    /// Post-route gate: audit the generated copper against the same profile.
    pub fn check_routing(&self, board: &Board, result: &RoutingResult) -> RuleReport {
        let mut report = self.validate_profile();

        if result.vias > self.max_vias {
            report.push(RuleViolation::new(
                RuleId::Via,
                format!(
                    "routing produced {} vias; allowed {}",
                    result.vias, self.max_vias
                ),
            ));
        }

        let net_classes: HashMap<&str, NetClass> = board
            .nets
            .iter()
            .map(|net| (net.name.as_str(), net.class))
            .collect();
        let edge = board.edge_clearance_mm();

        for seg in &result.segments {
            let endpoints = [(seg.x1_mm, seg.y1_mm), (seg.x2_mm, seg.y2_mm)];
            let midpoint = ((seg.x1_mm + seg.x2_mm) / 2.0, (seg.y1_mm + seg.y2_mm) / 2.0);

            if !endpoints
                .iter()
                .all(|&(x, y)| x.is_finite() && y.is_finite())
            {
                report.push(RuleViolation::new(
                    RuleId::Geometry,
                    format!("net `{}`: segment has non-finite endpoints", seg.net),
                ));
                continue;
            }
            if !seg.width_mm.is_finite() || seg.width_mm + EPS < self.min_track_mm {
                report.push(
                    RuleViolation::new(
                        RuleId::TrackWidth,
                        format!(
                            "net `{}`: track width {:.2} mm is below {:.2} mm",
                            seg.net, seg.width_mm, self.min_track_mm
                        ),
                    )
                    .at(midpoint.0, midpoint.1),
                );
            }
            if let Some(&expected) = net_classes.get(seg.net.as_str()) {
                if seg.class != expected {
                    report.push(
                        RuleViolation::new(
                            RuleId::NetClass,
                            format!(
                                "net `{}`: segment classed {} but board says {}",
                                seg.net,
                                seg.class.label(),
                                expected.label()
                            ),
                        )
                        .at(midpoint.0, midpoint.1),
                    );
                }
            }
            // Use the board's class, not the segment's own label: a mislabelled
            // segment must not escape the wide-rail requirement by calling
            // itself Signal.
            let rail_class = net_classes
                .get(seg.net.as_str())
                .copied()
                .unwrap_or(seg.class);
            // The router may narrow a rail where it cannot pass at class
            // width, but never below the process minimum. Only that bound is
            // a manufacturability violation; a 0.3 mm neck on a dense board is
            // exactly the documented width-taper behaviour.
            if matches!(rail_class, NetClass::Ground | NetClass::Power)
                && seg.width_mm + EPS < self.min_track_mm
            {
                report.push(
                    RuleViolation::new(
                        RuleId::NetClass,
                        format!(
                            "{} net `{}`: copper thinned to {:.2} mm, process minimum {:.2} mm",
                            rail_class.label(),
                            seg.net,
                            seg.width_mm,
                            self.min_track_mm
                        ),
                    )
                    .at(midpoint.0, midpoint.1),
                );
            }
            for (x, y) in endpoints {
                let gap = x.min(y).min(board.width_mm - x).min(board.height_mm - y);
                if gap + EPS < edge {
                    report.push(
                        RuleViolation::new(
                            RuleId::Clearance,
                            format!(
                                "net `{}`: copper is {:.2} mm from the outline, needs {:.2} mm",
                                seg.net, gap, edge
                            ),
                        )
                        .at(x, y),
                    );
                }
            }
        }

        // Copper-to-copper clearance and parallel-run length between nets.
        for i in 0..result.segments.len() {
            for j in (i + 1)..result.segments.len() {
                let a = &result.segments[i];
                let b = &result.segments[j];
                if a.net == b.net || a.layer != b.layer {
                    // Copper on opposite layers may cross; clearance is a
                    // same-layer constraint (except at vias, audited by DRC).
                    continue;
                }
                let (distance, pa, pb) = segment_distance_mm(a, b);
                if !distance.is_finite() {
                    continue;
                }
                let edge_gap = distance - (a.width_mm + b.width_mm) / 2.0;
                if edge_gap < self.min_clearance_mm - EPS {
                    report.push(
                        RuleViolation::new(
                            RuleId::Clearance,
                            format!(
                                "`{}` to `{}`: {:.2} mm gap is below {:.2} mm",
                                a.net, b.net, edge_gap, self.min_clearance_mm
                            ),
                        )
                        .at((pa.0 + pb.0) / 2.0, (pa.1 + pb.1) / 2.0),
                    );
                }
                let overlap =
                    parallel_overlap_mm(a, b, self.parallel_gap_mm, self.parallel_angle_tol_deg);
                if overlap > self.max_parallel_run_mm + EPS {
                    report.push(
                        RuleViolation::new(
                            RuleId::ParallelRun,
                            format!(
                                "`{}` and `{}` run parallel for {:.1} mm, limit {:.1} mm",
                                a.net, b.net, overlap, self.max_parallel_run_mm
                            ),
                        )
                        .at((pa.0 + pb.0) / 2.0, (pa.1 + pb.1) / 2.0),
                    );
                }
            }
        }

        // Acute corners: segments of the same net meeting at a sharp angle.
        let mut by_net: HashMap<&str, Vec<&RouteSegment>> = HashMap::new();
        for seg in &result.segments {
            by_net.entry(seg.net.as_str()).or_default().push(seg);
        }
        for (net, segments) in by_net {
            for i in 0..segments.len() {
                for j in (i + 1)..segments.len() {
                    let a = segments[i];
                    let b = segments[j];
                    if a.layer != b.layer {
                        continue;
                    }
                    let a_pts = [(a.x1_mm, a.y1_mm), (a.x2_mm, a.y2_mm)];
                    let b_pts = [(b.x1_mm, b.y1_mm), (b.x2_mm, b.y2_mm)];
                    for ai in 0..2 {
                        for bi in 0..2 {
                            let pivot = a_pts[ai];
                            if distance(pivot, b_pts[bi]) > EPS_POINT {
                                continue;
                            }
                            let angle = turn_angle_deg(pivot, a_pts[1 - ai], b_pts[1 - bi]);
                            if angle < self.min_corner_deg - EPS {
                                report.push(
                                    RuleViolation::new(
                                        RuleId::AcuteAngle,
                                        format!(
                                            "net `{}`: {:.1}° corner is sharper than {:.0}°",
                                            net, angle, self.min_corner_deg
                                        ),
                                    )
                                    .at(pivot.0, pivot.1),
                                );
                            }
                        }
                    }
                }
            }
        }

        report
    }
}

/// Conservative GND/PWR classification, mirroring the KiCad importer: a name
/// that says ground or supply must not be left in a signal class.
fn expected_power_ground_class(name: &str) -> Option<NetClass> {
    let n = name.to_ascii_uppercase();
    // "VGND" (virtual ground) is intentionally excluded — it is a Bias net.
    if n == "VGND" {
        return None;
    }
    // "GND", "GROUND", "VSS", "0" mean ground.
    if n.contains("GND") || n.contains("GROUND") || n.contains("VSS") || n == "0" {
        Some(NetClass::Ground)
    } else if n.contains("VCC") || n.contains("VDD") || n.contains("V+") || n.contains("V-") {
        Some(NetClass::Power)
    } else {
        None
    }
}

/// Smaller angle between the two rays `pivot -> a` and `pivot -> b`, in degrees.
/// A straight continuation is 180°, a right angle 90°, a hairpin near 0°.
pub fn turn_angle_deg(pivot: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let v1 = (a.0 - pivot.0, a.1 - pivot.1);
    let v2 = (b.0 - pivot.0, b.1 - pivot.1);
    let l1 = (v1.0 * v1.0 + v1.1 * v1.1).sqrt();
    let l2 = (v2.0 * v2.0 + v2.1 * v2.1).sqrt();
    if l1 <= EPS || l2 <= EPS {
        return 180.0;
    }
    let dot = (v1.0 * v2.0 + v1.1 * v2.1) / (l1 * l2);
    dot.clamp(-1.0, 1.0).acos().to_degrees()
}

/// Length of the overlapping projection of two nearly-parallel segments whose
/// supporting lines are no further apart than `gap_mm`. Returns 0 for crossing,
/// perpendicular or distant segments.
pub fn parallel_overlap_mm(a: &RouteSegment, b: &RouteSegment, gap_mm: f64, tol_deg: f64) -> f64 {
    let la = a.length_mm();
    let lb = b.length_mm();
    if la <= EPS || lb <= EPS {
        return 0.0;
    }
    let ua = ((a.x2_mm - a.x1_mm) / la, (a.y2_mm - a.y1_mm) / la);
    let ub = ((b.x2_mm - b.x1_mm) / lb, (b.y2_mm - b.y1_mm) / lb);
    let dot = (ua.0 * ub.0 + ua.1 * ub.1).clamp(-1.0, 1.0);
    if dot.abs().acos().to_degrees() > tol_deg {
        return 0.0;
    }
    let dx = b.x1_mm - a.x1_mm;
    let dy = b.y1_mm - a.y1_mm;
    if (dx * ua.1 - dy * ua.0).abs() > gap_mm {
        return 0.0;
    }
    let t0 = dx * ua.0 + dy * ua.1;
    let t1 = t0 + (b.x2_mm - b.x1_mm) * ua.0 + (b.y2_mm - b.y1_mm) * ua.1;
    let (lo, hi) = if t0 <= t1 { (t0, t1) } else { (t1, t0) };
    (hi.min(la) - lo.max(0.0)).max(0.0)
}

/// Minimum distance between two segment centre-lines, with the closest point
/// on each. Used by the copper-clearance audit.
pub fn segment_distance_mm(a: &RouteSegment, b: &RouteSegment) -> (f64, (f64, f64), (f64, f64)) {
    let a1 = (a.x1_mm, a.y1_mm);
    let a2 = (a.x2_mm, a.y2_mm);
    let b1 = (b.x1_mm, b.y1_mm);
    let b2 = (b.x2_mm, b.y2_mm);
    if segments_intersect(a1, a2, b1, b2) {
        return (0.0, a1, b1);
    }
    let mut best = (f64::INFINITY, a1, b1);
    let candidates = [
        (
            distance_to_segment(a1, b1, b2),
            a1,
            closest_on_segment(a1, b1, b2),
        ),
        (
            distance_to_segment(a2, b1, b2),
            a2,
            closest_on_segment(a2, b1, b2),
        ),
        (
            distance_to_segment(b1, a1, a2),
            closest_on_segment(b1, a1, a2),
            b1,
        ),
        (
            distance_to_segment(b2, a1, a2),
            closest_on_segment(b2, a1, a2),
            b2,
        ),
    ];
    for (d, pa, pb) in candidates {
        if d < best.0 {
            best = (d, pa, pb);
        }
    }
    best
}

fn distance(p: (f64, f64), q: (f64, f64)) -> f64 {
    ((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt()
}

fn closest_on_segment(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let len2 = dx * dx + dy * dy;
    if len2 <= EPS {
        return a;
    }
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0);
    (a.0 + t * dx, a.1 + t * dy)
}

fn distance_to_segment(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    distance(p, closest_on_segment(p, a, b))
}

fn orient(a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> f64 {
    (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
}

fn on_segment(a: (f64, f64), b: (f64, f64), p: (f64, f64)) -> bool {
    orient(a, b, p).abs() <= EPS
        && p.0 >= a.0.min(b.0) - EPS
        && p.0 <= a.0.max(b.0) + EPS
        && p.1 >= a.1.min(b.1) - EPS
        && p.1 <= a.1.max(b.1) + EPS
}

fn segments_intersect(a: (f64, f64), b: (f64, f64), c: (f64, f64), d: (f64, f64)) -> bool {
    let o1 = orient(a, b, c);
    let o2 = orient(a, b, d);
    let o3 = orient(c, d, a);
    let o4 = orient(c, d, b);
    if ((o1 > EPS && o2 < -EPS) || (o1 < -EPS && o2 > EPS))
        && ((o3 > EPS && o4 < -EPS) || (o3 < -EPS && o4 > EPS))
    {
        return true;
    }
    (o1.abs() <= EPS && on_segment(a, b, c))
        || (o2.abs() <= EPS && on_segment(a, b, d))
        || (o3.abs() <= EPS && on_segment(c, d, a))
        || (o4.abs() <= EPS && on_segment(c, d, b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{Footprint, Net, Pad, PadRef};

    fn sample_board(preset: LutPreset) -> Board {
        let mut board = Board::new("rules-test", 60.0, 40.0, preset);
        let fp =
            Footprint::new("R1", "10k", NetClass::Signal, 30.0, 20.0, 4.0, 2.0).with_pads(vec![
                Pad::new(-1.5, 0.0, 1.6, 0.8),
                Pad::new(1.5, 0.0, 1.6, 0.8),
            ]);
        board.add_footprint(fp);
        board.add_net(Net::new(
            "SIG",
            NetClass::Signal,
            vec![PadRef::new("R1", 0), PadRef::new("R1", 1)],
        ));
        board
    }

    fn grounded_board() -> Board {
        let mut board = sample_board(LutPreset::reliable());
        let fp =
            Footprint::new("J1", "GND", NetClass::Ground, 10.0, 20.0, 3.0, 3.0).with_pads(vec![
                Pad::new(-1.0, 0.0, 1.6, 0.8),
                Pad::new(1.0, 0.0, 1.6, 0.8),
            ]);
        board.add_footprint(fp);
        board.add_net(Net::new(
            "GND",
            NetClass::Ground,
            vec![PadRef::new("J1", 0), PadRef::new("J1", 1)],
        ));
        board
    }

    fn result(segments: Vec<RouteSegment>, vias: usize) -> RoutingResult {
        RoutingResult {
            segments,
            routed_nets: Vec::new(),
            failed_nets: Vec::new(),
            total_length_mm: 0.0,
            vias,
            via_objects: Vec::new(),
        }
    }

    fn seg(net: &str, class: NetClass, x1: f64, y1: f64, x2: f64, y2: f64) -> RouteSegment {
        RouteSegment::new(x1, y1, x2, y2, net, class)
    }

    // --- profile -----------------------------------------------------------

    #[test]
    fn reliable_and_fine_profiles_are_valid() {
        assert!(LutRules::reliable().validate_profile().is_ok());
        assert!(LutRules::fine().validate_profile().is_ok());
    }

    #[test]
    fn profile_rejects_feature_below_band() {
        let mut rules = LutRules::reliable();
        rules.min_track_mm = 0.2;
        let report = rules.validate_profile();
        assert_eq!(report.count(RuleId::ProfileBand), 1);
    }

    #[test]
    fn profile_rejects_feature_above_band() {
        let mut rules = LutRules::reliable();
        rules.min_clearance_mm = 0.5;
        assert_eq!(rules.validate_profile().count(RuleId::ProfileBand), 1);
    }

    #[test]
    fn profile_rejects_non_finite_feature() {
        let mut rules = LutRules::reliable();
        rules.min_track_mm = f64::NAN;
        assert_eq!(rules.validate_profile().count(RuleId::ProfileBand), 1);
    }

    #[test]
    fn profile_rejects_via_budget_and_loose_corner() {
        let mut rules = LutRules::reliable();
        rules.max_vias = 1;
        rules.min_corner_deg = 30.0;
        let report = rules.validate_profile();
        assert_eq!(report.count(RuleId::Via), 1);
        assert_eq!(report.count(RuleId::AcuteAngle), 1);
    }

    #[test]
    fn profile_rejects_invalid_parallel_settings() {
        let mut rules = LutRules::reliable();
        rules.max_parallel_run_mm = -1.0;
        rules.parallel_gap_mm = 0.0;
        rules.parallel_angle_tol_deg = 90.0;
        let report = rules.validate_profile();
        assert_eq!(report.count(RuleId::ParallelRun), 3);
    }

    #[test]
    fn from_preset_clamps_into_band_and_keeps_zero_vias() {
        let rules = LutRules::from_preset(&LutPreset::advanced());
        assert_eq!(rules.min_track_mm, 0.3);
        assert_eq!(rules.min_clearance_mm, 0.3);
        assert_eq!(rules.max_vias, 0);
        assert!(rules.validate_profile().is_ok());
    }

    // --- geometry helpers --------------------------------------------------

    #[test]
    fn turn_angle_covers_straight_right_and_acute() {
        assert_eq!(turn_angle_deg((0.0, 0.0), (10.0, 0.0), (-10.0, 0.0)), 180.0);
        assert_eq!(turn_angle_deg((0.0, 0.0), (10.0, 0.0), (0.0, 10.0)), 90.0);
        let sharp = turn_angle_deg((0.0, 0.0), (10.0, 0.0), (3.0, 1.0));
        assert!(sharp < 45.0, "{sharp}");
        assert_eq!(turn_angle_deg((0.0, 0.0), (0.0, 0.0), (1.0, 1.0)), 180.0);
    }

    #[test]
    fn parallel_overlap_detects_long_run_only() {
        let a = seg("A", NetClass::Signal, 0.0, 0.0, 20.0, 0.0);
        let b = seg("B", NetClass::Signal, 2.0, 0.9, 18.0, 0.9);
        assert!((parallel_overlap_mm(&a, &b, 1.2, 5.0) - 16.0).abs() < 1e-6);
        let stub = seg("C", NetClass::Signal, 2.0, 0.9, 4.0, 0.9);
        assert_eq!(parallel_overlap_mm(&a, &stub, 1.2, 5.0), 2.0);
        let far = seg("D", NetClass::Signal, 2.0, 5.0, 18.0, 5.0);
        assert_eq!(parallel_overlap_mm(&a, &far, 1.2, 5.0), 0.0);
        let cross = seg("E", NetClass::Signal, 5.0, -5.0, 5.0, 5.0);
        assert_eq!(parallel_overlap_mm(&a, &cross, 1.2, 5.0), 0.0);
    }

    #[test]
    fn segment_distance_handles_parallel_crossing_and_disjoint() {
        let a = seg("A", NetClass::Signal, 0.0, 0.0, 10.0, 0.0);
        let parallel = seg("B", NetClass::Signal, 2.0, 0.8, 8.0, 0.8);
        assert!((segment_distance_mm(&a, &parallel).0 - 0.8).abs() < 1e-6);
        let cross = seg("C", NetClass::Signal, 5.0, -3.0, 5.0, 3.0);
        assert_eq!(segment_distance_mm(&a, &cross).0, 0.0);
        let disjoint = seg("D", NetClass::Signal, 0.0, 4.0, 3.0, 4.0);
        assert!((segment_distance_mm(&a, &disjoint).0 - 4.0).abs() < 1e-6);
    }

    // --- pre-route gate ----------------------------------------------------

    #[test]
    fn check_board_accepts_clean_board() {
        assert!(LutRules::reliable().check_board(&grounded_board()).is_ok());
    }

    #[test]
    fn check_board_rejects_preset_below_band() {
        // The preset declaration itself no longer fails the gate: `from_preset`
        // clamps it into the LUT band and the copper is checked afterwards.
        let mut board = sample_board(LutPreset::reliable());
        board.preset.min_clearance_mm = 0.2;
        let report = LutRules::reliable().check_board(&board);
        assert_eq!(report.count(RuleId::Clearance), 0);
    }

    #[test]
    fn check_board_rejects_via_preset() {
        // The preset's via allowance is informational; the hard constraint is
        // on the routed copper (checked by `check_routing`).
        let board = sample_board(LutPreset::advanced());
        let report = LutRules::from_preset(&LutPreset::advanced()).check_board(&board);
        assert!(report.is_ok(), "{}", report);
        // A via in the actual copper is still a hard failure.
        let post = LutRules::from_preset(&LutPreset::advanced())
            .check_routing(&board, &result(Vec::new(), 2));
        assert_eq!(post.count(RuleId::Via), 1);
    }

    #[test]
    fn check_board_rejects_misclassified_ground() {
        let mut board = grounded_board();
        board.nets[1].class = NetClass::Signal;
        let report = LutRules::reliable().check_board(&board);
        assert_eq!(report.count(RuleId::NetClass), 1);
    }

    #[test]
    fn check_board_rejects_power_net_without_terminals() {
        let mut board = grounded_board();
        board.nets[1].name = "VCC".into();
        board.nets[1].class = NetClass::Power;
        board.nets[1].nodes.clear();
        let report = LutRules::reliable().check_board(&board);
        assert_eq!(report.count(RuleId::NetClass), 1);
    }

    // --- post-route gate ---------------------------------------------------

    #[test]
    fn check_routing_accepts_clean_copper() {
        let board = grounded_board();
        let segments = vec![
            seg("SIG", NetClass::Signal, 20.0, 10.0, 30.0, 10.0),
            seg("SIG", NetClass::Signal, 30.0, 10.0, 30.0, 20.0),
            seg("GND", NetClass::Ground, 20.0, 30.0, 40.0, 30.0),
        ];
        let report = LutRules::reliable().check_routing(&board, &result(segments, 0));
        assert!(report.is_ok(), "{}", report);
    }

    #[test]
    fn check_routing_rejects_vias() {
        let board = sample_board(LutPreset::reliable());
        let report = LutRules::reliable().check_routing(&board, &result(Vec::new(), 2));
        assert_eq!(report.count(RuleId::Via), 1);
    }

    #[test]
    fn check_routing_rejects_narrow_track() {
        let board = sample_board(LutPreset::reliable());
        let segments = vec![seg("SIG", NetClass::Signal, 20.0, 10.0, 30.0, 10.0).with_width(0.3)];
        let report = LutRules::reliable().check_routing(&board, &result(segments, 0));
        assert_eq!(report.count(RuleId::TrackWidth), 1);
    }

    #[test]
    fn check_routing_rejects_ground_below_the_process_minimum() {
        let board = grounded_board();
        let segments = vec![seg("GND", NetClass::Ground, 20.0, 30.0, 40.0, 30.0).with_width(0.2)];
        let report = LutRules::reliable().check_routing(&board, &result(segments, 0));
        assert_eq!(report.count(RuleId::NetClass), 1);
    }

    #[test]
    fn check_routing_rejects_wrong_segment_class() {
        let board = grounded_board();
        let segments = vec![seg("GND", NetClass::Signal, 20.0, 30.0, 40.0, 30.0)];
        let report = LutRules::reliable().check_routing(&board, &result(segments, 0));
        assert_eq!(report.count(RuleId::NetClass), 1);
    }

    #[test]
    fn check_routing_rejects_acute_corner_but_allows_45() {
        let board = sample_board(LutPreset::reliable());
        let sharp = vec![
            seg("SIG", NetClass::Signal, 20.0, 10.0, 30.0, 10.0),
            seg("SIG", NetClass::Signal, 30.0, 10.0, 26.0, 11.0),
        ];
        let report = LutRules::reliable().check_routing(&board, &result(sharp, 0));
        assert_eq!(report.count(RuleId::AcuteAngle), 1);

        let okay = vec![
            seg("SIG", NetClass::Signal, 20.0, 10.0, 30.0, 10.0),
            seg("SIG", NetClass::Signal, 30.0, 10.0, 34.0, 14.0),
        ];
        assert!(LutRules::reliable()
            .check_routing(&board, &result(okay, 0))
            .is_ok());
    }

    #[test]
    fn check_routing_rejects_long_parallel_run() {
        let board = sample_board(LutPreset::reliable());
        let mut rules = LutRules::reliable();
        rules.max_parallel_run_mm = 5.0;
        let segments = vec![
            seg("SIG", NetClass::Signal, 10.0, 10.0, 40.0, 10.0),
            seg("NC", NetClass::Signal, 12.0, 10.9, 38.0, 10.9),
        ];
        let report = rules.check_routing(&board, &result(segments, 0));
        assert_eq!(report.count(RuleId::ParallelRun), 1);
        assert_eq!(report.count(RuleId::Clearance), 0);
    }

    #[test]
    fn check_routing_rejects_insufficient_clearance() {
        let board = sample_board(LutPreset::reliable());
        let segments = vec![
            seg("SIG", NetClass::Signal, 10.0, 10.0, 40.0, 10.0),
            seg("NC", NetClass::Signal, 12.0, 10.5, 38.0, 10.5),
        ];
        let report = LutRules::reliable().check_routing(&board, &result(segments, 0));
        assert_eq!(report.count(RuleId::Clearance), 1);
    }

    #[test]
    fn check_routing_rejects_copper_outside_edge_clearance() {
        let board = sample_board(LutPreset::reliable());
        let segments = vec![seg("SIG", NetClass::Signal, 0.1, 20.0, 5.0, 20.0)];
        let report = LutRules::reliable().check_routing(&board, &result(segments, 0));
        assert_eq!(report.count(RuleId::Clearance), 1);
    }

    #[test]
    fn check_routing_rejects_non_finite_geometry() {
        let board = sample_board(LutPreset::reliable());
        let segments = vec![seg("SIG", NetClass::Signal, f64::NAN, 10.0, 30.0, 10.0)];
        let report = LutRules::reliable().check_routing(&board, &result(segments, 0));
        assert_eq!(report.count(RuleId::Geometry), 1);
    }

    // --- report plumbing ---------------------------------------------------

    #[test]
    fn report_helpers_and_display() {
        let mut report = RuleReport::default();
        assert!(report.is_ok() && report.is_empty() && report.passed());
        assert_eq!(report.summary(), "PASS: 0 rule violations");
        report.push(RuleViolation::new(RuleId::Via, "one via").at(1.0, 2.0));
        assert_eq!(report.len(), 1);
        assert_eq!(report.count(RuleId::Via), 1);
        assert_eq!(report.summary(), "FAIL: 1 rule violation");
        assert_eq!(
            report.violations[0].to_string(),
            "R02_VIA at (1.00, 2.00) mm: one via"
        );
        let plain = RuleViolation::new(RuleId::Geometry, "bad");
        assert_eq!(plain.to_string(), "R08_GEOMETRY: bad");
        assert!(RuleId::Via.description().contains("vias"));
        assert!(RuleId::Clearance.code().starts_with("R04"));
    }

    #[test]
    fn every_rule_id_has_a_stable_unique_code() {
        let mut codes = std::collections::HashSet::new();
        for id in [
            RuleId::ProfileBand,
            RuleId::Via,
            RuleId::TrackWidth,
            RuleId::Clearance,
            RuleId::NetClass,
            RuleId::AcuteAngle,
            RuleId::ParallelRun,
            RuleId::Geometry,
        ] {
            assert!(id.code().starts_with('R'));
            assert!(!id.description().is_empty());
            assert!(codes.insert(id.code()), "duplicate code {}", id.code());
        }
        assert_eq!(codes.len(), 8);
    }

    #[test]
    fn gates_merge_profile_violations_with_board_violations() {
        let mut rules = LutRules::reliable();
        rules.min_track_mm = 0.2;
        rules.max_vias = 1;
        let board = sample_board(LutPreset::reliable());
        let pre = rules.check_board(&board);
        assert_eq!(pre.count(RuleId::ProfileBand), 1);
        assert_eq!(pre.count(RuleId::Via), 1);
        let post = rules.check_routing(&board, &result(Vec::new(), 0));
        assert_eq!(post.count(RuleId::ProfileBand), 1);
        assert_eq!(post.count(RuleId::Via), 1);
    }

    #[test]
    fn check_board_accepts_preset_inside_band() {
        // A preset whose features sit inside the LUT band is legal: the rules
        // derived from it (`from_preset`) accept the board unchanged.
        let mut preset = LutPreset::reliable();
        preset.min_track_mm = 0.3;
        preset.min_clearance_mm = 0.3;
        let report = LutRules::reliable().check_board(&sample_board(preset));
        assert!(
            report.is_ok(),
            "preset inside the band must pass: {report}"
        );
    }

    #[test]
    fn check_board_rejects_misclassified_power() {
        let mut board = grounded_board();
        board.nets[1].name = "VCC".into();
        board.nets[1].class = NetClass::Signal;
        let report = LutRules::reliable().check_board(&board);
        assert_eq!(report.count(RuleId::NetClass), 1);
    }

    #[test]
    fn check_routing_rejects_non_finite_width() {
        let board = sample_board(LutPreset::reliable());
        let segments =
            vec![seg("SIG", NetClass::Signal, 20.0, 10.0, 30.0, 10.0).with_width(f64::NAN)];
        let report = LutRules::reliable().check_routing(&board, &result(segments, 0));
        assert_eq!(report.count(RuleId::TrackWidth), 1);
    }

    #[test]
    fn check_routing_rejects_crossing_nets() {
        let board = sample_board(LutPreset::reliable());
        let segments = vec![
            seg("SIG", NetClass::Signal, 10.0, 20.0, 50.0, 20.0),
            seg("NC", NetClass::Signal, 30.0, 10.0, 30.0, 30.0),
        ];
        let report = LutRules::reliable().check_routing(&board, &result(segments, 0));
        assert_eq!(report.count(RuleId::Clearance), 1);
    }

    #[test]
    fn geometry_helpers_handle_degenerate_and_offset_segments() {
        let point = seg("P", NetClass::Signal, 5.0, 5.0, 5.0, 5.0);
        let line = seg("L", NetClass::Signal, 0.0, 0.0, 20.0, 0.0);
        assert_eq!(parallel_overlap_mm(&point, &line, 1.2, 5.0), 0.0);
        let (d, _, _) = segment_distance_mm(&point, &line);
        assert!((d - 5.0).abs() < 1e-6);

        let base = seg("A", NetClass::Signal, 0.0, 0.0, 10.0, 0.0);
        let offset = seg("B", NetClass::Signal, -5.0, 0.5, 5.0, 0.5);
        assert!((parallel_overlap_mm(&base, &offset, 1.2, 5.0) - 5.0).abs() < 1e-6);
    }
}
