//! Acceptance: every built-in board must route completely, pass our DRC and
//! write the whole LUT deliverable set with the default settings.

use amproute_studio::boards::BOARD_SPECS;
use amproute_studio::pipeline::{run_pipeline, PipelineConfig};

#[test]
fn all_five_boards_route_100_percent_with_clean_drc() {
    for spec in BOARD_SPECS {
        let tmp = tempfile::tempdir().unwrap();
        let config = PipelineConfig {
            grid_mm: 0.5,
            time_budget_s: 180.0,
            ..Default::default()
        };
        let result = run_pipeline((spec.build)(), config, tmp.path().to_str().unwrap())
            .unwrap_or_else(|e| panic!("{}: pipeline failed: {e}", spec.id));
        assert!(
            result.routing.result.failed_nets.is_empty(),
            "{}: failed nets {:?}",
            spec.id,
            result.routing.result.failed_nets
        );
        assert_eq!(
            result.routing.result.routed_nets.len(),
            result.nets,
            "{}: only {}/{} nets routed",
            spec.id,
            result.routing.result.routed_nets.len(),
            result.nets
        );
        assert!(
            result.drc.is_ok(),
            "{}: DRC violations {:?}",
            spec.id,
            result.drc.violations
        );
        assert!(result.ok, "{}: pipeline result is not ok", spec.id);
        assert_eq!(result.exported_files.len(), 4, "{}: export set", spec.id);
        assert!(result.kicad_board_path.is_some(), "{}: KiCad board", spec.id);
    }
}
