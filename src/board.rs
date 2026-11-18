//! Board data model: footprints, pads, nets and the LUT manufacturing preset.

use serde::{Deserialize, Serialize};

/// Electrical class of a net. Drives track width, routing priority and
/// audio-specific proximity penalties.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetClass {
    Ground,
    Power,
    Input,
    Feedback,
    Output,
    Signal,
    Bias,
    Control,
}

impl NetClass {
    pub const ALL: [NetClass; 8] = [
        NetClass::Ground,
        NetClass::Power,
        NetClass::Input,
        NetClass::Feedback,
        NetClass::Output,
        NetClass::Signal,
        NetClass::Bias,
        NetClass::Control,
    ];

    /// Lower value routes first.
    pub fn priority(self) -> u8 {
        match self {
            NetClass::Ground => 0,
            NetClass::Power => 1,
            NetClass::Input => 2,
            NetClass::Feedback => 3,
            NetClass::Output => 4,
            NetClass::Signal => 5,
            NetClass::Bias => 6,
            NetClass::Control => 7,
        }
    }

    /// Copper width in millimetres.
    ///
    /// Kept within what a laser-iron transfer can actually hold: a 0.8 mm rail
    /// carries the current of a small power amp comfortably, and a thinner one
    /// still closes where a 1 mm track would not fit between two pads.
    pub fn track_width_mm(self) -> f64 {
        match self {
            NetClass::Power | NetClass::Output => 0.8,
            NetClass::Ground => 0.6,
            NetClass::Signal | NetClass::Input | NetClass::Feedback => 0.4,
            NetClass::Bias | NetClass::Control => 0.4,
        }
    }

    /// Extra cost applied when a track of this class runs next to `other`.
    /// Input must stay away from power and output (ТЗ 5.2).
    pub fn proximity_penalty(self, other: NetClass) -> f64 {
        use NetClass::*;
        match (self, other) {
            (Input, Output) | (Input, Power) | (Output, Input) | (Power, Input) => 10.0,
            (Signal, Power) | (Power, Signal) => 5.0,
            (Feedback, Output) | (Feedback, Power) => 5.0,
            _ => 1.0,
        }
    }

    /// Routing cost weight; low-impedance nets are cheap, high-impedance
    /// sensitive nets are expensive so they stay short and direct.
    pub fn cost_weight(self) -> f64 {
        match self {
            NetClass::Ground => 0.5,
            NetClass::Power => 0.7,
            NetClass::Input => 1.0,
            NetClass::Feedback => 1.2,
            NetClass::Output => 1.5,
            NetClass::Signal => 2.0,
            NetClass::Bias => 2.6,
            NetClass::Control => 3.0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            NetClass::Ground => "Ground",
            NetClass::Power => "Power",
            NetClass::Input => "Input",
            NetClass::Feedback => "Feedback",
            NetClass::Output => "Output",
            NetClass::Signal => "Signal",
            NetClass::Bias => "Bias",
            NetClass::Control => "Control",
        }
    }

    /// RGB used by the SVG/PDF writers and the GUI.
    pub fn rgb(self) -> (u8, u8, u8) {
        match self {
            NetClass::Ground => (60, 200, 110),
            NetClass::Power => (235, 90, 80),
            NetClass::Input => (240, 200, 70),
            NetClass::Feedback => (190, 120, 235),
            NetClass::Output => (90, 165, 245),
            NetClass::Signal => (240, 240, 240),
            NetClass::Bias => (235, 150, 95),
            NetClass::Control => (140, 200, 200),
        }
    }
}

/// Manufacturing constraints of the LUT (toner-transfer) process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LutPreset {
    pub name: String,
    pub min_track_mm: f64,
    pub min_clearance_mm: f64,
    pub min_pad_mm: f64,
    pub max_vias: usize,
    pub grid_mm: f64,
}

impl LutPreset {
    /// Safe for hobby ironing: thick tracks, no vias.
    pub fn reliable() -> Self {
        Self {
            name: "Reliable".into(),
            min_track_mm: 0.40,
            min_clearance_mm: 0.30,
            min_pad_mm: 1.00,
            max_vias: 0,
            grid_mm: 0.2,
        }
    }

    /// Tighter rules for a well-calibrated iron and glossy paper.
    pub fn advanced() -> Self {
        Self {
            name: "Advanced".into(),
            min_track_mm: 0.30,
            min_clearance_mm: 0.20,
            min_pad_mm: 0.80,
            max_vias: 10,
            grid_mm: 0.2,
        }
    }

