//! Auto-placement for audio amplifier PCBs (ТЗ 3.1).
//!
//! Places components along the signal chain: input (left) → gain stages (center) → output (right).
//! Power rail runs along the top edge, ground star-point along the bottom.
//! Enforces input/output/power isolation per ТЗ 5.2.
//!
//! Placement carries rotation: power, output and ground footprints are placed
//! rotated 180°, facing the edge they run along ([`Footprint::with_rotation`]),
//! and the placed board can be exported with [`PlacementResult::write_kicad_pcb`].

use crate::board::{Board, Footprint, LutPreset, Net, NetClass, Pad, PadRef};

/// Result of auto-placement: a fully populated board.
pub struct PlacementResult {
    pub board: Board,
}

impl PlacementResult {
    /// Opt-in placement optimization for a user board. Tries a 180° flip of
    /// each THT footprint and keeps it only when the validation issue count
    /// improves. Never called implicitly by the pipeline.
    pub fn optimize(board: &Board) -> Board {
        let mut best = board.clone();
        let mut best_issues = best.validate().len();
        for index in 0..best.footprints.len() {
            let mut candidate = best.clone();
            let fp = &mut candidate.footprints[index];
            fp.rotation_deg = if fp.rotation_deg.abs() < 1e-9 {
                180.0
            } else {
                0.0
            };
            for pad in &mut fp.pads {
                pad.x = fp.width_mm - pad.x;
                pad.y = fp.height_mm - pad.y;
            }
            let issues = candidate.validate().len();
            if issues < best_issues {
                best = candidate;
                best_issues = issues;
            }
        }
        best
    }

