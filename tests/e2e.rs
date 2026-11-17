//! End-to-end checks: build the demo board, route it, validate it, export it.

use amproute_studio::placement::PlacementResult;
use amproute_studio::router::Router;
use amproute_studio::{export, Board};

fn routed_demo() -> (Board, amproute_studio::router::RoutingResult) {
    let board = PlacementResult::auto_amplifier().board;
    let mut router = Router::new(0.5, board.width_mm, board.height_mm);
    for (x, y, r) in board.thermal_hotspots() {
        router.add_penalty_circle(x, y, r);
    }
    if let Some((x, y)) = board.star_ground_point() {
        router.add_penalty_circle(x, y, 8.0);
    }
    let result = router.route_board(&board);
    (board, result)
}

#[test]
fn demo_board_is_valid_and_fully_routed() {
    let (board, result) = routed_demo();

    let issues = board.validate();
    assert!(issues.is_empty(), "demo board must validate: {issues:?}");

    assert_eq!(
        result.failed_nets,
        Vec::<String>::new(),
        "every demo net must route"
    );
    assert_eq!(result.routed_nets.len(), board.nets.len());
    assert!(result.total_length_mm > 0.0);
}

#[test]
fn export_produces_four_non_empty_files() {
    let (board, result) = routed_demo();
    let dir = std::env::temp_dir().join("amproute_e2e");
    let _ = std::fs::remove_dir_all(&dir);

    let files =
        export::write_all(&board, &result, dir.to_str().unwrap()).expect("export should succeed");
    assert_eq!(files.len(), 4);

    let lut = std::fs::read(dir.join("lut_print.pdf")).unwrap();
    assert!(lut.starts_with(b"%PDF"), "LUT template must be a PDF");
    assert!(lut.len() > 2000, "LUT template looks empty");

    let gerber = std::fs::read_to_string(dir.join("board.gbr")).unwrap();
    assert!(gerber.contains("%FSLAX46Y46*%"));
    assert!(gerber.contains("%MOMM*%"));
    assert!(gerber.contains("M02*"));
}

#[test]
fn board_json_roundtrip_preserves_geometry() {
    let board = PlacementResult::auto_amplifier().board;
    let json = board.to_json().expect("serialize");
    let back = Board::from_json(&json).expect("deserialize");

    assert_eq!(back.width_mm, board.width_mm);
    assert_eq!(back.height_mm, board.height_mm);
    assert_eq!(back.footprints.len(), board.footprints.len());
    assert_eq!(back.nets.len(), board.nets.len());
    assert!(back.validate().is_empty());
}

#[test]
fn broken_json_board_is_reported_not_ignored() {
    // A net pointing at a pad that does not exist must be caught by validation.
    let mut board = PlacementResult::auto_amplifier().board;
    board.nets[0].nodes[0].refdes = "GHOST".to_string();
    let issues = board.validate();
    assert!(
        issues.iter().any(|i| i.contains("GHOST")),
        "dangling reference must be reported: {issues:?}"
    );
}