    pub fn by_name(name: &str) -> Self {
        match name.to_ascii_lowercase().as_str() {
            "advanced" => Self::advanced(),
            _ => Self::reliable(),
        }
    }
}

/// A single through-hole pad, positioned relative to its footprint origin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pad {
    pub x: f64,
    pub y: f64,
    pub size_mm: f64,
    pub drill_mm: f64,
}

impl Pad {
    pub fn new(x: f64, y: f64, size_mm: f64, drill_mm: f64) -> Self {
        Self {
            x,
            y,
            size_mm,
            drill_mm,
        }
    }
}

/// A placed component.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Footprint {
    pub refdes: String,
    pub value: String,
    pub class: NetClass,
    pub x: f64,
    pub y: f64,
    pub width_mm: f64,
    pub height_mm: f64,
    pub pads: Vec<Pad>,
}

impl Footprint {
    pub fn new(refdes: &str, value: &str, class: NetClass, x: f64, y: f64, w: f64, h: f64) -> Self {
        Self {
            refdes: refdes.into(),
            value: value.into(),
            class,
            x,
            y,
            width_mm: w,
            height_mm: h,
            pads: vec![Pad::new(w / 2.0, h / 2.0, 1.8, 0.9)],
        }
    }

    pub fn with_pads(mut self, pads: Vec<Pad>) -> Self {
        self.pads = pads;
        self
    }

    /// World position of pad `index`.
    pub fn pad_world(&self, index: usize) -> Option<(f64, f64)> {
        self.pads.get(index).map(|p| (self.x + p.x, self.y + p.y))
    }

    pub fn center(&self) -> (f64, f64) {
        (self.x + self.width_mm / 2.0, self.y + self.height_mm / 2.0)
    }

    /// Axis-aligned bounding box as `(x0, y0, x1, y1)`.
    pub fn rect(&self) -> (f64, f64, f64, f64) {
        (
            self.x,
            self.y,
            self.x + self.width_mm,
            self.y + self.height_mm,
        )
    }

    pub fn contains(&self, px: f64, py: f64) -> bool {
        let (x0, y0, x1, y1) = self.rect();
        px >= x0 && px <= x1 && py >= y0 && py <= y1
    }
}

/// Reference to one pad of a footprint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PadRef {
    pub refdes: String,
    pub pad: usize,
}

impl PadRef {
    pub fn new(refdes: &str, pad: usize) -> Self {
        Self {
            refdes: refdes.into(),
            pad,
        }
    }
}

/// An electrical net.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Net {
    pub name: String,
    pub class: NetClass,
    pub nodes: Vec<PadRef>,
}

impl Net {
    pub fn new(name: &str, class: NetClass, nodes: Vec<PadRef>) -> Self {
        Self {
            name: name.into(),
            class,
            nodes,
        }
    }
}

/// A non-plated mounting hole in the board outline.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct MountingHole {
    pub x: f64,
    pub y: f64,
    /// Drill diameter in millimetres.
    pub drill_mm: f64,
}

impl MountingHole {
    pub fn new(x: f64, y: f64, drill_mm: f64) -> Self {
        Self { x, y, drill_mm }
    }

    /// Radius around the hole where copper must not go, in millimetres.
    ///
    /// Half the drill plus a full clearance band, so a track of any width the
    /// process allows still keeps its distance from the hole wall.
    pub fn copper_keepout_mm(&self) -> f64 {
        self.drill_mm / 2.0 + 1.2
    }
}

/// The whole board: outline, manufacturing preset, components and nets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Board {
    pub name: String,
    pub width_mm: f64,
    pub height_mm: f64,
    pub preset: LutPreset,
    pub footprints: Vec<Footprint>,
    pub nets: Vec<Net>,
    /// Plated-through mechanical holes. Copper keeps its distance.
    #[serde(default)]
    pub mounting_holes: Vec<MountingHole>,
    /// One-line description shown in the board picker.
    #[serde(default)]
    pub description: String,
}

impl Board {
    pub fn new(name: &str, width_mm: f64, height_mm: f64, preset: LutPreset) -> Self {
        Self {
            name: name.into(),
            width_mm,
            height_mm,
            preset,
            footprints: Vec::new(),
            nets: Vec::new(),
            mounting_holes: Vec::new(),
            description: String::new(),
        }
    }

    pub fn add_footprint(&mut self, fp: Footprint) {
        self.footprints.push(fp);
    }

    pub fn add_mounting_hole(&mut self, hole: MountingHole) {
        self.mounting_holes.push(hole);
    }

