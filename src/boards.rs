//! The five built-in test amplifier boards.
//!
//! Each one is a single **left channel** of a stereo amplifier: input connector
//! on the left, gain stage in the middle, output devices and speaker terminals
//! on the right. They are deliberately different sizes and topologies, because
//! the point is to prove the autorouter copes with more than one layout — not to
//! be a product line.
//!
//! All five follow the same physical rules:
//!
//! * Mounting holes in the corners, each with a copper keep-out around it.
//! * Copper kept clear of the board edge by `preset.min_clearance_mm + 0.2`.
//! * Every pad belongs to exactly one net. A pad on no net is a permanent
//!   obstacle for every other net, and a pad on two nets is a short circuit.
//! * Input nets kept away from output and power nets (ТЗ 5.2).
//!
//! Pad numbering for the three-lead parts is `0 = base/gate`, `1 = collector`
//! or the supply pin, `2 = emitter` or the output pin.

use crate::board::{Board, Footprint, LutPreset, Net, NetClass, Pad, PadRef};

/// A board offered in the picker.
pub struct BoardSpec {
    /// Stable identifier used by `--board` and the GUI.
    pub id: &'static str,
    /// One-line description for the picker.
    pub summary: &'static str,
    /// Builds the board on demand, so nothing is built that is not shown.
    pub build: fn() -> Board,
}

/// All built-in boards, in the order the GUI lists them.
pub const BOARD_SPECS: &[BoardSpec] = &[
    BoardSpec {
        id: "lm386-mini",
        summary: "LM386 single chip — 45 × 35 mm",
        build: lm386_mini,
    },
    BoardSpec {
        id: "tda2030-compact",
        summary: "TDA2030 power IC — 70 × 50 mm, split supply",
        build: tda2030_compact,
    },
    BoardSpec {
        id: "cmoy-headphone",
        summary: "NE5532 op-amp headphone amp — 80 × 50 mm, virtual ground",
        build: cmoy_headphone,
    },
    BoardSpec {
        id: "classic-ltp",
        summary: "Class-AB LTP + VAS + triple output — 130 × 90 mm",
        build: classic_ltp,
    },
    BoardSpec {
        id: "twin-pair-200",
        summary: "Dual-pair discrete output — 170 × 110 mm",
        build: twin_pair_200,
    },
];

/// Look a board up by id.
pub fn build(id: &str) -> Option<Board> {
    BOARD_SPECS
        .iter()
        .find(|s| s.id.eq_ignore_ascii_case(id))
        .map(|s| (s.build)())
}

/// The board shown at start-up.
pub fn default_board() -> Board {
    classic_ltp()
}

/// Every board, built. Used by the tests and by `--cli --board all`.
pub fn all_boards() -> Vec<Board> {
    BOARD_SPECS.iter().map(|s| (s.build)()).collect()
}

// ---------------------------------------------------------------------------
// Building blocks
// ---------------------------------------------------------------------------

/// Two-lead axial part (resistor, small cap): pads 10 mm apart.
fn axial(refdes: &str, value: &str, class: NetClass, x: f64, y: f64) -> Footprint {
    Footprint::new(refdes, value, class, x, y, 12.0, 6.0).with_pads(vec![
        Pad::new(1.0, 3.0, 1.8, 0.9),
        Pad::new(11.0, 3.0, 1.8, 0.9),
    ])
}

/// Radial electrolytic: 5 mm lead spacing.
fn radial(refdes: &str, value: &str, class: NetClass, x: f64, y: f64) -> Footprint {
    Footprint::new(refdes, value, class, x, y, 9.0, 6.0).with_pads(vec![
        Pad::new(2.0, 3.0, 1.8, 0.9),
        Pad::new(7.0, 3.0, 1.8, 0.9),
    ])
}

/// TO-92 transistor: base, collector, emitter on a 2.54 mm pitch.
fn to92(refdes: &str, value: &str, class: NetClass, x: f64, y: f64) -> Footprint {
    Footprint::new(refdes, value, class, x, y, 9.0, 7.0).with_pads(vec![
        Pad::new(1.3, 3.5, 1.6, 0.8),
        Pad::new(3.84, 3.5, 1.6, 0.8),
        Pad::new(6.38, 3.5, 1.6, 0.8),
    ])
}

