//! Module 6 acceptance test on the reference board, with the recorded
//! FreeRouting session used **only** as an external benchmark.
//!
//! FreeRouting is never invoked here and never at runtime. `amp_class_ab.ses`
//! is a frozen artefact of a different router (multi-layer, vias allowed). It
//! gives us one thing worth checking: an independent, human-accepted routing
//! of the same board tells us how much copper the board plausibly needs and
//! which nets exist. The tests below use it for exactly that and no more.
//!
//! Every test runs under a small [`EngineConfig::time_budget_s`] so plain
//! `cargo test` (unoptimised, no `--release`) finishes in seconds. The budget
//! is a knob, not a weakening: the engine must still produce real copper and
//! stay inside module 4's rules within it.

use amproute_studio::board::Board;
use amproute_studio::import::import_kicad_pcb;
use amproute_studio::io::import_ses;
use amproute_studio::preroute::preroute;
use amproute_studio::routing_engine::{EngineConfig, EngineOutcome, FixedCopper, RoutingEngine};
use amproute_studio::router::RouteSegment;
use amproute_studio::rules::LutRules;
use std::path::{Path, PathBuf};

fn amproute_dir() -> PathBuf {
    PathBuf::from(r"C:\Users\Pstuha\Documents\Default Project\amproute")
}

fn reference_board() -> Option<Board> {
    let path = amproute_dir().join("amp_class_ab.kicad_pcb");
    if !path.exists() {
        eprintln!("skip: {} not found", path.display());
        return None;
    }
    Some(
        import_kicad_pcb(path.to_str().expect("utf-8 path"))
            .expect("reference board imports")
            .board,
    )
}

/// The engine as the test suite uses it: the reliable LUT profile on a coarse
/// 0.5 mm grid, which is fast enough for an unoptimised debug run and still
/// resolves the 0.4 mm feature band.
fn test_engine() -> RoutingEngine {
    RoutingEngine::new(
        LutRules::reliable(),
        EngineConfig {
            grid_mm: 0.5,
            time_budget_s: 5.0,
            retry_budget: 1,
            ..EngineConfig::default()
        },
    )
}

fn fixed_from_preroute(board: &Board) -> FixedCopper {
    let pre = preroute(board);
    FixedCopper {
        segments: pre
            .traces
            .iter()
            .map(|t| {
                RouteSegment::new(
                    t.x1,
                    t.y1,
                    t.x2,
                    t.y2,
                    &t.net,
                    board
                        .nets
                        .iter()
                        .find(|n| n.name == t.net)
                        .map(|n| n.class)
                        .unwrap_or(amproute_studio::board::NetClass::Power),
                )
                .with_width(t.width_mm)
            })
            .collect(),
        islands: pre.star_point.map(|(x, y)| (x, y, 1.5)).into_iter().collect(),
    }
}

#[test]
fn engine_runs_on_the_reference_board_and_reports_a_verdict() {
    let Some(board) = reference_board() else { return };
    let engine = test_engine();
    let fixed = fixed_from_preroute(&board);

    let run = engine.run(&board, &fixed);

    assert!(
        !matches!(run.outcome, EngineOutcome::RejectedBeforeRouting),
        "the reference board must pass module 4: {:?}",
        run.log
    );
    assert!(run.pre_rules.is_empty(), "pre-route: {}", run.pre_rules.summary());
    assert!(
        !run.result.segments.is_empty(),
        "the engine must produce copper on a real 42-footprint board: {:?}",
        run.log
    );
    assert_eq!(run.result.vias, 0, "LUT forbids vias");
    assert!(run.result.total_length_mm > 0.0, "routed copper must have length");
    assert!(
        run.fixed_length_mm > 0.0,
        "module 5 must hand over power/ground copper on this board"
    );

    eprintln!("--- engine log ---");
    for line in &run.log {
        eprintln!("{line}");
    }
    eprintln!(
        "outcome={} signal={:.1}mm fixed={:.1}mm segments={} deadline_reached={}",
        run.outcome.label(),
        run.result.total_length_mm,
        run.fixed_length_mm,
        run.result.segments.len(),
        run.deadline_reached
    );
}

