//! Optional external SPICE bridge.
//!
//! The self-contained solver in `ac.rs` is always available. This module can
//! additionally emit a SPICE deck from the same board/model description and,
//! if `ngspice` is installed, run it in batch mode and convert the AC table
//! into the same `AcReport`. It never fails a pipeline when the simulator is
//! absent: agents get the proxy result instead.

use crate::ac::{AcConfig, AcModel, AcPoint, AcReport};
use crate::board::Board;
use std::path::PathBuf;

fn value_token(value: &str) -> Option<String> {
    let v = value.trim();
    let lower = v.to_ascii_lowercase();
    if lower.ends_with("uh") || lower.ends_with('h') {
        let number: f64 = lower.trim_end_matches("uh").trim_end_matches('h').parse().ok()?;
        return Some(format!("{}u", number));
    }
    if let Some(s) = lower.strip_suffix('n') {
        let number: f64 = s.parse().ok()?;
        return Some(format!("{}n", number));
    }
    if let Some(s) = lower.strip_suffix('u') {
        let number: f64 = s.parse().ok()?;
        return Some(format!("{}u", number));
    }
    if let Some(s) = lower.strip_suffix('k') {
        let number: f64 = s.parse().ok()?;
        return Some(format!("{}k", number));
    }
    if let Some(s) = lower.strip_suffix('r') {
        let number: f64 = s.parse().ok()?;
        return Some(format!("{}", number));
    }
    None
}

fn net_of(board: &Board, refdes: &str, pin: usize) -> Option<String> {
    board
        .nets
        .iter()
        .find(|net| net.nodes.iter().any(|n| n.refdes == refdes && n.pad == pin))
        .map(|n| n.name.clone())
}

/// Generate a deterministic ngspice AC deck.
pub fn generate_netlist(board: &Board, config: &AcConfig) -> String {
    let mut deck = String::from("* AmpRoute Studio AC analysis\n");
    deck.push_str("* board: ");
    deck.push_str(&board.name);
    deck.push('\n');
    for fp in &board.footprints {
        let (a, b) = (net_of(board, &fp.refdes, 0), net_of(board, &fp.refdes, 1));
        match config.models.get(&fp.refdes) {
            Some(AcModel::Resistor { ohm }) => {
                if let (Some(a), Some(b)) = (a, b) {
                    deck.push_str(&format!("R{} {} {} {}\n", fp.refdes, a, b, ohm));
                }
            }
            Some(AcModel::Capacitor { farad }) => {
                if let (Some(a), Some(b)) = (a, b) {
                    deck.push_str(&format!("C{} {} {} {}\n", fp.refdes, a, b, farad));
                }
            }
            Some(AcModel::Inductor { henry }) => {
                if let (Some(a), Some(b)) = (a, b) {
                    deck.push_str(&format!("L{} {} {} {}\n", fp.refdes, a, b, henry));
                }
            }
            Some(AcModel::OpAmp {
                plus_pin,
                minus_pin,
                out_pin,
                gain,
                input_impedance_ohm,
            }) => {
                let (p, m, out) = (
                    net_of(board, &fp.refdes, *plus_pin),
                    net_of(board, &fp.refdes, *minus_pin),
                    net_of(board, &fp.refdes, *out_pin),
                );
                if let (Some(p), Some(m), Some(out)) = (p, m, out) {
                    deck.push_str(&format!("Rin{} {} 0 {}\n", fp.refdes, p, input_impedance_ohm));
                    deck.push_str(&format!("Rinm{} {} 0 {}\n", fp.refdes, m, input_impedance_ohm));
                    deck.push_str(&format!(
                        "E{} {} 0 {} {} {}\n",
                        fp.refdes, out, p, m, gain
                    ));
                }
            }
            None => {
                if let (Some((kind, value)), Some(a), Some(b)) =
                    (value_token(&fp.value).map(|v| (fp.value.chars().next().unwrap_or('R'), v)), a, b)
                {
                    let prefix = fp.value.to_ascii_lowercase().chars().last().unwrap_or('r');
                    let letter = if fp.value.to_ascii_lowercase().ends_with('h') {
                        'L'
                    } else if matches!(prefix, 'n' | 'u' | 'p' | 'f') {
                        'C'
                    } else {
                        let _ = kind;
                        'R'
                    };
                    deck.push_str(&format!("{}{} {} {} {}\n", letter, fp.refdes, a, b, value));
                }
            }
        }
    }
    let input = config.input_net.clone().unwrap_or_else(|| {
        board
            .nets
            .iter()
            .find(|n| n.class == crate::board::NetClass::Input)
            .map(|n| n.name.clone())
            .unwrap_or_else(|| "IN".into())
    });
    let output = config.output_net.clone().unwrap_or_else(|| {
        board
            .nets
            .iter()
            .find(|n| n.class == crate::board::NetClass::Output)
            .map(|n| n.name.clone())
            .unwrap_or_else(|| "OUT".into())
    });
    deck.push_str(&format!("Vin {} 0 DC 0 AC 1\n", input));
    if let Some(load) = config.load_ohm {
        deck.push_str(&format!("Rload {} 0 {}\n", output, load));
    }
    deck.push_str(".op\n");
    deck.push_str(&format!(
        ".ac dec {} {} {}\n",
        config.points.max(2),
        config.f_start_hz,
        config.f_end_hz
    ));
    deck.push_str(&format!(".print ac vdb({}) vp({})\n", output, output));
    deck.push_str(".end\n");
    deck
}