    /// Mounting holes in the four corners, `inset_mm` from each edge.
    pub fn add_corner_holes(&mut self, inset_mm: f64, drill_mm: f64) {
        let (w, h) = (self.width_mm, self.height_mm);
        for (x, y) in [
            (inset_mm, inset_mm),
            (w - inset_mm, inset_mm),
            (inset_mm, h - inset_mm),
            (w - inset_mm, h - inset_mm),
        ] {
            self.add_mounting_hole(MountingHole::new(x, y, drill_mm));
        }
    }

    pub fn add_net(&mut self, net: Net) {
        self.nets.push(net);
    }

    pub fn footprint(&self, refdes: &str) -> Option<&Footprint> {
        self.footprints.iter().find(|f| f.refdes == refdes)
    }

    /// World coordinates of every pad belonging to `net`.
    pub fn net_terminals(&self, net: &Net) -> Vec<(f64, f64)> {
        let mut out = Vec::new();
        for node in &net.nodes {
            if let Some(fp) = self.footprint(&node.refdes) {
                if let Some(pos) = fp.pad_world(node.pad) {
                    out.push(pos);
                }
            }
        }
        out
    }

    /// `(position, size, owning net name)` for every pad on the board.
    pub fn all_pads(&self) -> Vec<((f64, f64), f64, Option<String>)> {
        let mut out = Vec::new();
        for fp in &self.footprints {
            for (i, pad) in fp.pads.iter().enumerate() {
                let owner = self
                    .nets
                    .iter()
                    .find(|n| n.nodes.iter().any(|r| r.refdes == fp.refdes && r.pad == i))
                    .map(|n| n.name.clone());
                out.push(((fp.x + pad.x, fp.y + pad.y), pad.size_mm, owner));
            }
        }
        out
    }

    /// Pads that dissipate power — used as thermal penalty sources (ТЗ 5.3).
    ///
    /// The radius is a soft hint: enough to nudge copper away from a hot part,
    /// small enough that a dense single-layer board still closes.
    pub fn thermal_hotspots(&self) -> Vec<(f64, f64, f64)> {
        self.footprints
            .iter()
            .filter(|f| matches!(f.class, NetClass::Power | NetClass::Output))
            .map(|f| {
                let (cx, cy) = f.center();
                (cx, cy, (f.width_mm.max(f.height_mm) * 0.5).clamp(3.0, 6.0))
            })
            .collect()
    }

    /// Copper-to-outline clearance for this process, in millimetres.
    pub fn edge_clearance_mm(&self) -> f64 {
        // The LUT process needs the same gap to the routed edge as between two
        // tracks, with a floor so the board outline is never in doubt.
        (self.preset.min_clearance_mm + 0.2).max(0.3)
    }

    /// Star-ground centre: gravity point of all ground pads (ТЗ 5.1).
    pub fn star_ground_point(&self) -> Option<(f64, f64)> {
        let gnd: Vec<(f64, f64)> = self
            .nets
            .iter()
            .filter(|n| n.class == NetClass::Ground)
            .flat_map(|n| self.net_terminals(n))
            .collect();
        if gnd.is_empty() {
            return None;
        }
        let n = gnd.len() as f64;
        Some((
            gnd.iter().map(|p| p.0).sum::<f64>() / n,
            gnd.iter().map(|p| p.1).sum::<f64>() / n,
        ))
    }

    /// Number of routed-relevant nets (two or more pads).
    pub fn routable_nets(&self) -> usize {
        self.nets
            .iter()
            .filter(|n| self.net_terminals(n).len() >= 2)
            .count()
    }