    /// Create a complete amplifier board with auto-placed components.
    ///
    /// Signal flow: **JIN** (left) → **C_IN** → **Q1/Q2** (LTP) → **Q3/Q4** (VAS) →
    /// **Q5/Q6** (output) → **JOUT** (right).
    ///
    /// Power: VCC rail and decoupling caps along the top edge; the output
    /// stage sits in the last strip of the board. Ground: star point on the
    /// bottom edge (ТЗ 5.1). Power, output and ground footprints are placed
    /// rotated 180°, facing the edge.
    pub fn auto_amplifier() -> Self {
        let (bw, bh) = (200.0f64, 150.0f64);
        let mut board = Board::new("AmpRoute Studio Demo", bw, bh, LutPreset::reliable());

        let mut add_fp = |refdes: &str,
                          value: &str,
                          class: NetClass,
                          x: f64,
                          y: f64,
                          w: f64,
                          h: f64,
                          pads: Vec<Pad>| {
            let mut fp = Footprint::new(refdes, value, class, x, y, w, h);
            fp.pads = pads;
            board.add_footprint(fp);
        };

        // === Input stage (far left, isolated from power/output) ===
        add_fp(
            "JIN",
            "RCA-IN",
            NetClass::Input,
            15.0,
            bh * 0.62,
            12.0,
            10.0,
            vec![
                Pad::new(3.0, 5.0, 1.8, 0.9), // tip
                Pad::new(9.0, 5.0, 1.8, 0.9), // sleeve
            ],
        );
        add_fp(
            "C_IN",
            "10uF",
            NetClass::Signal,
            35.0,
            bh * 0.62,
            7.0,
            6.0,
            vec![Pad::new(2.5, 3.0, 1.5, 0.8), Pad::new(4.5, 3.0, 1.5, 0.8)],
        );

        // === Differential pair (LTP) ===
        add_fp(
            "Q1",
            "K310",
            NetClass::Signal,
            55.0,
            bh * 0.55,
            8.0,
            7.0,
            vec![
                Pad::new(2.0, 2.0, 1.4, 0.7), // B
                Pad::new(6.0, 2.0, 1.4, 0.7), // C
                Pad::new(4.0, 5.0, 1.4, 0.7), // E
            ],
        );
        add_fp(
            "Q2",
            "K310",
            NetClass::Signal,
            55.0,
            bh * 0.72,
            8.0,
            7.0,
            vec![
                Pad::new(2.0, 2.0, 1.4, 0.7),
                Pad::new(6.0, 2.0, 1.4, 0.7),
                Pad::new(4.0, 5.0, 1.4, 0.7),
            ],
        );
        add_fp(
            "R_CS",
            "10k",
            NetClass::Bias,
            70.0,
            bh * 0.82,
            7.0,
            5.0,
            vec![Pad::new(2.0, 2.5, 1.4, 0.8), Pad::new(5.0, 2.5, 1.4, 0.8)],
        );

        // === VAS (voltage amplification stage) ===
        add_fp(
            "Q3",
            "K310",
            NetClass::Signal,
            85.0,
            bh * 0.55,
            8.0,
            7.0,
            vec![
                Pad::new(2.0, 2.0, 1.4, 0.7),
                Pad::new(6.0, 2.0, 1.4, 0.7),
                Pad::new(4.0, 5.0, 1.4, 0.7),
            ],
        );
        add_fp(
            "Q4",
            "K310",
            NetClass::Signal,
            85.0,
            bh * 0.72,
            8.0,
            7.0,
            vec![
                Pad::new(2.0, 2.0, 1.4, 0.7),
                Pad::new(6.0, 2.0, 1.4, 0.7),
                Pad::new(4.0, 5.0, 1.4, 0.7),
            ],
        );

        // === Feedback network (close to output stage) ===
        add_fp(
            "R_FB",
            "47k",
            NetClass::Feedback,
            100.0,
            bh * 0.48,
            7.0,
            5.0,
            vec![Pad::new(2.0, 2.5, 1.4, 0.8), Pad::new(5.0, 2.5, 1.4, 0.8)],
        );
        add_fp(
            "C_FB",
            "100pF",
            NetClass::Feedback,
            100.0,
            bh * 0.35,
            6.0,
            5.0,
            vec![Pad::new(2.0, 2.5, 1.4, 0.8), Pad::new(4.0, 2.5, 1.4, 0.8)],
        );

        // === Output stage (far right, keeps distance from input) ===
        add_fp(
            "Q5",
            "IRF540",
            NetClass::Output,
            175.0,
            bh * 0.55,
            10.0,
            8.0,
            vec![
                Pad::new(2.5, 2.5, 1.6, 0.9), // G
                Pad::new(7.5, 2.5, 1.6, 0.9), // D
                Pad::new(5.0, 5.5, 1.6, 0.9), // S
            ],
        );
        add_fp(
            "Q6",
            "IRF9540",
            NetClass::Output,
            175.0,
            bh * 0.72,
            10.0,
            8.0,
            vec![
                Pad::new(2.5, 2.5, 1.6, 0.9),
                Pad::new(7.5, 2.5, 1.6, 0.9),
                Pad::new(5.0, 5.5, 1.6, 0.9),
            ],
        );
        add_fp(
            "JOUT",
            "RCA-OUT",
            NetClass::Output,
            174.0,
            bh * 0.62,
            12.0,
            10.0,
            vec![Pad::new(3.0, 5.0, 1.8, 0.9), Pad::new(9.0, 5.0, 1.8, 0.9)],
        );

        // === Power rail (top edge) ===
        add_fp(
            "VCC",
            "+V",
            NetClass::Power,
            bw * 0.5,
            bh - 10.0,
            12.0,
            8.0,
            vec![Pad::new(4.0, 4.0, 2.0, 1.0), Pad::new(8.0, 4.0, 2.0, 1.0)],
        );
        add_fp(
            "C_P1",
            "100uF",
            NetClass::Bias,
            40.0,
            bh - 10.0,
            8.0,
            7.0,
            vec![Pad::new(2.5, 3.5, 1.6, 0.9), Pad::new(5.5, 3.5, 1.6, 0.9)],
        );
        add_fp(
            "C_P2",
            "100uF",
            NetClass::Bias,
            bw - 48.0,
            bh - 10.0,
            8.0,
            7.0,
            vec![Pad::new(2.5, 3.5, 1.6, 0.9), Pad::new(5.5, 3.5, 1.6, 0.9)],
        );

        // === Ground star (bottom edge, ТЗ 5.1) ===
        add_fp(
            "GND",
            "GND-STAR",
            NetClass::Ground,
            bw * 0.5 - 6.0,
            12.0,
            12.0,
            8.0,
            vec![Pad::new(4.0, 4.0, 2.0, 1.0), Pad::new(8.0, 4.0, 2.0, 1.0)],
        );

        // Placement with rotation: power, output, bias and ground parts carry
        // a 180° rotation so they face the edge they run along.
        for fp in board.footprints.iter_mut() {
            if matches!(
                fp.class,
                NetClass::Power | NetClass::Output | NetClass::Ground | NetClass::Bias
            ) {
                fp.rotation_deg = 180.0;
                // The pads were authored at their final world positions. Mirror
                // the local offsets so rotating the footprint keeps them there.
                for pad in &mut fp.pads {
                    pad.x = fp.width_mm - pad.x;
                    pad.y = fp.height_mm - pad.y;
                }
            }
        }

        // === Define nets (signal chain) ===
        let mut add_net = |name: &str, class: NetClass, nodes: Vec<(&str, usize)>| {
            board.add_net(Net::new(
                name,
                class,
                nodes.into_iter().map(|(r, p)| PadRef::new(r, p)).collect(),
            ));
        };

        // Input → C_IN → Q1 base
        add_net("VIN", NetClass::Input, vec![("JIN", 0), ("C_IN", 0)]);
        add_net("VIN_Q1", NetClass::Input, vec![("C_IN", 1), ("Q1", 0)]);

        // LTP emitters → current source
        add_net(
            "E_Q1_Q2",
            NetClass::Signal,
            vec![("Q1", 2), ("Q2", 2), ("R_CS", 0)],
        );

        // VAS input from Q1 collector
        add_net("C_Q1_Q3", NetClass::Signal, vec![("Q1", 1), ("Q3", 0)]);

        // Output stage
        add_net("OUT_Q5", NetClass::Output, vec![("Q3", 1), ("Q5", 0)]);
        add_net(
            "VOUT",
            NetClass::Output,
            vec![("Q5", 1), ("JOUT", 0), ("R_FB", 0), ("C_FB", 0)],
        );

        // Feedback divider: output → R_FB/C_FB → inverting input (Q2 base).
        add_net(
            "FB",
            NetClass::Feedback,
            vec![("R_FB", 1), ("C_FB", 1), ("Q2", 0)],
        );

        // Power
        add_net(
            "VCC",
            NetClass::Power,
            vec![("VCC", 0), ("C_P1", 0), ("C_P2", 0), ("Q5", 2)],
        );
        add_net(
            "GND",
            NetClass::Ground,
            vec![
                ("GND", 0),
                ("C_P1", 1),
                ("C_P2", 1),
                ("JIN", 1),
                ("Q6", 2),
                ("JOUT", 1),
                ("GND", 1),
            ],
        );
        Self { board }
    }

