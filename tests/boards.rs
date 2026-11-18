//! The five built-in boards must be valid *and* fully routable.
//!
//! These are the acceptance tests for the shipped examples: if a board cannot
//! be routed, or its model is inconsistent, the test says which one and why.

use amproute_studio::boards::{self, BOARD_SPECS};
use amproute_studio::export;
use amproute_studio::kicad;
use amproute_studio::thermal;
use amproute_studio::Router;

/// Router grid used for every board. Finer than the LUT pitch, coarse enough to
/// finish quickly.
const GRID_MM: f64 = 0.5;

fn routed(board: &amproute_studio::Board) -> amproute_studio::RoutingResult {
    let mut router = Router::new(GRID_MM, board.width_mm, board.height_mm);
    for (x, y, r) in board.thermal_hotspots() {
        router.add_penalty_circle(x, y, r);
    }
    if let Some((x, y)) = board.star_ground_point() {
        router.add_penalty_circle(x, y, 8.0);
    }
    router.route_board(board)
}

#[test]
fn there_are_five_distinct_boards() {
    assert_eq!(BOARD_SPECS.len(), 5);
    assert_eq!(boards::all_boards().len(), 5);

    let mut ids: Vec<&str> = BOARD_SPECS.iter().map(|s| s.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 5, "board ids must be unique");

    // Different sizes prove the router is not tuned to one geometry.
    let mut sizes: Vec<(u32, u32)> = boards::all_boards()
        .iter()
        .map(|b| (b.width_mm as u32, b.height_mm as u32))
        .collect();
    sizes.sort_unstable();
    sizes.dedup();
    assert_eq!(sizes.len(), 5, "all five boards should differ in size");
}

#[test]
fn every_board_is_structurally_valid() {
    for spec in BOARD_SPECS {
        let board = (spec.build)();
        let issues = board.validate();
        assert!(
            issues.is_empty(),
            "{} ({}) failed validation: {issues:#?}",
            board.name,
            spec.id
        );
    }
}

#[test]
fn every_board_has_mounting_holes_clear_of_copper() {
    for spec in BOARD_SPECS {
        let board = (spec.build)();
        assert!(
            !board.mounting_holes.is_empty(),
            "{} has no mounting holes",
            board.name
        );
        for (i, hole) in board.mounting_holes.iter().enumerate() {
            let clearance = hole.copper_keepout_mm();
            for fp in &board.footprints {
                for (pi, pad) in fp.pads.iter().enumerate() {
                    let (px, py) = (fp.x + pad.x, fp.y + pad.y);
                    let d = ((px - hole.x).powi(2) + (py - hole.y).powi(2)).sqrt();
                    assert!(
                        d - pad.size_mm / 2.0 >= clearance - 1e-6,
                        "{} H{}: pad {}.{} is {:.2} mm from the hole, needs {:.2} mm",
                        board.name,
                        i + 1,
                        fp.refdes,
                        pi,
                        d - pad.size_mm / 2.0,
                        clearance
                    );
                }
            }
        }
    }
}

#[test]
fn every_board_has_the_expected_stages() {
    for spec in BOARD_SPECS {
        let board = (spec.build)();
        // A left-channel amplifier needs an input, an output, power and ground.
        for (class, what) in [
            (amproute_studio::NetClass::Input, "input"),
            (amproute_studio::NetClass::Output, "output"),
            (amproute_studio::NetClass::Power, "power"),
            (amproute_studio::NetClass::Ground, "ground"),
        ] {
            assert!(
                board.nets.iter().any(|n| n.class == class),
                "{} has no {what} net",
                board.name
            );
        }
        assert!(
            board.routable_nets() >= 6,
            "{} only has {} routable nets",
            board.name,
            board.routable_nets()
        );
        assert!(
            board.footprints.len() >= 6,
            "{} only has {} components",
            board.name,
            board.footprints.len()
        );
    }
}

#[test]
fn every_board_routes_completely() {
    let mut report = Vec::new();
    for spec in BOARD_SPECS {
        let board = (spec.build)();
        let result = routed(&board);
        if !result.failed_nets.is_empty() {
            report.push(format!(
                "{} ({}): {}/{} nets routed, failures: {:?}",
                board.name,
                spec.id,
                result.routed_nets.len(),
                result.total_nets(),
                result.failed_nets
            ));
        }
        assert!(
            result.segments.iter().all(|s| s.length_mm() > 0.0),
            "{} produced a zero-length track",
            board.name
        );
    }
    assert!(report.is_empty(), "boards failed to route:\n{}", report.join("\n"));
}