    /// Structural validation: geometry, pad references and net integrity.
    ///
    /// This is *not* a manufacturing DRC. It catches broken models — pads wired
    /// to two different nets (a short), nets pointing at missing pads, geometry
    /// outside the board, and dimensions below the LUT process minimum.
    pub fn validate(&self) -> Vec<String> {
        let mut issues = Vec::new();

        if !(self.width_mm.is_finite() && self.height_mm.is_finite())
            || self.width_mm <= 0.0
            || self.height_mm <= 0.0
        {
            issues.push("Board size must be positive and finite".to_string());
            return issues;
        }

        let mut seen_refdes: std::collections::HashMap<&str, usize> =
            std::collections::HashMap::new();
        for fp in &self.footprints {
            *seen_refdes.entry(fp.refdes.as_str()).or_insert(0) += 1;
            if !(fp.x.is_finite() && fp.y.is_finite()) {
                issues.push(format!("{}: position is not finite", fp.refdes));
                continue;
            }
            let (x0, y0, x1, y1) = fp.rect();
            if x0 < 0.0 || y0 < 0.0 || x1 > self.width_mm || y1 > self.height_mm {
                issues.push(format!(
                    "{}: footprint extends outside the board outline",
                    fp.refdes
                ));
            }
            if fp.pads.is_empty() {
                issues.push(format!("{}: footprint has no pads", fp.refdes));
            }
            for (i, pad) in fp.pads.iter().enumerate() {
                if pad.size_mm < self.preset.min_pad_mm {
                    issues.push(format!(
                        "{}.{}: pad {:.2} mm is below LUT minimum {:.2} mm",
                        fp.refdes, i, pad.size_mm, self.preset.min_pad_mm
                    ));
                }
                if pad.drill_mm <= 0.0 || pad.drill_mm >= pad.size_mm {
                    issues.push(format!(
                        "{}.{}: drill {:.2} mm must be > 0 and smaller than the pad",
                        fp.refdes, i, pad.drill_mm
                    ));
                }
            }
        }
        for (refdes, count) in seen_refdes {
            if count > 1 {
                issues.push(format!(
                    "{refdes}: duplicate reference designator ({count} footprints)"
                ));
            }
        }

        // Mounting holes must sit on the board, clear of the outline.
        for (i, hole) in self.mounting_holes.iter().enumerate() {
            if !(hole.x.is_finite() && hole.y.is_finite()) {
                issues.push(format!("H{}: mounting hole position is not finite", i + 1));
                continue;
            }
            let r = hole.drill_mm / 2.0;
            if hole.x - r < 0.0
                || hole.y - r < 0.0
                || hole.x + r > self.width_mm
                || hole.y + r > self.height_mm
            {
                issues.push(format!(
                    "H{}: mounting hole at ({:.1}, {:.1}) mm is too close to the board edge",
                    i + 1,
                    hole.x,
                    hole.y
                ));
            }
            if hole.drill_mm <= 0.0 {
                issues.push(format!("H{}: drill must be positive", i + 1));
            }
        }

        // Copper too close to the outline fails a real DRC, so catch it here.
        let edge_clearance = self.edge_clearance_mm();
        for fp in &self.footprints {
            for (i, pad) in fp.pads.iter().enumerate() {
                let (px, py) = (fp.x + pad.x, fp.y + pad.y);
                let r = pad.size_mm / 2.0;
                let gap = (px - r)
                    .min(py - r)
                    .min(self.width_mm - px - r)
                    .min(self.height_mm - py - r);
                if gap < edge_clearance - 1e-9 {
                    issues.push(format!(
                        "{}.{}: pad is {:.2} mm from the board edge, needs {:.2} mm",
                        fp.refdes, i, gap, edge_clearance
                    ));
                }
            }
        }

        // A pad may belong to at most one net.
        let mut pad_owner: std::collections::HashMap<(&str, usize), &str> =
            std::collections::HashMap::new();
        for net in &self.nets {
            for node in &net.nodes {
                match self.footprint(&node.refdes) {
                    None => issues.push(format!(
                        "{}: net references unknown footprint {}",
                        net.name, node.refdes
                    )),
                    Some(fp) if node.pad >= fp.pads.len() => issues.push(format!(
                        "{}: net references pad {} of {} which has {} pads",
                        net.name,
                        node.pad,
                        node.refdes,
                        fp.pads.len()
                    )),
                    Some(_) => {
                        if let Some(prev) =
                            pad_owner.insert((node.refdes.as_str(), node.pad), &net.name)
                        {
                            issues.push(format!(
                                "{}.{}: pad assigned to two nets ({prev} and {}) — short circuit",
                                node.refdes, node.pad, net.name
                            ));
                        }
                    }
                }
            }
            if net.nodes.len() < 2 {
                issues.push(format!(
                    "{}: net has fewer than two nodes and cannot be routed",
                    net.name
                ));
            }
        }

        for class in NetClass::ALL {
            let w = class.track_width_mm();
            if w < self.preset.min_track_mm {
                issues.push(format!(
                    "{}: track width {:.2} mm is below LUT minimum {:.2} mm",
                    class.label(),
                    w,
                    self.preset.min_track_mm
                ));
            }
        }

        issues
    }

    /// Deserialize a board from JSON.
    pub fn from_json(data: &str) -> anyhow::Result<Self> {
        Ok(serde_json::from_str(data)?)
    }