    /// Write the placed board to `path` as a `.kicad_pcb` file — footprints
    /// with their placed (rotated) positions and nets defined; no routed
    /// copper yet (the routing stage adds that later).
    pub fn write_kicad_pcb(&self, path: &str) -> anyhow::Result<()> {
        let empty = crate::router::RoutingResult::new();
        let fab = crate::kicad::FabOptions::default();
        crate::kicad::write_kicad_pcb(&self.board, &empty, &fab, path)
    }

    /// Component placement checks: overlapping or too-close footprints.
    ///
    /// This is a placement sanity check only. It does not verify copper
    /// clearance, connectivity or manufacturing rules — see [`Board::validate`]
    /// for model integrity and the router for copper conflicts.
    pub fn drc_report(&self, min_gap_mm: f64) -> Vec<String> {
        let mut issues = Vec::new();
        let fps = &self.board.footprints;
        for i in 0..fps.len() {
            for j in (i + 1)..fps.len() {
                let a = &fps[i];
                let b = &fps[j];
                let (ax0, ay0, ax1, ay1) = a.rect();
                let (bx0, by0, bx1, by1) = b.rect();

                let gx = (ax1.min(bx1) - ax0.max(bx0)).max(0.0);
                let gy = (ay1.min(by1) - ay0.max(by0)).max(0.0);
                if gx > 0.0 && gy > 0.0 {
                    issues.push(format!("Overlap: {} ↔ {}", a.refdes, b.refdes));
                    continue;
                }

                // Axis-aligned gap on each axis, then the true Euclidean distance
                // between the rectangles (diagonal gaps are not additive).
                let dx = if ax1 < bx0 {
                    bx0 - ax1
                } else {
                    (ax0 - bx1).max(0.0)
                };
                let dy = if ay1 < by0 {
                    by0 - ay1
                } else {
                    (ay0 - by1).max(0.0)
                };
                let dist = (dx * dx + dy * dy).sqrt();
                if dist < min_gap_mm {
                    issues.push(format!(
                        "Clearance {:.2} mm < {:.2} mm: {} ↔ {}",
                        dist, min_gap_mm, a.refdes, b.refdes
                    ));
                }
            }
        }
        issues
    }
}