#[test]
fn every_board_exports_gerber_and_kicad() {
    let dir = std::env::temp_dir().join("amprs_boards");
    std::fs::create_dir_all(&dir).unwrap();

    for spec in BOARD_SPECS {
        let board = (spec.build)();
        let result = routed(&board);

        let out = dir.join(spec.id);
        std::fs::create_dir_all(&out).unwrap();
        let files = export::write_all(&board, &result, out.to_str().unwrap())
            .unwrap_or_else(|e| panic!("{} export failed: {e}", board.name));
        assert_eq!(files.len(), 4, "{} should write four files", board.name);

        let pcb = out.join("board.kicad_pcb");
        kicad::write_kicad_pcb(
            &board,
            &result,
            &kicad::FabOptions::default(),
            pcb.to_str().unwrap(),
        )
        .unwrap_or_else(|e| panic!("{} kicad export failed: {e}", board.name));

        let text = std::fs::read_to_string(&pcb).unwrap();
        assert!(text.starts_with("(kicad_pcb"), "{} is not a KiCad board", board.name);
        assert!(
            text.contains("Edge.Cuts"),
            "{} has no board outline",
            board.name
        );
        // Every mounting hole must survive the round trip.
        assert_eq!(
            text.matches("np_thru_hole").count(),
            board.mounting_holes.len(),
            "{} lost mounting holes in the KiCad export",
            board.name
        );
    }
}

#[test]
fn kicad_export_can_be_read_back() {
    // A board written here must be readable by our own importer: that proves the
    // file is not just text that happens to look right.
    for spec in BOARD_SPECS {
        let board = (spec.build)();
        let result = routed(&board);
        let text = kicad::to_kicad_pcb(&board, &result, &kicad::FabOptions::default());

        let parsed = amproute_studio::sexpr::parse(&text)
            .unwrap_or_else(|e| panic!("{} export does not parse: {e}", board.name));
        assert_eq!(parsed.kind(), Some("kicad_pcb"));

        let imported = amproute_studio::import_kicad_pcb_str(&text)
            .unwrap_or_else(|e| panic!("{} cannot be re-imported: {e}", board.name));
        assert_eq!(
            imported.board.footprints.len(),
            board.footprints.len(),
            "{} lost footprints on re-import",
            board.name
        );
        assert!(
            (imported.board.width_mm - board.width_mm).abs() < 0.01,
            "{} width changed from {:.2} to {:.2} on re-import",
            board.name,
            board.width_mm,
            imported.board.width_mm
        );
    }
}

#[test]
fn thermal_simulation_finds_the_power_devices() {
    for spec in BOARD_SPECS {
        let board = (spec.build)();
        let sources = thermal::sources_for(&board);
        assert!(
            !sources.is_empty(),
            "{} has no heat sources — the thermal overlay would be empty",
            board.name
        );

        let field = thermal::solve(&board, &sources, &thermal::ThermalParams::default());
        assert!(
            field.peak_rise_c > 0.5,
            "{} peak rise {:.3} °C is too low to display",
            board.name,
            field.peak_rise_c
        );
        // The hottest point must lie on the board, not float in space.
        assert!(
            field.peak_at.0 >= 0.0
                && field.peak_at.0 <= board.width_mm
                && field.peak_at.1 >= 0.0
                && field.peak_at.1 <= board.height_mm,
            "{} peak at {:?} is outside the board",
            board.name,
            field.peak_at
        );
        // It must be near a real heat source, not decoration in a corner.
        let nearest = sources
            .iter()
            .map(|s| {
                ((s.x - field.peak_at.0).powi(2) + (s.y - field.peak_at.1).powi(2)).sqrt()
            })
            .fold(f64::MAX, f64::min);
        assert!(
            nearest < 20.0,
            "{} hottest point is {nearest:.1} mm from any component",
            board.name
        );
    }
}

#[test]
fn board_lookup_by_id_works() {
    assert!(boards::build("lm386-mini").is_some());
    assert!(boards::build("LM386-MINI").is_some(), "lookup should ignore case");
    assert!(boards::build("nope").is_none());
    assert_eq!(boards::default_board().name, "Classic LTP");
}
