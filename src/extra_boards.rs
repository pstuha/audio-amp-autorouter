//! Ten experimental amplifier boards for routing stress tests.
//!
//! They are deliberately kept out of `BOARD_SPECS`: the five shipped boards
//! are the acceptance baseline, while these probes exercise new topologies,
//! sizes and pad densities without changing that baseline.

use crate::board::{Board, Footprint, LutPreset, Net, NetClass, Pad, PadRef};

pub struct ExtraSpec {
    pub id: &'static str,
    pub summary: &'static str,
    pub build: fn() -> Board,
}

pub const EXTRA_BOARD_SPECS: &[ExtraSpec] = &[
    ExtraSpec { id: "pam8403-mini", summary: "Class-D DIP-8 mini — 50 × 35 mm", build: pam8403_mini },
    ExtraSpec { id: "lm358-preamp", summary: "Dual op-amp preamp — 55 × 40 mm", build: lm358_preamp },
    ExtraSpec { id: "tda2822-bridge", summary: "BTL bridge amp — 60 × 40 mm", build: tda2822_bridge },
    ExtraSpec { id: "ne555-tone", summary: "Tone generator + buffer — 55 × 40 mm", build: ne555_tone },
    ExtraSpec { id: "classa-single", summary: "Single-ended class-A stage — 70 × 45 mm", build: classa_single },
    ExtraSpec { id: "jfet-buffer", summary: "Two-transistor source follower — 60 × 40 mm", build: jfet_buffer },
    ExtraSpec { id: "discrete-opamp", summary: "Discrete differential pair — 75 × 50 mm", build: discrete_opamp },
    ExtraSpec { id: "lm1875-mono", summary: "TO-220-5 power block — 80 × 55 mm", build: lm1875_mono },
    ExtraSpec { id: "ta2024-tripath", summary: "Class-T with output filter — 90 × 60 mm", build: ta2024_tripath },
    ExtraSpec { id: "hybrid-output", summary: "Hybrid driver + output stage — 100 × 65 mm", build: hybrid_output },
];

pub fn build(id: &str) -> Option<Board> {
    EXTRA_BOARD_SPECS
        .iter()
        .find(|s| s.id.eq_ignore_ascii_case(id))
        .map(|s| (s.build)())
}

fn board(name: &str, w: f64, h: f64) -> Board {
    let mut b = Board::new(name, w, h, LutPreset::reliable());
    b.description = format!("Experimental probe — {w:.0} × {h:.0} mm");
    b.add_corner_holes(5.0, 2.5);
    b
}

fn axial(r: &str, v: &str, class: NetClass, x: f64, y: f64) -> Footprint {
    Footprint::new(r, v, class, x, y, 12.0, 6.0).with_pads(vec![
        Pad::new(1.0, 3.0, 1.8, 0.9),
        Pad::new(11.0, 3.0, 1.8, 0.9),
    ])
}

fn radial(r: &str, v: &str, class: NetClass, x: f64, y: f64) -> Footprint {
    Footprint::new(r, v, class, x, y, 9.0, 6.0).with_pads(vec![
        Pad::new(2.0, 3.0, 1.8, 0.9),
        Pad::new(7.0, 3.0, 1.8, 0.9),
    ])
}

fn to92(r: &str, v: &str, class: NetClass, x: f64, y: f64) -> Footprint {
    Footprint::new(r, v, class, x, y, 9.0, 7.0).with_pads(vec![
        Pad::new(1.3, 3.5, 1.6, 0.8),
        Pad::new(3.84, 3.5, 1.6, 0.8),
        Pad::new(6.38, 3.5, 1.6, 0.8),
    ])
}

fn to220(r: &str, v: &str, class: NetClass, x: f64, y: f64) -> Footprint {
    let pads = (0..3)
        .map(|i| Pad::new(2.0 + i as f64 * 2.54, 8.0, 1.8, 1.0))
        .collect();
    Footprint::new(r, v, class, x, y, 14.0, 12.0).with_pads(pads)
}