    /// Serialize the board to pretty JSON.
    pub fn to_json(&self) -> anyhow::Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_board() -> Board {
        let mut b = Board::new("tiny", 50.0, 40.0, LutPreset::reliable());
        b.add_footprint(Footprint::new(
            "R1",
            "10k",
            NetClass::Signal,
            10.0,
            10.0,
            6.0,
            4.0,
        ));
        b.add_footprint(Footprint::new(
            "R2",
            "10k",
            NetClass::Signal,
            30.0,
            10.0,
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
    fn terminals_resolve_to_pad_centres() {
        let b = tiny_board();
        let t = b.net_terminals(&b.nets[0]);
        assert_eq!(t.len(), 2);
        assert!((t[0].0 - 13.0).abs() < 1e-9);
        assert!((t[0].1 - 12.0).abs() < 1e-9);
    }

    #[test]
    fn footprint_contains_and_rect() {
        let fp = Footprint::new("U1", "opamp", NetClass::Signal, 5.0, 6.0, 10.0, 8.0);
        assert_eq!(fp.rect(), (5.0, 6.0, 15.0, 14.0));
        assert!(fp.contains(10.0, 10.0));
        assert!(!fp.contains(0.0, 0.0));
    }

    #[test]
    fn star_ground_is_centroid_of_ground_pads() {
        let mut b = Board::new("g", 100.0, 100.0, LutPreset::reliable());
        b.add_footprint(Footprint::new(
            "G1",
            "gnd",
            NetClass::Ground,
            0.0,
            0.0,
            2.0,
            2.0,
        ));
        b.add_footprint(Footprint::new(
            "G2",
            "gnd",
            NetClass::Ground,
            18.0,
            0.0,
            2.0,
            2.0,
        ));
        b.add_net(Net::new(
            "GND",
            NetClass::Ground,
            vec![PadRef::new("G1", 0), PadRef::new("G2", 0)],
        ));
        let (x, _y) = b.star_ground_point().expect("has ground");
        assert!((x - 10.0).abs() < 1e-9);
    }

    #[test]
    fn class_priorities_are_ordered() {
        assert!(NetClass::Ground.priority() < NetClass::Power.priority());
        assert!(NetClass::Power.priority() < NetClass::Input.priority());
        assert!(NetClass::Input.priority() < NetClass::Control.priority());
    }

    #[test]
    fn input_rejects_power_and_output() {
        assert!(NetClass::Input.proximity_penalty(NetClass::Power) > 1.0);
        assert!(NetClass::Input.proximity_penalty(NetClass::Output) > 1.0);
        assert_eq!(NetClass::Input.proximity_penalty(NetClass::Signal), 1.0);
    }

    #[test]
    fn preset_lookup_is_case_insensitive() {
        assert_eq!(LutPreset::by_name("ADVANCED").name, "Advanced");
        assert_eq!(LutPreset::by_name("whatever").name, "Reliable");
    }

    #[test]
    fn validate_accepts_a_consistent_board() {
        let b = tiny_board();
        assert!(
            b.validate().is_empty(),
            "expected no issues, got {:?}",
            b.validate()
        );
    }

    #[test]
    fn validate_rejects_passive_shared_between_nets() {
        let mut b = tiny_board();
        b.add_net(Net::new(
            "N2",
            NetClass::Power,
            vec![PadRef::new("R1", 0), PadRef::new("R2", 0)],
        ));
        let issues = b.validate();
        assert!(
            issues.iter().any(|i| i.contains("two nets")),
            "expected a short-circuit report, got {issues:?}"
        );
    }

    #[test]
    fn validate_rejects_dangling_pad_reference() {
        let mut b = tiny_board();
        b.add_net(Net::new(
            "BAD",
            NetClass::Signal,
            vec![PadRef::new("R1", 0), PadRef::new("R9", 0)],
        ));
        let issues = b.validate();
        assert!(
            issues.iter().any(|i| i.contains("unknown footprint R9")),
            "got {issues:?}"
        );
    }

    #[test]
    fn validate_rejects_footprint_outside_board() {
        let mut b = tiny_board();
        b.footprints.push(Footprint::new(
            "R3",
            "10k",
            NetClass::Signal,
            48.0,
            10.0,
            6.0,
            4.0,
        ));
        let issues = b.validate();
        assert!(
            issues.iter().any(|i| i.contains("outside the board")),
            "got {issues:?}"
        );
    }

    #[test]
    fn validate_rejects_track_narrower_than_preset() {
        let mut b = tiny_board();
        b.preset = LutPreset::advanced();
        b.preset.min_track_mm = 0.5;
        let issues = b.validate();
        assert!(
            issues.iter().any(|i| i.contains("below LUT minimum")),
            "got {issues:?}"
        );
    }
}
