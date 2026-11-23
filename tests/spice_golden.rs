//! Golden AC case against a real ngspice run.
//!
//! Set AMPROUTE_NGSPICE to the executable path to enable. Without it the test
//! skips, so the suite stays portable.

use amproute_studio::ac::{AcConfig, AcModel};
use std::collections::HashMap;
use amproute_studio::board::{Board, Footprint, LutPreset, Net, NetClass, Pad, PadRef};
use amproute_studio::spice::{find_ngspice, generate_netlist, run_ngspice};

fn rc_board() -> Board {
    let mut b = Board::new("rc-golden", 40.0, 30.0, LutPreset::reliable());
    b.add_footprint(
        Footprint::new("R1", "10k", NetClass::Signal, 5.0, 10.0, 10.0, 4.0)
            .with_pads(vec![Pad::new(1.0, 2.0, 1.6, 0.8), Pad::new(9.0, 2.0, 1.6, 0.8)]),
    );
    b.add_footprint(
        Footprint::new("C1", "100n", NetClass::Signal, 20.0, 10.0, 10.0, 4.0)
            .with_pads(vec![Pad::new(1.0, 2.0, 1.6, 0.8), Pad::new(9.0, 2.0, 1.6, 0.8)]),
    );
    b.add_net(Net::new("IN", NetClass::Input, vec![PadRef::new("R1", 0)]));
    b.add_net(Net::new(
        "OUT",
        NetClass::Output,
        vec![PadRef::new("R1", 1), PadRef::new("C1", 0)],
    ));
    b.add_net(Net::new("GND", NetClass::Ground, vec![PadRef::new("C1", 1)]));
    b
}

#[test]
fn ngspice_matches_the_analytic_rc_cutoff_when_available() {
    let Some(exe) = find_ngspice() else {
        eprintln!("skip: set AMPROUTE_NGSPICE to run the golden SPICE case");
        return;
    };
    let config = AcConfig {
        input_net: Some("IN".into()),
        output_net: Some("OUT".into()),
        f_start_hz: 10.0,
        f_end_hz: 10_000.0,
        points: 40,
        ..Default::default()
    };
    let deck = generate_netlist(&rc_board(), &config);
    assert!(deck.contains("R1 IN OUT 10k"));
    assert!(deck.contains("C1 OUT GND 100n"));

    let report = run_ngspice(&rc_board(), &config)
        .unwrap_or_else(|| panic!("ngspice at {} failed to produce a report", exe.display()));
    assert!(report.model.contains("ngspice"), "{}", report.model);
    assert!(report.points.len() > 10);
    let measured = report.bandwidth_hz.expect("−3 dB point");
    let expected = 1.0 / (std::f64::consts::TAU * 10_000.0 * 100e-9);
    assert!(
        (measured / expected - 1.0).abs() < 0.15,
        "ngspice cutoff {measured:.1} Hz, analytic {expected:.1} Hz"
    );
}

#[test]
fn ngspice_inverting_opamp_gain_matches_resistor_ratio() {
    let Some(_exe) = find_ngspice() else {
        eprintln!("skip: set AMPROUTE_NGSPICE to run the golden SPICE case");
        return;
    };
    let mut b = Board::new("opamp-golden", 50.0, 30.0, LutPreset::reliable());
    for (r, value, x) in [("R1", "10k", 5.0), ("R2", "100k", 20.0)] {
        b.add_footprint(
            Footprint::new(r, value, NetClass::Signal, x, 10.0, 10.0, 4.0).with_pads(vec![
                Pad::new(1.0, 2.0, 1.6, 0.8),
                Pad::new(9.0, 2.0, 1.6, 0.8),
            ]),
        );
    }
    b.add_footprint(
        Footprint::new("U1", "LM358", NetClass::Signal, 35.0, 10.0, 12.0, 10.0).with_pads(vec![
            Pad::new(2.0, 5.0, 1.6, 0.8),
            Pad::new(6.0, 5.0, 1.6, 0.8),
            Pad::new(10.0, 5.0, 1.6, 0.8),
        ]),
    );
    b.add_net(Net::new("IN", NetClass::Input, vec![PadRef::new("R1", 0)]));
    b.add_net(Net::new(
        "MID",
        NetClass::Signal,
        vec![PadRef::new("R1", 1), PadRef::new("R2", 0), PadRef::new("U1", 1)],
    ));
    b.add_net(Net::new(
        "OUT",
        NetClass::Output,
        vec![PadRef::new("R2", 1), PadRef::new("U1", 2)],
    ));
    b.add_net(Net::new("GND", NetClass::Ground, vec![PadRef::new("U1", 0)]));

    let config = AcConfig {
        models: HashMap::from([(
            "U1".to_string(),
            AcModel::OpAmp {
                plus_pin: 0,
                minus_pin: 1,
                out_pin: 2,
                gain: 1e5,
                input_impedance_ohm: 1e9,
            },
        )]),
        input_net: Some("IN".into()),
        output_net: Some("OUT".into()),
        f_start_hz: 100.0,
        f_end_hz: 100.0,
        points: 2,
        ..Default::default()
    };
    eprintln!("DECK:\n{}", generate_netlist(&b, &config));
    let report = run_ngspice(&b, &config).expect("ngspice AC report");
    let gain = 10.0_f64.powf(report.dc_gain_db / 20.0);
    assert!(
        (gain - 10.0).abs() < 1.0,
        "measured gain {gain:.2}, expected 10 (100k/10k)"
    );
}