/// TO-220 three-lead power device: base, collector, emitter.
///
/// 1.8 mm pads on a 2.54 mm pitch match the standard KiCad footprint and leave
/// a 0.74 mm gap, comfortably above the LUT clearance.
fn to220(refdes: &str, value: &str, class: NetClass, x: f64, y: f64) -> Footprint {
    let mut pads = Vec::new();
    for i in 0..3 {
        pads.push(Pad::new(2.0 + i as f64 * 2.54, 8.0, 1.8, 1.0));
    }
    Footprint::new(refdes, value, class, x, y, 14.0, 12.0).with_pads(pads)
}

/// TO-220-5 power IC (TDA2030 and relatives), 2.54 mm pitch.
fn to220_5(refdes: &str, value: &str, class: NetClass, x: f64, y: f64) -> Footprint {
    let mut pads = Vec::new();
    for i in 0..5 {
        pads.push(Pad::new(2.0 + i as f64 * 2.54, 8.0, 1.8, 1.0));
    }
    Footprint::new(refdes, value, class, x, y, 16.0, 12.0).with_pads(pads)
}

/// DIP-8. Pads 0–3 are pins 1–4 down the left, 4–7 are pins 5–8 back on the right.
fn dip8(refdes: &str, value: &str, class: NetClass, x: f64, y: f64) -> Footprint {
    let mut pads = Vec::new();
    for i in 0..4 {
        pads.push(Pad::new(2.0 + i as f64 * 2.54, 2.0, 1.6, 0.8));
    }
    for i in 0..4 {
        pads.push(Pad::new(2.0 + (3 - i) as f64 * 2.54, 9.62, 1.6, 0.8));
    }
    Footprint::new(refdes, value, class, x, y, 14.0, 14.0).with_pads(pads)
}

/// Three-pad screw terminal on the standard 5.08 mm pitch.
///
/// The pitch matters: at 3.5 mm a 0.8 mm ground track cannot pass between two
/// neighbouring pads once clearance is added, which is a real manufacturing
/// limit rather than a router limitation.
fn terminal3(refdes: &str, x: f64, y: f64) -> Footprint {
    Footprint::new(refdes, "PWR", NetClass::Power, x, y, 17.0, 8.0).with_pads(vec![
        Pad::new(2.0, 4.0, 2.0, 1.0),
        Pad::new(7.08, 4.0, 2.0, 1.0),
        Pad::new(12.16, 4.0, 2.0, 1.0),
    ])
}

/// Two-pad speaker terminal on the standard 5.08 mm pitch.
fn speaker2(refdes: &str, x: f64, y: f64) -> Footprint {
    Footprint::new(refdes, "SPK", NetClass::Output, x, y, 13.0, 8.0).with_pads(vec![
        Pad::new(2.0, 4.0, 2.2, 1.1),
        Pad::new(7.08, 4.0, 2.2, 1.1),
    ])
}