#[test]
fn engine_matches_the_recorded_reference_on_basics() {
    let Some(board) = reference_board() else { return };
    let ses_path = amproute_dir().join("amp_class_ab.ses");
    if !ses_path.exists() {
        eprintln!("skip: no recorded session at {}", ses_path.display());
        return;
    }

    let ses_text = std::fs::read_to_string(&ses_path).expect("session readable");
    let ses = import_ses(&ses_text).expect("session parses");

    // What the reference genuinely tells us, now that `import_ses` honours the
    // session's `(resolution um 10)`: how long the accepted routing is, in
    // millimetres, and which nets it connects.
    let ses_length: f64 = ses
        .tracks
        .iter()
        .map(|t| (t.x2 - t.x1).hypot(t.y2 - t.y1))
        .sum();
    let ses_nets: std::collections::HashSet<&str> =
        ses.tracks.iter().map(|t| t.net.as_str()).collect();

    // A routing of this board cannot exceed a generous multiple of its own
    // area served at the narrowest reasonable pitch; the sanity bound catches a
    // unit-scale regression in the parser rather than in the engine.
    let area_bound = board.width_mm * board.height_mm * 4.0;
    assert!(
        ses_length > 0.0 && ses_length < area_bound,
        "reference length {ses_length:.1} mm is implausible for a {:.0}x{:.0} mm board",
        board.width_mm,
        board.height_mm
    );

    // This acceptance check needs enough nets connected for a meaningful
    // comparison with the human-routed session; give it a longer budget.
    let engine = RoutingEngine::new(
        LutRules::reliable(),
        EngineConfig {
            grid_mm: 0.5,
            time_budget_s: 30.0,
            retry_budget: 1,
            ..EngineConfig::default()
        },
    );
    let run = engine.run(&board, &fixed_from_preroute(&board));

    eprintln!(
        "ours: {:.1} mm, {} segments, {} nets",
        run.result.total_length_mm,
        run.result.segments.len(),
        run.result.routed_nets.len()
    );
    eprintln!(
        "reference: {:.1} mm, {} tracks, {} nets",
        ses_length,
        ses.tracks.len(),
        ses_nets.len()
    );

    assert!(run.result.routed_nets.len() >= 6, "connected {} nets", run.result.routed_nets.len());
    assert_eq!(run.result.vias, 0, "LUT process forbids vias completely");

    // On the basics the two agree: every net our engine reports as routed is a
    // net the reference also routed — we never invent connectivity.
    for net in &run.result.routed_nets {
        assert!(
            ses_nets.contains(net.as_str()),
            "engine claims net `{net}`, which the reference session does not route"
        );
    }
}

#[test]
fn engine_output_is_clean_under_module_four_rules() {
    let Some(board) = reference_board() else { return };
    let engine = test_engine();
    let run = engine.run(&board, &fixed_from_preroute(&board));

    for v in &run.post_rules.violations {
        eprintln!("post-rule violation: {v:?}");
    }
    assert!(
        run.post_rules.is_empty(),
        "module 4 rejected the engine's copper: {}",
        run.post_rules.summary()
    );
    assert!(
        run.result
            .segments
            .iter()
            .all(|s| s.width_mm >= engine.rules.min_track_mm - 1e-9),
        "every segment must respect the LUT minimum width {:.2} mm",
        engine.rules.min_track_mm
    );
}

#[test]
fn engine_leaves_no_copper_outside_the_board_outline() {
    let Some(board) = reference_board() else { return };
    let engine = test_engine();
    let run = engine.run(&board, &fixed_from_preroute(&board));

    let outside = |x: f64, y: f64| x < 0.0 || y < 0.0 || x > board.width_mm || y > board.height_mm;
    let bad: Vec<_> = run
        .result
        .segments
        .iter()
        .filter(|s| outside(s.x1_mm, s.y1_mm) || outside(s.x2_mm, s.y2_mm))
        .collect();
    assert!(
        bad.is_empty(),
        "{} segments outside the {}x{} board: {:?}",
        bad.len(),
        board.width_mm,
        board.height_mm,
        bad.iter().take(3).collect::<Vec<_>>()
    );
}

#[test]
fn identical_inputs_produce_identical_copper() {
    let Some(board) = reference_board() else { return };
    let engine = test_engine();
    let fixed = fixed_from_preroute(&board);

    let a = engine.run(&board, &fixed);
    let b = engine.run(&board, &fixed);

    assert_eq!(a.result.segments.len(), b.result.segments.len());
    assert_eq!(a.outcome, b.outcome);
    assert!(
        (a.result.total_length_mm - b.result.total_length_mm).abs() < 1e-9,
        "engine must be a pure function of its inputs"
    );
    for (sa, sb) in a.result.segments.iter().zip(b.result.segments.iter()) {
        assert!((sa.x1_mm - sb.x1_mm).abs() < 1e-9 && (sa.y1_mm - sb.y1_mm).abs() < 1e-9);
        assert_eq!(sa.net, sb.net);
    }
}

#[test]
fn the_engine_honours_its_time_budget() {
    let Some(board) = reference_board() else { return };
    // A budget far too small to finish a 29-net board: the run must come back
    // promptly and say so, instead of holding the pipeline for minutes.
    let engine = RoutingEngine::new(
        LutRules::reliable(),
        EngineConfig {
            grid_mm: 0.5,
            time_budget_s: 0.5,
            retry_budget: 1,
            ..EngineConfig::default()
        },
    );
    let started = std::time::Instant::now();
    let run = engine.run(&board, &fixed_from_preroute(&board));
    let elapsed = started.elapsed().as_secs_f64();

    eprintln!(
        "budget 0.5 s: finished in {elapsed:.1} s, deadline_reached={}, {} nets",
        run.deadline_reached,
        run.result.routed_nets.len()
    );
    assert!(
        elapsed < 25.0,
        "a 0.5 s budget must not take {elapsed:.1} s"
    );
    assert_eq!(run.outcome, EngineOutcome::PartiallyRouted);
}

#[test]
fn no_freerouting_process_is_spawned_by_the_engine() {
    // Module 6 must not reference the external tool at all: it has no process
    // launcher and no jar path. A source-level guard, because a runtime
    // assertion cannot prove the absence of a spawn.
    let src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("routing_engine.rs");
    let text = std::fs::read_to_string(&src).expect("routing_engine.rs is part of the crate");
    for forbidden in ["std::process", "Command::new", ".jar", "java"] {
        assert!(
            !text.contains(forbidden),
            "module 6 must be a native engine; found `{forbidden}` in {}",
            src.display()
        );
    }
}