/// Legacy alias for backward compatibility.
pub fn drc_report(board: &Board, min_gap_mm: f64) -> Vec<String> {
    PlacementResult {
        board: board.clone(),
    }
    .drc_report(min_gap_mm)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_amplifier_places_components() {
        let pr = PlacementResult::auto_amplifier();
        assert!(pr.board.footprints.len() >= 15);
        assert!(pr.board.nets.len() >= 8);
        assert_eq!(pr.board.width_mm, 200.0);
        assert_eq!(pr.board.height_mm, 150.0);
    }

    #[test]
    fn drc_detects_no_overlaps_in_auto_placement() {
        let pr = PlacementResult::auto_amplifier();
        let issues = pr.drc_report(0.5);
        assert_eq!(
            issues.len(),
            0,
            "Auto-placement should be DRC-clean: {issues:?}"
        );
    }

    #[test]
    fn drc_detects_overlap_and_clearance_violations() {
        let mut board = Board::new("drc", 60.0, 40.0, LutPreset::reliable());
        board.add_footprint(Footprint::new(
            "A",
            "a",
            NetClass::Signal,
            10.0,
            10.0,
            10.0,
            10.0,
        ));
        // Overlapping footprint.
        board.add_footprint(Footprint::new(
            "B",
            "b",
            NetClass::Signal,
            15.0,
            15.0,
            10.0,
            10.0,
        ));
        // Diagonal neighbour: 1 mm gap on each axis → 1.41 mm true distance.
        board.add_footprint(Footprint::new(
            "C",
            "c",
            NetClass::Signal,
            21.0,
            21.0,
            4.0,
            4.0,
        ));
        let pr = PlacementResult { board };

        let issues = pr.drc_report(0.5);
        assert!(
            issues.iter().any(|i| i.contains("Overlap: A ↔ B")),
            "{issues:?}"
        );
        let diagonal = pr.drc_report(1.5);
        assert!(
            diagonal.iter().any(|i| i.contains("B ↔ C")),
            "diagonal gap of 1.41 mm should violate 1.5 mm: {diagonal:?}"
        );
    }

    #[test]
    fn auto_placement_is_structurally_valid() {
        let pr = PlacementResult::auto_amplifier();
        let issues = pr.board.validate();
        assert!(issues.is_empty(), "demo board must validate: {issues:?}");
    }

    #[test]
    fn input_is_isolated_from_output() {
        let pr = PlacementResult::auto_amplifier();
        let jin = pr.board.footprint("JIN").expect("JIN exists");
        let jout = pr.board.footprint("JOUT").expect("JOUT exists");
        let (_ax0, ay0, ax1, ay1) = jin.rect();
        let (bx0, by0, _bx1, by1) = jout.rect();
        let dx = bx0 - ax1;
        let dy = (ay0 + ay1) / 2.0 - (by0 + by1) / 2.0;
        assert!(
            dx > 100.0,
            "Input/output separation should be >100mm, got {}",
            dx
        );
        assert!(dy.abs() < 20.0, "Should be roughly on same horizontal line");
    }

    #[test]
    fn power_and_thermal_parts_are_at_board_edges() {
        let pr = PlacementResult::auto_amplifier();
        let b = &pr.board;
        let power: Vec<&Footprint> = b
            .footprints
            .iter()
            .filter(|f| matches!(f.class, NetClass::Power | NetClass::Ground))
            .collect();
        assert!(!power.is_empty());
        for f in &power {
            let (x0, y0, x1, y1) = f.rect();
            let dist = [x0, y0, b.width_mm - x1, b.height_mm - y1]
                .into_iter()
                .fold(f64::MAX, f64::min);
            assert!(dist <= 15.0, "{} not at the board edge: {dist} mm", f.refdes);
        }
        // Thermal hotspots: the output stage sits in the last strip of the board.
        assert!(b.thermal_hotspots().len() >= 3);
        for (x, y, _r) in b.thermal_hotspots() {
            let dist = [x, y, b.width_mm - x, b.height_mm - y]
                .into_iter()
                .fold(f64::MAX, f64::min);
            assert!(
                dist <= 30.0,
                "hotspot ({x:.1},{y:.1}) too far from an edge: {dist:.1} mm"
            );
        }
    }

    #[test]
    fn power_parts_carry_rotation() {
        let pr = PlacementResult::auto_amplifier();
        for refdes in ["VCC", "C_P1", "C_P2", "GND"] {
            let f = pr.board.footprint(refdes).expect("footprint exists");
            assert!(
                f.rotation_deg.abs() > 1.0,
                "{refdes} should be placed rotated"
            );
        }
    }

    #[test]
    fn write_kicad_pcb_writes_placed_footprints() {
        let pr = PlacementResult::auto_amplifier();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("placed.kicad_pcb");
        let path = path.to_string_lossy().to_string();
        pr.write_kicad_pcb(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("(kicad_pcb"));
        for fp in &pr.board.footprints {
            assert!(text.contains(&fp.refdes), "missing refdes {}", fp.refdes);
        }
    }
}