fn dip8(r: &str, v: &str, class: NetClass, x: f64, y: f64) -> Footprint {
    let mut pads = Vec::new();
    for i in 0..4 {
        pads.push(Pad::new(2.0 + i as f64 * 2.54, 2.0, 1.6, 0.8));
    }
    for i in 0..4 {
        pads.push(Pad::new(2.0 + (3 - i) as f64 * 2.54, 9.62, 1.6, 0.8));
    }
    Footprint::new(r, v, class, x, y, 14.0, 14.0).with_pads(pads)
}

fn to220_5(r: &str, v: &str, class: NetClass, x: f64, y: f64) -> Footprint {
    let pads = (0..5)
        .map(|i| Pad::new(2.0 + i as f64 * 2.54, 8.0, 1.8, 1.0))
        .collect();
    Footprint::new(r, v, class, x, y, 16.0, 12.0).with_pads(pads)
}

fn rca(r: &str, x: f64, y: f64) -> Footprint {
    Footprint::new(r, "RCA", NetClass::Input, x, y, 13.0, 9.0).with_pads(vec![
        Pad::new(2.5, 4.5, 2.0, 1.0),
        Pad::new(7.58, 4.5, 2.0, 1.0),
    ])
}

fn speaker(r: &str, x: f64, y: f64) -> Footprint {
    Footprint::new(r, "SPK", NetClass::Output, x, y, 13.0, 8.0).with_pads(vec![
        Pad::new(2.0, 4.0, 2.2, 1.1),
        Pad::new(7.08, 4.0, 2.2, 1.1),
    ])
}

fn supply(r: &str, x: f64, y: f64) -> Footprint {
    Footprint::new(r, "PWR", NetClass::Power, x, y, 17.0, 8.0).with_pads(vec![
        Pad::new(2.0, 4.0, 2.0, 1.0),
        Pad::new(7.08, 4.0, 2.0, 1.0),
        Pad::new(12.16, 4.0, 2.0, 1.0),
    ])
}

fn net(name: &str, class: NetClass, nodes: &[(&str, usize)]) -> Net {
    Net::new(
        name,
        class,
        nodes.iter().map(|(r, p)| PadRef::new(r, *p)).collect(),
    )
}

// ---------------------------------------------------------------------------

fn pam8403_mini() -> Board {
    let mut b = board("PAM8403 Mini", 50.0, 35.0);
    b.add_footprint(dip8("U1", "PAM8403", NetClass::Output, 17.0, 10.0));
    b.add_footprint(rca("J1", 3.0, 6.0));
    b.add_footprint(speaker("J2", 34.0, 6.0));
    b.add_footprint(radial("C1", "10u", NetClass::Input, 6.0, 22.0));
    b.add_footprint(radial("C2", "100n", NetClass::Power, 20.0, 25.0));
    b.add_footprint(axial("R1", "10k", NetClass::Signal, 32.0, 22.0));
    b.add_net(net("IN", NetClass::Input, &[("J1", 0), ("C1", 0)]));
    b.add_net(net("IN_AC", NetClass::Input, &[("C1", 1), ("U1", 2)]));
    b.add_net(net("OUT", NetClass::Output, &[("U1", 4), ("J2", 0)]));
    b.add_net(net("FB", NetClass::Feedback, &[("U1", 5), ("R1", 0), ("R1", 1)]));
    b.add_net(net("V+", NetClass::Power, &[("J1", 1), ("U1", 3), ("C2", 0)]));
    b.add_net(net("GND", NetClass::Ground, &[("J2", 1), ("U1", 0), ("U1", 1), ("C2", 1)]));
    b
}

