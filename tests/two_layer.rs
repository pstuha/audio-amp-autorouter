//! Two-layer deliverable: both copper layers and plated vias survive export
//! and are accepted by our own audit.

use amproute_studio::board::{Board, Footprint, LutPreset, Net, NetClass, Pad, PadRef};
use amproute_studio::drc::own_audit;
use amproute_studio::export::write_all;
use amproute_studio::router::{CopperLayer, RouteSegment, RoutingResult, Via};

fn board_with_via() -> Board {
    let mut board = Board::new("via_board", 30.0, 20.0, LutPreset::reliable()).with_two_layer();
    board.add_footprint(
        Footprint::new("A", "test", NetClass::Signal, 4.0, 9.0, 4.0, 2.0)
            .with_pads(vec![Pad::new(2.0, 1.0, 1.6, 0.8)]),
    );
    board.add_footprint(
        Footprint::new("B", "test", NetClass::Signal, 24.0, 9.0, 4.0, 2.0)
            .with_pads(vec![Pad::new(2.0, 1.0, 1.6, 0.8)]),
    );
    board.add_net(Net::new(
        "SIG",
        NetClass::Signal,
        vec![PadRef::new("A", 0), PadRef::new("B", 0)],
    ));
    board
}

fn routed() -> RoutingResult {
    RoutingResult {
        segments: vec![
            RouteSegment::new(6.0, 10.0, 12.0, 10.0, "SIG", NetClass::Signal),
            RouteSegment::new(18.0, 10.0, 24.0, 10.0, "SIG", NetClass::Signal)
                .on_layer(CopperLayer::BCu),
        ],
        routed_nets: vec!["SIG".into()],
        via_objects: vec![Via {
            x_mm: 15.0,
            y_mm: 10.0,
            net: "SIG".into(),
            class: NetClass::Signal,
            diameter_mm: 1.6,
            drill_mm: 0.8,
        }],
        vias: 1,
        ..Default::default()
    }
}

#[test]
fn two_layer_export_writes_both_gerbers_and_the_via() {
    let board = board_with_via();
    let routing = routed();
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_str().unwrap();
    let files = write_all(&board, &routing, dir).unwrap();
    assert_eq!(files.len(), 7, "front + back Gerber, SVG, PDF, LUT x2, drill: {files:?}");

    let back = std::fs::read_to_string(format!("{dir}/board_b.gbr")).unwrap();
    assert!(back.contains("B.Cu"));
    assert!(back.contains("D03*"), "vias are flashed on B.Cu");

    let front = std::fs::read_to_string(format!("{dir}/board.gbr")).unwrap();
    assert!(front.contains("D03*"), "vias are flashed on F.Cu too");
    assert!(std::path::Path::new(&format!("{dir}/lut_print_b.pdf")).exists());
}

#[test]
fn two_layer_kicad_board_has_via_and_back_copper() {
    let board = board_with_via();
    let text = amproute_studio::kicad::to_kicad_pcb(&board, &routed(), &Default::default());
    assert!(text.contains("(via (at 15.0000 10.0000)"));
    assert!(text.contains("(layers \"F.Cu\" \"B.Cu\")"));
    assert!(text.contains("(layer \"B.Cu\")"));
}

#[test]
fn own_audit_accepts_a_two_layer_via() {
    let board = board_with_via();
    let report = own_audit(&board, Some(&routed()));
    assert!(report.is_ok(), "{:?}", report.violations);
}