/// Two-pad line input: signal and ground on a 5.08 mm pitch.
fn rca_in(refdes: &str, x: f64, y: f64) -> Footprint {
    Footprint::new(refdes, "RCA-IN", NetClass::Input, x, y, 13.0, 9.0).with_pads(vec![
        Pad::new(2.5, 4.5, 2.0, 1.0),
        Pad::new(7.58, 4.5, 2.0, 1.0),
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
// 1. LM386 mini — 45 × 35 mm
// ---------------------------------------------------------------------------

/// Smallest useful amplifier: one LM386 in a DIP-8 and a handful of passives.
///
/// All eight pins are wired, including gain and bypass, so no pad is left
/// behind as a routing obstacle.
pub fn lm386_mini() -> Board {
    let mut b = Board::new("LM386 Mini", 45.0, 35.0, LutPreset::advanced());
    b.description = "Single-chip LM386, 45 × 35 mm".into();
    b.add_corner_holes(6.0, 2.5);

    // U1 pins 1–8 → pads 0–7.
    b.add_footprint(dip8("U1", "LM386", NetClass::Output, 18.0, 10.0));

    b.add_footprint(rca_in("J1", 3.0, 8.0));
    b.add_footprint(radial("C1", "100nF", NetClass::Input, 4.0, 22.0));
    b.add_footprint(radial("C2", "10uF", NetClass::Signal, 16.0, 26.0));
    b.add_footprint(radial("C3", "100uF", NetClass::Power, 28.0, 26.0));
    b.add_footprint(speaker2("J2", 28.0, 3.0));

    b.add_net(net("IN", NetClass::Input, &[("J1", 0), ("C1", 0)]));
    b.add_net(net("IN_AC", NetClass::Input, &[("C1", 1), ("U1", 2)]));
    b.add_net(net("GAIN", NetClass::Signal, &[("U1", 0), ("U1", 7)]));
    b.add_net(net("BYPASS", NetClass::Signal, &[("U1", 6), ("C2", 0)]));
    b.add_net(net("OUT", NetClass::Output, &[("U1", 5), ("J2", 0)]));
    b.add_net(net("V+", NetClass::Power, &[("U1", 4), ("C3", 0)]));
    b.add_net(net(
        "GND",
        NetClass::Ground,
        &[
            ("J1", 1),
            ("U1", 1),
            ("U1", 3),
            ("C2", 1),
            ("C3", 1),
            ("J2", 1),
        ],
    ));
    b
}

// ---------------------------------------------------------------------------
// 2. TDA2030 compact — 70 × 50 mm
// ---------------------------------------------------------------------------

/// A power-IC amplifier: one TDA2030 in TO-220-5 on a heatsink tab, split
/// supply, and a gain-setting feedback divider.
pub fn tda2030_compact() -> Board {
    let mut b = Board::new("TDA2030 Compact", 70.0, 50.0, LutPreset::reliable());
    b.description = "TDA2030 power IC, 70 × 50 mm".into();
    b.add_corner_holes(5.0, 3.2);

    // U1 pads: 0 = +IN, 1 = -IN, 2 = -VS, 3 = OUT, 4 = +VS.
    b.add_footprint(to220_5("U1", "TDA2030", NetClass::Output, 30.0, 14.0));

    b.add_footprint(rca_in("J1", 4.0, 8.0));
    b.add_footprint(radial("C1", "2u2", NetClass::Input, 5.0, 24.0));
    b.add_footprint(axial("R1", "22k", NetClass::Feedback, 22.0, 36.0));
    b.add_footprint(axial("R2", "680R", NetClass::Feedback, 6.0, 36.0));

    b.add_footprint(terminal3("J2", 8.0, 40.0));
    b.add_footprint(radial("C2", "100nF", NetClass::Power, 54.0, 6.0));
    b.add_footprint(radial("C3", "2200uF", NetClass::Power, 52.0, 18.0));
    b.add_footprint(radial("C4", "2200uF", NetClass::Power, 52.0, 32.0));
    b.add_footprint(speaker2("J3", 24.0, 40.0));

    b.add_net(net("IN", NetClass::Input, &[("J1", 0), ("C1", 0)]));
    b.add_net(net("IN_AC", NetClass::Input, &[("C1", 1), ("U1", 0)]));
    b.add_net(net("FB", NetClass::Feedback, &[("U1", 1), ("R1", 0), ("R2", 1)]));
    b.add_net(net("OUT", NetClass::Output, &[("U1", 3), ("R1", 1), ("J3", 0)]));
    b.add_net(net(
        "V+",
        NetClass::Power,
        &[("U1", 4), ("C2", 0), ("C3", 0), ("J2", 0)],
    ));
    b.add_net(net("V-", NetClass::Power, &[("U1", 2), ("C4", 0), ("J2", 1)]));
    b.add_net(net(
        "GND",
        NetClass::Ground,
        &[
            ("J1", 1),
            ("R2", 0),
            ("C2", 1),
            ("C3", 1),
            ("C4", 1),
            ("J2", 2),
            ("J3", 1),
        ],
    ));
    b
}

// ---------------------------------------------------------------------------
// 3. CMoy headphone amp — 80 × 50 mm
// ---------------------------------------------------------------------------

/// A CMoy-style headphone amplifier: a single NE5532 channel, a resistor
/// divider for the virtual ground, and an output coupling capacitor.
pub fn cmoy_headphone() -> Board {
    let mut b = Board::new("CMoy Headphone", 80.0, 50.0, LutPreset::advanced());
    b.description = "NE5532 headphone amp with virtual ground, 80 × 50 mm".into();
    b.add_corner_holes(5.0, 3.2);

    // U1 pads 0–7 are pins 1–8. Channel A amplifies; channel B is unused and its
    // pins are tied to a defined potential so they are not floating metal.
    b.add_footprint(dip8("U1", "NE5532", NetClass::Signal, 26.0, 10.0));

    b.add_footprint(rca_in("J1", 5.0, 8.0));
    b.add_footprint(radial("C1", "1uF", NetClass::Input, 6.0, 24.0));
    b.add_footprint(axial("R1", "10k", NetClass::Feedback, 24.0, 32.0));
    b.add_footprint(axial("R2", "47k", NetClass::Feedback, 40.0, 32.0));
    b.add_footprint(radial("C2", "470uF", NetClass::Output, 58.0, 8.0));

    // Virtual-ground divider and its reservoir.
    b.add_footprint(axial("R3", "10k", NetClass::Bias, 26.0, 42.0));
    b.add_footprint(axial("R4", "10k", NetClass::Bias, 42.0, 42.0));
    b.add_footprint(radial("C3", "100uF", NetClass::Bias, 60.0, 24.0));
    b.add_footprint(radial("C4", "10uF", NetClass::Power, 44.0, 8.0));

    b.add_footprint(speaker2("J2", 64.0, 40.0));

    b.add_net(net("IN", NetClass::Input, &[("J1", 0), ("C1", 0)]));
    b.add_net(net("IN_AC", NetClass::Input, &[("C1", 1), ("U1", 2)]));
    b.add_net(net("FB", NetClass::Feedback, &[("U1", 1), ("R1", 0), ("R2", 1)]));
    b.add_net(net("AMP_OUT", NetClass::Output, &[("U1", 0), ("R2", 0), ("C2", 0)]));
    b.add_net(net("PHONES", NetClass::Output, &[("C2", 1), ("J2", 0)]));
    b.add_net(net("SPARE", NetClass::Signal, &[("U1", 5), ("U1", 6)]));
    b.add_net(net(
        "VGND",
        NetClass::Bias,
        &[("R3", 1), ("R4", 0), ("C3", 0), ("U1", 4)],
    ));
    b.add_net(net(
        "V+",
        NetClass::Power,
        &[("U1", 7), ("R3", 0), ("C3", 1), ("C4", 0)],
    ));
    b.add_net(net(
        "GND",
        NetClass::Ground,
        &[
            ("J1", 1),
            ("U1", 3),
            ("R1", 1),
            ("R4", 1),
            ("C4", 1),
            ("J2", 1),
        ],
    ));
    b
}

// ---------------------------------------------------------------------------
// 4. Classic LTP — 130 × 90 mm
// ---------------------------------------------------------------------------

/// The classic three-stage class-AB amplifier: long-tailed pair, voltage
/// amplifier stage, complementary drivers and a quasi-complementary output.
///
/// The default board: it exercises every net class the router knows about.
pub fn classic_ltp() -> Board {
    let mut b = Board::new("Classic LTP", 130.0, 90.0, LutPreset::reliable());
    b.description = "Class-AB, LTP + VAS + triple output, 130 × 90 mm".into();
    b.add_corner_holes(6.0, 3.2);

    // --- Input, far left ---
    b.add_footprint(rca_in("J1", 6.0, 22.0));
    b.add_footprint(radial("C1", "4u7", NetClass::Input, 8.0, 36.0));
    b.add_footprint(axial("R1", "100k", NetClass::Input, 24.0, 38.0));

    // --- Long-tailed pair ---
    b.add_footprint(to92("Q1", "K170", NetClass::Input, 44.0, 18.0));
    b.add_footprint(to92("Q2", "K170", NetClass::Feedback, 44.0, 34.0));
    b.add_footprint(axial("R2", "22k", NetClass::Bias, 44.0, 50.0));

    // --- Voltage amplifier stage ---
    b.add_footprint(to92("Q3", "K310", NetClass::Signal, 64.0, 18.0));
    b.add_footprint(radial("C2", "47pF", NetClass::Feedback, 64.0, 34.0));

    // --- Drivers ---
    b.add_footprint(to220("Q4", "TIP41C", NetClass::Output, 82.0, 8.0));
    b.add_footprint(to220("Q5", "TIP42C", NetClass::Output, 82.0, 44.0));

    // --- Output devices ---
    b.add_footprint(to220("Q6", "2N3055", NetClass::Output, 106.0, 8.0));
    b.add_footprint(to220("Q7", "MJE2955", NetClass::Output, 106.0, 44.0));

    // --- Emitter resistors ---
    b.add_footprint(axial("R3", "0R47", NetClass::Output, 82.0, 28.0));
    b.add_footprint(axial("R4", "0R47", NetClass::Output, 82.0, 64.0));

    // --- Feedback divider ---
    b.add_footprint(axial("R5", "22k", NetClass::Feedback, 64.0, 50.0));
    b.add_footprint(axial("R6", "1k", NetClass::Feedback, 64.0, 64.0));

    // --- Supply and output connectors ---
    b.add_footprint(terminal3("J2", 8.0, 74.0));
    b.add_footprint(radial("C3", "4700uF", NetClass::Power, 32.0, 78.0));
    b.add_footprint(radial("C4", "4700uF", NetClass::Power, 50.0, 78.0));
    b.add_footprint(speaker2("J3", 110.0, 74.0));

    b.add_net(net("IN", NetClass::Input, &[("J1", 0), ("C1", 0)]));
    b.add_net(net("IN_AC", NetClass::Input, &[("C1", 1), ("R1", 0), ("Q1", 0)]));
    b.add_net(net("LTP_A", NetClass::Signal, &[("Q1", 1), ("Q3", 0)]));
    b.add_net(net("LTP_B", NetClass::Signal, &[("Q2", 1), ("C2", 0)]));
    b.add_net(net("TAIL", NetClass::Bias, &[("Q1", 2), ("Q2", 2), ("R2", 0)]));
    b.add_net(net(
        "VAS_OUT",
        NetClass::Signal,
        &[("Q3", 1), ("Q4", 0), ("Q5", 0)],
    ));
    b.add_net(net("FB", NetClass::Feedback, &[("Q2", 0), ("C2", 1), ("R5", 1), ("R6", 0)]));
    b.add_net(net("DRV_N", NetClass::Output, &[("Q4", 2), ("Q6", 0)]));
    b.add_net(net("DRV_P", NetClass::Output, &[("Q5", 2), ("Q7", 0)]));
    b.add_net(net("E_N", NetClass::Output, &[("Q6", 2), ("R3", 0)]));
    b.add_net(net("E_P", NetClass::Output, &[("Q7", 2), ("R4", 0)]));
    b.add_net(net(
        "SPK",
        NetClass::Output,
        &[("R3", 1), ("R4", 1), ("R5", 0), ("J3", 0)],
    ));
    b.add_net(net(
        "V+",
        NetClass::Power,
        &[("Q3", 2), ("Q4", 1), ("Q6", 1), ("C3", 0), ("J2", 0)],
    ));
    b.add_net(net(
        "V-",
        NetClass::Power,
        &[("R2", 1), ("Q5", 1), ("Q7", 1), ("C4", 0), ("J2", 1)],
    ));
    b.add_net(net(
        "GND",
        NetClass::Ground,
        &[
            ("J1", 1),
            ("R1", 1),
            ("R6", 1),
            ("C3", 1),
            ("C4", 1),
            ("J2", 2),
            ("J3", 1),
        ],
    ));
    b
}

// ---------------------------------------------------------------------------
// 5. Twin pair 200 — 170 × 110 mm
// ---------------------------------------------------------------------------

/// A bigger discrete amplifier: two output devices in parallel per rail for
/// more current, each with its own emitter resistor.
pub fn twin_pair_200() -> Board {
    let mut b = Board::new("Twin Pair 200", 170.0, 110.0, LutPreset::reliable());
    b.description = "Dual-pair discrete output stage, 170 × 110 mm".into();
    b.add_corner_holes(6.0, 3.2);

    // --- Input ---
    b.add_footprint(rca_in("J1", 6.0, 26.0));
    b.add_footprint(radial("C1", "10uF", NetClass::Input, 8.0, 40.0));
    b.add_footprint(axial("R1", "100k", NetClass::Input, 26.0, 42.0));

    // --- Differential pair ---
    b.add_footprint(to92("Q1", "K170", NetClass::Input, 48.0, 18.0));
    b.add_footprint(to92("Q2", "K170", NetClass::Feedback, 48.0, 36.0));
    b.add_footprint(axial("R2", "15k", NetClass::Bias, 48.0, 54.0));

    // --- Voltage amplifier ---
    b.add_footprint(to92("Q3", "K310", NetClass::Signal, 70.0, 14.0));
    b.add_footprint(radial("C2", "100pF", NetClass::Feedback, 70.0, 38.0));

    // --- Drivers ---
    b.add_footprint(to220("Q4", "MJE15030", NetClass::Output, 88.0, 6.0));
    b.add_footprint(to220("Q5", "MJE15031", NetClass::Output, 88.0, 48.0));

    // --- Output pairs ---
    b.add_footprint(to220("Q6", "2SC5200", NetClass::Output, 116.0, 4.0));
    b.add_footprint(to220("Q7", "2SC5200", NetClass::Output, 116.0, 26.0));
    b.add_footprint(to220("Q8", "2SA1943", NetClass::Output, 116.0, 60.0));
    b.add_footprint(to220("Q9", "2SA1943", NetClass::Output, 116.0, 82.0));

    // --- Emitter resistors, one per output device ---
    b.add_footprint(axial("R3", "0R22", NetClass::Output, 90.0, 28.0));
    b.add_footprint(axial("R4", "0R22", NetClass::Output, 90.0, 66.0));
    b.add_footprint(axial("R5", "0R22", NetClass::Output, 142.0, 6.0));
    b.add_footprint(axial("R6", "0R22", NetClass::Output, 142.0, 82.0));

    // --- Feedback divider ---
    b.add_footprint(axial("R7", "33k", NetClass::Feedback, 70.0, 54.0));
    b.add_footprint(axial("R8", "1k", NetClass::Feedback, 70.0, 68.0));

    // --- Supply and output ---
    b.add_footprint(terminal3("J2", 10.0, 90.0));
    b.add_footprint(radial("C3", "10000uF", NetClass::Power, 36.0, 94.0));
    b.add_footprint(radial("C4", "10000uF", NetClass::Power, 54.0, 94.0));
    b.add_footprint(speaker2("J3", 150.0, 92.0));

    b.add_net(net("IN", NetClass::Input, &[("J1", 0), ("C1", 0)]));
    b.add_net(net("IN_AC", NetClass::Input, &[("C1", 1), ("R1", 0), ("Q1", 0)]));
    b.add_net(net("LTP_A", NetClass::Signal, &[("Q1", 1), ("Q3", 0)]));
    b.add_net(net("LTP_B", NetClass::Signal, &[("Q2", 1), ("C2", 0)]));
    b.add_net(net("TAIL", NetClass::Bias, &[("Q1", 2), ("Q2", 2), ("R2", 0)]));
    b.add_net(net(
        "VAS_OUT",
        NetClass::Signal,
        &[("Q3", 1), ("Q4", 0), ("Q5", 0)],
    ));
    b.add_net(net("FB", NetClass::Feedback, &[("Q2", 0), ("C2", 1), ("R7", 1), ("R8", 0)]));
    b.add_net(net("DRV_N", NetClass::Output, &[("Q4", 2), ("Q6", 0), ("Q7", 0)]));
    b.add_net(net("DRV_P", NetClass::Output, &[("Q5", 2), ("Q8", 0), ("Q9", 0)]));
    b.add_net(net("E_N1", NetClass::Output, &[("Q6", 2), ("R3", 0)]));
    b.add_net(net("E_N2", NetClass::Output, &[("Q7", 2), ("R4", 0)]));
    b.add_net(net("E_P1", NetClass::Output, &[("Q8", 2), ("R5", 0)]));
    b.add_net(net("E_P2", NetClass::Output, &[("Q9", 2), ("R6", 0)]));
    b.add_net(net(
        "SPK",
        NetClass::Output,
        &[
            ("R3", 1),
            ("R4", 1),
            ("R5", 1),
            ("R6", 1),
            ("R7", 0),
            ("J3", 0),
        ],
    ));
    b.add_net(net(
        "V+",
        NetClass::Power,
        &[
            ("Q3", 2),
            ("Q4", 1),
            ("Q6", 1),
            ("Q7", 1),
            ("C3", 0),
            ("J2", 0),
        ],
    ));
    b.add_net(net(
        "V-",
        NetClass::Power,
        &[
            ("R2", 1),
            ("Q5", 1),
            ("Q8", 1),
            ("Q9", 1),
            ("C4", 0),
            ("J2", 1),
        ],
    ));
    b.add_net(net(
        "GND",
        NetClass::Ground,
        &[
            ("J1", 1),
            ("R1", 1),
            ("R8", 1),
            ("C3", 1),
            ("C4", 1),
            ("J2", 2),
            ("J3", 1),
        ],
    ));
    b
}