fn lm358_preamp() -> Board {
    let mut b = board("LM358 Preamp", 55.0, 40.0);
    b.add_footprint(dip8("U1", "LM358", NetClass::Signal, 19.0, 12.0));
    b.add_footprint(rca("J1", 3.0, 8.0));
    b.add_footprint(rca("J2", 38.0, 8.0));
    b.add_footprint(axial("R1", "1k", NetClass::Signal, 5.0, 24.0));
    b.add_footprint(axial("R2", "100k", NetClass::Feedback, 19.0, 29.0));
    b.add_footprint(radial("C1", "1u", NetClass::Input, 34.0, 24.0));
    b.add_net(net("IN", NetClass::Input, &[("J1", 0), ("R1", 0)]));
    b.add_net(net("IN2", NetClass::Input, &[("R1", 1), ("U1", 2)]));
    b.add_net(net("OUT", NetClass::Output, &[("U1", 1), ("C1", 0)]));
    b.add_net(net("OUT_J", NetClass::Output, &[("C1", 1), ("J2", 0)]));
    b.add_net(net("FB", NetClass::Feedback, &[("U1", 5), ("U1", 6), ("R2", 0)]));
    b.add_net(net("FB2", NetClass::Feedback, &[("R2", 1), ("U1", 7)]));
    b.add_net(net("V+", NetClass::Power, &[("J2", 1), ("U1", 3)]));
    b.add_net(net("GND", NetClass::Ground, &[("J1", 1), ("U1", 0), ("U1", 4)]));
    b
}

fn tda2822_bridge() -> Board {
    let mut b = board("TDA2822 Bridge", 60.0, 40.0);
    b.add_footprint(dip8("U1", "TDA2822", NetClass::Output, 22.0, 12.0));
    b.add_footprint(rca("J1", 3.0, 8.0));
    b.add_footprint(speaker("J2", 44.0, 8.0));
    b.add_footprint(radial("C1", "100u", NetClass::Power, 6.0, 26.0));
    b.add_footprint(radial("C2", "10u", NetClass::Output, 46.0, 26.0));
    b.add_footprint(axial("R1", "4k7", NetClass::Signal, 24.0, 29.0));
    b.add_net(net("IN", NetClass::Input, &[("J1", 0), ("U1", 2)]));
    b.add_net(net("OUT_A", NetClass::Output, &[("U1", 4), ("J2", 0)]));
    b.add_net(net("OUT_B", NetClass::Output, &[("U1", 5), ("C2", 0)]));
    b.add_net(net("OUT_C", NetClass::Output, &[("C2", 1), ("J2", 1)]));
    b.add_net(net("GAIN", NetClass::Signal, &[("U1", 0), ("U1", 7), ("R1", 0)]));
    b.add_net(net("FB", NetClass::Feedback, &[("R1", 1), ("U1", 6)]));
    b.add_net(net("V+", NetClass::Power, &[("J1", 1), ("U1", 3), ("C1", 0)]));
    b.add_net(net("GND", NetClass::Ground, &[("U1", 1), ("C1", 1)]));
    b
}

fn ne555_tone() -> Board {
    let mut b = board("NE555 Tone", 55.0, 40.0);
    b.add_footprint(dip8("U1", "NE555", NetClass::Signal, 20.0, 12.0));
    b.add_footprint(to92("Q1", "2N3904", NetClass::Output, 41.0, 14.0));
    b.add_footprint(speaker("J1", 40.0, 26.0));
    b.add_footprint(radial("C1", "10u", NetClass::Signal, 7.0, 25.0));
    b.add_footprint(axial("R1", "10k", NetClass::Signal, 5.0, 12.0));
    b.add_footprint(axial("R2", "1k", NetClass::Output, 20.0, 28.0));
    b.add_footprint(supply("J2", 36.0, 20.0));
    b.add_net(net("TH", NetClass::Signal, &[("R1", 0), ("U1", 6), ("U1", 2)]));
    b.add_net(net("TL", NetClass::Signal, &[("R1", 1), ("U1", 1)]));
    b.add_net(net("CAP", NetClass::Signal, &[("U1", 0), ("C1", 0)]));
    b.add_net(net("CAP_G", NetClass::Ground, &[("C1", 1), ("U1", 4)]));
    b.add_net(net("OUT", NetClass::Output, &[("U1", 3), ("Q1", 0)]));
    b.add_net(net("DRIVE", NetClass::Output, &[("Q1", 1), ("R2", 0)]));
    b.add_net(net("LOAD", NetClass::Output, &[("R2", 1), ("J1", 0), ("Q1", 2)]));
    b.add_net(net("GND", NetClass::Ground, &[("J1", 1), ("U1", 5), ("J2", 1), ("J2", 2)]));
    b.add_net(net("V+", NetClass::Power, &[("U1", 7), ("J2", 0)]));
    b
}