/// Find ngspice, including the usual Windows install locations.
pub fn find_ngspice() -> Option<PathBuf> {
    // Explicit override wins: useful for portable installs and CI.
    if let Some(path) = std::env::var_os("AMPROUTE_NGSPICE") {
        let candidate = PathBuf::from(&path);
        if candidate.is_file() {
            // On Windows the GUI-subsystem ngspice.exe writes nothing when
            // launched with -b; the console build is the batch-capable one.
            let console = candidate.with_file_name("ngspice_con.exe");
            return Some(if console.is_file() { console } else { candidate });
        }
    }
    let mut candidates = Vec::new();
    if let Some(programs) = std::env::var_os("ProgramFiles") {
        for name in ["ngspice", "Ngspice"] {
            candidates.push(
                PathBuf::from(&programs)
                    .join(name)
                    .join("bin")
                    .join("ngspice.exe"),
            );
        }
    }
    candidates.push(PathBuf::from("/usr/bin/ngspice"));
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .or_else(|| {
            std::process::Command::new("ngspice")
                .arg("--version")
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|_| PathBuf::from("ngspice"))
        })
}

/// Run the deck through ngspice, returning `None` when unavailable.
pub fn run_ngspice(board: &Board, config: &AcConfig) -> Option<AcReport> {
    let exe = find_ngspice()?;
    let deck = generate_netlist(board, config);
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = std::env::temp_dir().join(format!(
        "amproute_ac_{}_{}.cir",
        std::process::id(),
        seq
    ));
    std::fs::write(&tmp, &deck).ok()?;
    let output = std::process::Command::new(exe)
        .arg("-b")
        .arg(&tmp)
        .output()
        .ok()?;
    std::fs::remove_file(&tmp).ok();
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut points = Vec::new();
    for line in text.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        // ngspice -b prints "Index frequency vdb(v) vp(v)": skip the index.
        let offset = if cols.first().is_some_and(|c| c.parse::<usize>().is_ok()) {
            1
        } else {
            0
        };
        if cols.len() < offset + 3 {
            continue;
        }
        if let (Ok(hz), Ok(db), Ok(phase)) = (
            cols[offset].parse::<f64>(),
            cols[offset + 1].parse::<f64>(),
            cols[offset + 2].parse::<f64>(),
        ) {
            points.push(AcPoint { hz, gain_db: db, phase_deg: phase });
        }
    }
    if points.is_empty() {
        return None;
    }
    let dc_gain_db = points[0].gain_db;
    let bandwidth_hz = points.iter().find(|p| p.gain_db <= dc_gain_db - 3.0).map(|p| p.hz);
    let phase_margin_deg = points
        .windows(2)
        .find(|w| w[0].gain_db >= 0.0 && w[1].gain_db < 0.0)
        .map(|w| 180.0 + w[1].phase_deg);
    Some(AcReport {
        points,
        dc_gain_db,
        bandwidth_hz,
        phase_margin_deg,
        model: format!("ngspice {}", String::from_utf8_lossy(&output.stdout).lines().take(1).next().unwrap_or("")),
        confidence: "medium".into(),
        assumptions: vec!["external SPICE: models supplied by the agent".into()],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{Footprint, LutPreset, Net, NetClass, Pad, PadRef};

    fn board() -> Board {
        let mut b = Board::new("spice-test", 40.0, 30.0, LutPreset::reliable());
        b.add_footprint(
            Footprint::new("R1", "10k", NetClass::Signal, 5.0, 10.0, 10.0, 4.0)
                .with_pads(vec![Pad::new(1.0, 2.0, 1.6, 0.8), Pad::new(9.0, 2.0, 1.6, 0.8)]),
        );
        b.add_footprint(
            Footprint::new("C1", "100n", NetClass::Signal, 20.0, 10.0, 10.0, 4.0)
                .with_pads(vec![Pad::new(1.0, 2.0, 1.6, 0.8), Pad::new(9.0, 2.0, 1.6, 0.8)]),
        );
        b.add_net(Net::new("IN", NetClass::Input, vec![PadRef::new("R1", 0)]));
        b.add_net(Net::new("OUT", NetClass::Output, vec![PadRef::new("R1", 1), PadRef::new("C1", 0)]));
        b.add_net(Net::new("GND", NetClass::Ground, vec![PadRef::new("C1", 1)]));
        b
    }

    #[test]
    fn netlist_contains_passives_source_and_ac_deck() {
        let deck = generate_netlist(&board(), &AcConfig::default());
        assert!(deck.contains("R1 IN OUT 10k"));
        assert!(deck.contains("C1 OUT GND 100n"));
        assert!(deck.contains("Vin IN 0 DC 0 AC 1"));
        assert!(deck.contains(".ac dec"));
        assert!(deck.trim_end().ends_with(".end"));
    }

    #[test]
    fn external_run_is_absent_on_machines_without_ngspice() {
        if find_ngspice().is_none() {
            assert!(run_ngspice(&board(), &AcConfig::default()).is_none());
        }
    }
}