fn classa_single() -> Board {
    let mut b = board("Class-A Single", 70.0, 45.0);
    b.add_footprint(rca("J1", 4.0, 8.0));
    b.add_footprint(to92("Q1", "BC547", NetClass::Signal, 24.0, 10.0));
    b.add_footprint(to92("Q2", "BD137", NetClass::Output, 46.0, 10.0));
    b.add_footprint(speaker("J2", 54.0, 28.0));
    b.add_footprint(supply("J3", 4.0, 30.0));
    b.add_footprint(axial("R1", "22k", NetClass::Signal, 18.0, 24.0));
    b.add_footprint(axial("R2", "1k", NetClass::Output, 38.0, 24.0));
    b.add_footprint(radial("C1", "10u", NetClass::Signal, 28.0, 32.0));
    b.add_net(net("IN", NetClass::Input, &[("J1", 0), ("R1", 0)]));
    b.add_net(net("BASE", NetClass::Signal, &[("R1", 1), ("Q1", 0)]));
    b.add_net(net("COL", NetClass::Signal, &[("Q1", 1), ("Q2", 0)]));
    b.add_net(net("EM", NetClass::Ground, &[("Q1", 2), ("J3", 1)]));
    b.add_net(net("DRIVER", NetClass::Output, &[("Q2", 1), ("R2", 0)]));
    b.add_net(net("OUT", NetClass::Output, &[("R2", 1), ("C1", 0)]));
    b.add_net(net("SPK", NetClass::Output, &[("C1", 1), ("J2", 0)]));
    b.add_net(net("RET", NetClass::Ground, &[("J2", 1), ("J3", 2)]));
    b.add_net(net("V+", NetClass::Power, &[("J3", 0), ("Q2", 2)]));
    b
}

fn jfet_buffer() -> Board {
    let mut b = board("JFET Buffer", 60.0, 40.0);
    b.add_footprint(rca("J1", 3.0, 7.0));
    b.add_footprint(to92("Q1", "2N5457", NetClass::Signal, 22.0, 9.0));
    b.add_footprint(to92("Q2", "2N5457", NetClass::Signal, 40.0, 9.0));
    b.add_footprint(speaker("J2", 42.0, 26.0));
    b.add_footprint(supply("J3", 4.0, 27.0));
    b.add_footprint(axial("R1", "1M", NetClass::Signal, 16.0, 25.0));
    b.add_footprint(radial("C1", "2u2", NetClass::Signal, 30.0, 29.0));
    b.add_net(net("IN", NetClass::Input, &[("J1", 0), ("Q1", 0)]));
    b.add_net(net("BIAS", NetClass::Signal, &[("R1", 0), ("Q1", 1)]));
    b.add_net(net("RBIAS", NetClass::Signal, &[("R1", 1), ("Q2", 0)]));
    b.add_net(net("SRC1", NetClass::Signal, &[("Q1", 2), ("Q2", 1)]));
    b.add_net(net("OUT", NetClass::Output, &[("Q2", 2), ("C1", 0)]));
    b.add_net(net("SPK", NetClass::Output, &[("C1", 1), ("J2", 0)]));
    b.add_net(net("GND", NetClass::Ground, &[("J1", 1), ("J2", 1), ("J3", 1)]));
    b.add_net(net("V+", NetClass::Power, &[("J3", 0), ("J3", 2)]));
    b
}

fn discrete_opamp() -> Board {
    let mut b = board("Discrete Opamp", 75.0, 50.0);
    b.add_footprint(rca("J1", 3.0, 8.0));
    b.add_footprint(to92("Q1", "2N3904", NetClass::Signal, 24.0, 8.0));
    b.add_footprint(to92("Q2", "2N3904", NetClass::Signal, 24.0, 22.0));
    b.add_footprint(to92("Q3", "2N3906", NetClass::Signal, 42.0, 8.0));
    b.add_footprint(to92("Q4", "2N3906", NetClass::Output, 42.0, 22.0));
    b.add_footprint(supply("J2", 3.0, 34.0));
    b.add_footprint(speaker("J3", 58.0, 34.0));
    b.add_footprint(axial("R1", "2k2", NetClass::Signal, 26.0, 36.0));
    b.add_footprint(axial("R2", "47k", NetClass::Feedback, 44.0, 36.0));
    b.add_footprint(radial("C1", "100n", NetClass::Power, 60.0, 8.0));
    b.add_net(net("IN", NetClass::Input, &[("J1", 0), ("Q1", 0)]));
    b.add_net(net("IN2", NetClass::Input, &[("J1", 1), ("Q2", 0)]));
    b.add_net(net("PAIR", NetClass::Signal, &[("Q1", 1), ("Q2", 1)]));
    b.add_net(net("TAIL", NetClass::Ground, &[("Q1", 2), ("Q2", 2), ("J2", 1), ("C1", 1)]));
    b.add_net(net("MIRROR", NetClass::Bias, &[("Q3", 0), ("Q3", 1), ("Q4", 0), ("R2", 1)]));
    b.add_net(net("STAGE", NetClass::Signal, &[("Q3", 2), ("Q4", 1)]));
    b.add_net(net("OUT", NetClass::Output, &[("Q4", 2), ("J3", 0)]));
    b.add_net(net("RET", NetClass::Ground, &[("J3", 1), ("R1", 1)]));
    b.add_net(net("FB", NetClass::Feedback, &[("R1", 0), ("R2", 0)]));
    b.add_net(net("V+", NetClass::Power, &[("J2", 0), ("J2", 2), ("C1", 0)]));
    b
}

fn lm1875_mono() -> Board {
    let mut b = board("LM1875 Mono", 80.0, 55.0);
    b.add_footprint(to220_5("U1", "LM1875", NetClass::Output, 30.0, 10.0));
    b.add_footprint(rca("J1", 4.0, 10.0));
    b.add_footprint(speaker("J2", 60.0, 10.0));
    b.add_footprint(supply("J3", 4.0, 38.0));
    b.add_footprint(radial("C1", "2200u", NetClass::Power, 20.0, 38.0));
    b.add_footprint(radial("C2", "10u", NetClass::Signal, 38.0, 38.0));
    b.add_footprint(axial("R1", "22k", NetClass::Signal, 50.0, 36.0));
    b.add_footprint(axial("R2", "1k", NetClass::Feedback, 64.0, 36.0));
    b.add_net(net("IN", NetClass::Input, &[("J1", 0), ("U1", 0)]));
    b.add_net(net("NFB", NetClass::Feedback, &[("U1", 1), ("R1", 0)]));
    b.add_net(net("FB", NetClass::Feedback, &[("R1", 1), ("R2", 0)]));
    b.add_net(net("FB2", NetClass::Signal, &[("R2", 1), ("C2", 0)]));
    b.add_net(net("OUT", NetClass::Output, &[("U1", 3), ("J2", 0)]));
    b.add_net(net("RET", NetClass::Ground, &[("J2", 1), ("J3", 1)]));
    b.add_net(net("V+", NetClass::Power, &[("J3", 0), ("U1", 4), ("C1", 0)]));
    b.add_net(net("V-", NetClass::Power, &[("J3", 2), ("C1", 1)]));
    b.add_net(net("SIGNAL_G", NetClass::Ground, &[("C2", 1), ("U1", 2), ("J1", 1)]));
    b
}

fn ta2024_tripath() -> Board {
    let mut b = board("TA2024 Tripath", 90.0, 60.0);
    b.add_footprint(dip8("U1", "TA2024", NetClass::Output, 30.0, 12.0));
    b.add_footprint(rca("J1", 4.0, 10.0));
    b.add_footprint(speaker("J2", 70.0, 10.0));
    b.add_footprint(supply("J3", 4.0, 42.0));
    b.add_footprint(radial("C1", "1000u", NetClass::Power, 20.0, 42.0));
    b.add_footprint(radial("C2", "10u", NetClass::Signal, 36.0, 42.0));
    b.add_footprint(axial("L1", "10uH", NetClass::Output, 52.0, 42.0));
    b.add_footprint(axial("R1", "10k", NetClass::Signal, 68.0, 42.0));
    b.add_net(net("IN", NetClass::Input, &[("J1", 0), ("U1", 2)]));
    b.add_net(net("OUT_A", NetClass::Output, &[("U1", 4), ("L1", 0)]));
    b.add_net(net("FILTER", NetClass::Output, &[("L1", 1), ("J2", 0)]));
    b.add_net(net("OUT_B", NetClass::Output, &[("U1", 5), ("J2", 1)]));
    b.add_net(net("GAIN", NetClass::Signal, &[("U1", 0), ("R1", 0)]));
    b.add_net(net("FB", NetClass::Feedback, &[("R1", 1), ("U1", 6)]));
    b.add_net(net("V+", NetClass::Power, &[("J3", 0), ("U1", 3), ("C1", 0)]));
    b.add_net(net("GND", NetClass::Ground, &[("J1", 1), ("U1", 1), ("C1", 1), ("C2", 0)]));
    b.add_net(net("SIGNAL", NetClass::Signal, &[("C2", 1), ("U1", 7)]));
    b.add_net(net("RET", NetClass::Ground, &[("J3", 1), ("J3", 2)]));
    b
}

fn hybrid_output() -> Board {
    let mut b = board("Hybrid Output", 100.0, 65.0);
    b.add_footprint(rca("J1", 4.0, 10.0));
    b.add_footprint(to92("Q1", "2N3904", NetClass::Signal, 26.0, 10.0));
    b.add_footprint(to92("Q2", "2N3906", NetClass::Signal, 26.0, 26.0));
    b.add_footprint(to220("Q3", "TIP41", NetClass::Output, 50.0, 10.0));
    b.add_footprint(to220("Q4", "TIP42", NetClass::Output, 50.0, 38.0));
    b.add_footprint(speaker("J2", 78.0, 12.0));
    b.add_footprint(supply("J3", 4.0, 46.0));
    b.add_footprint(radial("C1", "4700u", NetClass::Power, 22.0, 48.0));
    b.add_footprint(axial("R1", "1k5", NetClass::Signal, 72.0, 46.0));
    b.add_net(net("IN", NetClass::Input, &[("J1", 0), ("Q1", 0)]));
    b.add_net(net("DRIVE", NetClass::Signal, &[("Q1", 1), ("Q2", 0)]));
    b.add_net(net("BIAS", NetClass::Bias, &[("Q1", 2), ("Q2", 1)]));
    b.add_net(net("UP", NetClass::Output, &[("Q2", 2), ("Q3", 0)]));
    b.add_net(net("LOW", NetClass::Output, &[("Q3", 1), ("Q4", 0)]));
    b.add_net(net("OUT", NetClass::Output, &[("Q3", 2), ("Q4", 2), ("J2", 0)]));
    b.add_net(net("FB", NetClass::Feedback, &[("J2", 1), ("R1", 0)]));
    b.add_net(net("FB2", NetClass::Feedback, &[("R1", 1), ("Q4", 1)]));
    b.add_net(net("V+", NetClass::Power, &[("J3", 0), ("C1", 0)]));
    b.add_net(net("V-", NetClass::Power, &[("J3", 2), ("C1", 1)]));
    b.add_net(net("GND", NetClass::Ground, &[("J1", 1), ("J3", 1)]));
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_extra_boards_are_unique_and_geometrically_valid() {
        let mut ids = std::collections::HashSet::new();
        for spec in EXTRA_BOARD_SPECS {
            assert!(ids.insert(spec.id), "duplicate id {}", spec.id);
            let board = (spec.build)();
            let issues = board.validate();
            assert!(issues.is_empty(), "{}: {issues:?}", spec.id);
            assert!(build(spec.id).is_some());
        }
        assert_eq!(EXTRA_BOARD_SPECS.len(), 10);
    }
}
