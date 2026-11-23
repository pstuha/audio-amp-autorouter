//! Module 10 — Integration Pipeline: end-to-end board processing.
//!
//! Composes modules 1–9 into a single, testable pipeline:
//!
//! 1. **Load** — import `.kicad_pcb` or JSON (module 2/import).
//! 2. **Validate** — module 4 rules gate (`LutRules::check_board`).
//! 3. **Pre-route** — module 5 fixed power/ground copper (`preroute`).
//! 4. **Route** — module 6 routing engine with thermal/star penalties.
//! 5. **Jumpers** — module 7 insulated wire instead of vias.
//! 6. **Measure** — module 8 DC drop, loss, width-by-current.
//! 7. **DRC** — module 9 own audit (+ KiCad CLI DRC when available).
//! 8. **Export** — Gerber, SVG, PDF, LUT template, KiCad board (module 4/export/kicad).
//!
//! The pipeline never mutates the input board; every stage returns new data
//! that the next stage consumes. A failure at any stage is reported
//! structurally so callers (CLI, GUI, CI) can react.

use crate::ac::{AcConfig, AcReport};
use crate::board::{Board, NetClass};
use crate::emi::{EmiConfig, EmiReport};
use crate::drc::{audit, DrcReport};
use crate::export::write_all;
use crate::import::import_kicad_pcb_str_named;
use crate::jumper::{BreakRequest, JumperConfig, JumperHandler, JumperResult};
use crate::kicad::{write_kicad_pcb, FabOptions};
use crate::measure::{Measure, MeasureConfig, MeasureRule, MeasureReport, NetCurrent, NetExcitation};
use crate::preroute::preroute;
use crate::router::{RouteSegment, RoutingResult};
use crate::routing_engine::{EngineConfig, EngineRun, RoutingEngine};
use crate::rules::{LutRules, RuleReport};
use crate::thermal::{
    solve, solve_transient, sources_for, ThermalField, ThermalParams, TransientThermal,
};
use anyhow::{Context, Result};
use serde::Serialize;
use std::path::Path;

/// Result of the full pipeline run.
#[derive(Debug, Serialize)]
pub struct PipelineResult {
    /// Original board name.
    pub board_name: String,
    /// Board dimensions (mm).
    pub width_mm: f64,
    pub height_mm: f64,
    /// Footprints / nets / mounting holes counts.
    pub footprints: usize,
    pub nets: usize,
    pub mounting_holes: usize,
    /// Module 4 gate before routing.
    pub pre_rules: RuleReport,
    /// Module 5 fixed copper length (mm).
    pub fixed_copper_mm: f64,
    /// Module 6 routing outcome.
    pub routing: EngineRun,
    /// All copper the board is exported with: fixed power/ground rails plus
    /// the router's and jumpers' tracks. This is what the viewer draws.
    pub final_copper: RoutingResult,
    /// Module 7 jumpers inserted (None when routing completed cleanly).
    pub jumpers: Option<JumperResult>,
    /// Insulated-wire jumpers planned for nets with no copper at all.
    pub planned_jumpers: Vec<crate::jumper::Jumper>,
    /// Module 8 measurements on final copper.
    pub measurements: Option<MeasureReport>,
    /// AC analysis result when the agent supplied models.
    pub ac: Option<AcReport>,
    /// EMI geometry report when requested.
    pub emi: Option<EmiReport>,
    /// Electrical/thermal quality verdict, separate from manufacturability.
    pub quality: QualityReport,
    /// Module 9 DRC (own audit + KiCad CLI if available).
    pub drc: DrcReport,
    /// Files written by the export stage.
    pub exported_files: Vec<String>,
    /// KiCad board export path, if written.
    pub kicad_board_path: Option<String>,
    /// Thermal simulation field (peak rise, mean, sources).
    pub thermal: ThermalField,
    /// Transient junction estimate from the delivered copper.
    pub transient: Option<TransientThermal>,
    /// Total wall-clock time (ms).
    pub elapsed_ms: u64,
    /// Overall success: everything routed, no DRC errors, export ok.
    pub ok: bool,
}

/// Version of the machine-readable quality contract.
pub const QUALITY_SCHEMA_VERSION: &str = "1.0";

/// Quality verdict computed from the electrical/thermal measurements.
///
/// Unlike `ok` (routing + DRC + export), this is not a manufacturing gate by
/// default: currents are design assumptions when the caller supplies none, so
/// failing them would reject a perfectly routeable board on guessed currents.
/// An agent that provides real currents gets a hard quality verdict.
#[derive(Debug, Default, Serialize)]
pub struct QualityReport {
    /// True when the currents came from class defaults, not from the caller.
    pub assumed_currents: bool,
    pub electrical_ok: bool,
    pub thermal_ok: bool,
    /// Pass when the requested AC analysis is stable and has finite gain.
    pub ac_ok: bool,
    /// Pass when the requested EMI screens are clean (set by stage 3).
    pub emi_ok: bool,
    pub violations: Vec<String>,
    pub schema_version: String,
    /// `proxy` for built-in models, `spice` for an external run.
    pub model: String,
    pub confidence: String,
    pub assumptions: Vec<String>,
    pub summary: String,
}

/// Configuration for the pipeline.
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    /// Routing grid (mm). 0.5 mm is the default used for the five test boards.
    pub grid_mm: f64,
    /// Routing engine time budget (seconds). 0 = no budget.
    pub time_budget_s: f64,
    /// LUT rules profile. `None` derives them from the board's own preset —
    /// the right default, because a board built for the `Advanced` preset is
    /// legal under module 4 and must not be rejected by a stricter profile.
    pub rules: Option<LutRules>,
    /// Jumper configuration (module 7).
    pub jumper_config: JumperConfig,
    /// Measurement configuration (module 8).
    pub measure_config: MeasureConfig,
    /// Per-net currents for the DC measurement. When `None`, conservative
    /// currents are derived from each net's class.
    pub measure_currents: Option<Vec<NetCurrent>>,
    /// AC analysis configuration. `None` skips AC entirely.
    pub ac: Option<AcConfig>,
    /// Prefer ngspice when installed; otherwise use the built-in proxy solver.
    pub prefer_ngspice: bool,
    /// EMI geometry screens. `None` skips them.
    pub emi: Option<EmiConfig>,
    /// Add a transient thermal step-response estimate.
    pub include_transient: bool,
    /// Which quality verdicts block `ok`. Empty means all supplied analyses.
    pub analysis_policy: Vec<String>,
    /// Compare a 45°/octile search against the stable Manhattan search.
    pub diagonal_search: bool,
    /// Rich agent-supplied excitations (spectrum, return net). When present,
    /// these take precedence over `measure_currents`.
    pub excitations: Option<Vec<NetExcitation>>,
    /// Thermal simulation parameters.
    pub thermal_params: ThermalParams,
    /// Whether to run KiCad CLI DRC (requires `kicad-cli` on PATH).
    pub run_kicad_cli_drc: bool,
    /// KiCad fabrication options.
    pub fab_options: FabOptions,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            grid_mm: 0.5,
            time_budget_s: 120.0,
            rules: None,
            jumper_config: JumperConfig::default(),
            measure_config: MeasureConfig::default(),
            measure_currents: None,
            excitations: None,
            ac: None,
            prefer_ngspice: false,
            emi: None,
            include_transient: false,
            analysis_policy: Vec::new(),
            diagonal_search: true,
            thermal_params: ThermalParams::default(),
            run_kicad_cli_drc: false,
            fab_options: FabOptions::default(),
        }
    }
}

/// Rules that match the board's own manufacturing preset.
///
/// The pipeline must not impose a stricter profile than the board itself
/// declares: a board built for the `Advanced` preset (0.3 mm copper, 0.2 mm
/// clearance) is legal under module 4, and the rules derived from its preset
/// accept it while still clamping every value into the manufacturable band.
fn rules_for_board(board: &Board) -> LutRules {
    LutRules::from_preset_for(&board.preset, board.two_layer)
}

/// Conservative per-class currents used when the caller supplies none.
/// They are design assumptions, not measurements; an explicit
/// [`PipelineConfig::measure_currents`] always takes precedence.
fn currents_for_board(board: &Board) -> Vec<NetCurrent> {
    board
        .nets
        .iter()
        .map(|net| {
            let rms = match net.class {
                NetClass::Power | NetClass::Ground => 1.0,
                NetClass::Output => 0.5,
                NetClass::Signal | NetClass::Feedback | NetClass::Control => 0.01,
                NetClass::Input | NetClass::Bias => 0.001,
            };
            NetCurrent::new(&net.name, rms).with_peak(rms * 2.0)
        })
        .collect()
}

/// Run the full pipeline on a board already in memory.
pub fn run_pipeline(board: Board, config: PipelineConfig, out_dir: &str) -> Result<PipelineResult> {
    let started = std::time::Instant::now();
    let board_name = board.name.clone();
    let rules = config.rules.clone().unwrap_or_else(|| rules_for_board(&board));
    std::fs::create_dir_all(out_dir)
        .with_context(|| format!("cannot create output directory {out_dir}"))?;

    // 1. Module 4 pre-route gate — reject unmanufacturable boards before
    //    spending any search time.
    let pre_rules = rules.check_board(&board);
    if !pre_rules.is_empty() {
        return Ok(PipelineResult {
            board_name: board.name,
            width_mm: board.width_mm,
            height_mm: board.height_mm,
            footprints: board.footprints.len(),
            nets: board.nets.len(),
            mounting_holes: board.mounting_holes.len(),
            pre_rules,
            fixed_copper_mm: 0.0,
            routing: EngineRun {
                outcome: crate::routing_engine::EngineOutcome::RejectedBeforeRouting,
                result: RoutingResult::new(),
                pre_rules: RuleReport::default(),
                post_rules: RuleReport::default(),
                fixed_length_mm: 0.0,
                deadline_reached: false,
                log: vec!["pre-route gate failed".into()],
            },
            final_copper: RoutingResult::new(),
            jumpers: None,
            planned_jumpers: Vec::new(),
            measurements: None,
            ac: None,
            emi: None,
            quality: QualityReport::default(),
            drc: DrcReport::default(),
            exported_files: vec![],
            kicad_board_path: None,
            transient: None,
            thermal: ThermalField {
                cols: 0,
                rows: 0,
                cell_mm: 0.0,
                temps: Vec::new(),
                sources: Vec::new(),
                ambient_c: 20.0,
                peak_rise_c: 0.0,
                peak_at: (0.0, 0.0),
            },
            elapsed_ms: started.elapsed().as_millis() as u64,
            ok: false,
        });
    }

    // 2. Module 5 pre-route — power/ground become immovable fixed copper.
    let pre = preroute(&board);
    let fixed = crate::routing_engine::FixedCopper {
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
                        .unwrap_or(NetClass::Power),
                )
                .with_width(t.width_mm)
            })
            .collect(),
        islands: pre
            .star_point
            .map(|(x, y)| (x, y, 1.5))
            .into_iter()
            .collect(),
    };
    let fixed_copper_mm = fixed.total_length_mm();

    // 3. Module 6 routing engine.
    let engine = RoutingEngine::new(
        rules.clone(),
        EngineConfig {
            grid_mm: config.grid_mm,
            time_budget_s: config.time_budget_s,
            retry_budget: 10,
            diagonal_search: config.diagonal_search,
            penalty_weight: 1.5,
        },
    );
    let routing = engine.run(&board, &fixed);

    // 4. Module 7 jumpers — single-layer boards only. A two-layer board has
    //    plated vias as the legitimate way across.
    let jumpers = if board.two_layer || routing.result.failed_nets.is_empty() {
        None
    } else {
        let mut requests: Vec<BreakRequest> = Vec::new();
        for failed in &routing.result.failed_nets {
            let net_name = failed.split(':').next().unwrap_or(failed).trim_start_matches('/');
            // Only break a segment that actually exists for this net.
            let seg_idx = routing
                .result
                .segments
                .iter()
                .position(|s| s.net == net_name);
            if let Some(seg_idx) = seg_idx {
                requests.push(BreakRequest::new(net_name, seg_idx, 0.5));
            }
        }
        if requests.is_empty() {
            None
        } else {
            let handler = JumperHandler::new(rules.clone(), config.jumper_config.clone());
            let jres = handler.apply(&board, &routing.result, &requests);
            Some(jres)
        }
    };

    // Final board and signal copper: jumper footprints and their copper when
    // jumpers were inserted, otherwise the original board and engine copper.
    let planned_jumpers: Vec<crate::jumper::Jumper> = if board.two_layer {
        Vec::new()
    } else {
        routing
            .result
            .failed_nets
            .iter()
            .filter_map(|failed| {
                let name = failed.split(':').next().unwrap_or(failed).trim();
                crate::jumper::bridge_failed_net(&board, name, &config.jumper_config)
            })
            .collect()
    };
    let jumper_board = if planned_jumpers.is_empty() {
        None
    } else {
        Some(crate::jumper::apply_jumpers(&board, &planned_jumpers))
    };
    let final_board = jumper_board
        .as_ref()
        .or_else(|| jumpers.as_ref().map(|j| &j.board))
        .unwrap_or(&board);
    let mut final_routing = jumpers
        .as_ref()
        .map(|j| j.routing.clone())
        .unwrap_or_else(|| routing.result.clone());

    // Two-layer completion: nets the single-layer engine could not finish are
    // searched again on a layered grid, with plated vias at layer changes.
    if board.two_layer {
        let two = crate::two_layer::route_unfinished(
            &board,
            &routing.result,
            &fixed.segments,
            &rules,
            config.grid_mm,
        );
        if !two.routed_nets.is_empty() {
            final_routing.segments.extend(two.segments.iter().cloned());
            final_routing.routed_nets.extend(two.routed_nets);
            final_routing.failed_nets = two
                .failed_nets
                .iter()
                .map(|n| format!("{n}: no two-layer path"))
                .collect();
            final_routing.via_objects.extend(two.vias);
            final_routing.vias = final_routing.via_objects.len();
            final_routing.total_length_mm =
                final_routing.segments.iter().map(|s| s.length_mm()).sum();
        }
    }

    // Deliverable copper is fixed power/ground plus signal/jumper/two-layer copper.
    // Auditing and exporting only the engine's share would hide shorts and
    // ship a board whose rails are missing.
    let mut complete = RoutingResult::new();
    // Module 5's star rails are only a routing bias: the engine routes every
    // net, including power/ground, so exporting both sets would duplicate
    // copper and invalidate clearance. `fixed_copper_mm` reports the bias.
    complete.segments.extend(final_routing.segments.iter().cloned());
    complete.routed_nets = final_routing.routed_nets.clone();
    for seg in &fixed.segments {
        if !complete.routed_nets.contains(&seg.net) {
            complete.routed_nets.push(seg.net.clone());
        }
    }
    complete.failed_nets = final_routing.failed_nets.clone();
    complete.via_objects = final_routing.via_objects.clone();
    complete.vias = final_routing.vias;
    complete.total_length_mm = complete.segments.iter().map(|s| s.length_mm()).sum();

    // Thermal solve (recorded for the report/GUI).
    let sources = sources_for(final_board);
    let thermal_field = solve(final_board, &sources, &config.thermal_params);
    let transient = if config.include_transient {
        Some(solve_transient(
            final_board,
            &sources,
            Some(&complete),
            &config.thermal_params,
            thermal_field.peak_rise_c,
        ))
    } else {
        None
    };

    // 5. Module 8 measurements on the complete copper.
    let currents = if let Some(excitations) = &config.excitations {
        excitations.iter().map(NetExcitation::to_current).collect()
    } else {
        config
            .measure_currents
            .clone()
            .unwrap_or_else(|| currents_for_board(&board))
    };
    let measurements = Some(
        Measure::new(config.measure_config.clone()).measure(final_board, &complete, &currents),
    );

    // AC analysis: always available through the built-in MNA proxy; an
    // installed ngspice can be opted into for higher confidence.
    let ac_report = config.ac.as_ref().map(|ac_config| {
        let proxy = crate::ac::analyze(final_board, ac_config);
        if config.prefer_ngspice {
            crate::spice::run_ngspice(final_board, ac_config).unwrap_or(proxy)
        } else {
            proxy
        }
    });

    let emi_report = config.emi.as_ref().map(|emi_config| {
        crate::emi::analyze(
            final_board,
            &complete,
            config.excitations.as_deref(),
            emi_config,
        )
    });

    let stem = final_board.name.replace(' ', "_").to_lowercase();
    let kicad_path = format!("{out_dir}/{stem}.kicad_pcb");

    // 6. Export — Gerber, SVG, PDF, LUT template, then the KiCad board.
    //    Only complete copper is safe to manufacture.
    let mut exported_files = if complete.failed_nets.is_empty() {
        match write_all(final_board, &complete, out_dir) {
            Ok(files) => files,
            Err(e) => {
                eprintln!("Export refused: {e}");
                vec![]
            }
        }
    } else {
        vec![]
    };
    if !planned_jumpers.is_empty() {
        let jumper_csv = format!("{out_dir}/jumpers.csv");
        let mut text = String::from("refdes,net,x1_mm,y1_mm,x2_mm,y2_mm,length_mm\n");
        for jumper in &planned_jumpers {
            text.push_str(&format!(
                "{},{},{:.4},{:.4},{:.4},{:.4},{:.4}\n",
                jumper.refdes,
                jumper.net,
                jumper.pad_a.0,
                jumper.pad_a.1,
                jumper.pad_b.0,
                jumper.pad_b.1,
                jumper.wire_length_mm
            ));
        }
        if std::fs::write(&jumper_csv, text).is_ok() {
            exported_files.push(jumper_csv);
        }
    }
    let kicad_board_path = match write_kicad_pcb(
        final_board,
        &complete,
        &config.fab_options,
        &kicad_path,
    ) {
        Ok(()) => Some(kicad_path.clone()),
        Err(e) => {
            eprintln!("KiCad export failed: {e}");
            None
        }
    };

    // 7. Module 9 DRC — runs after the board is on disk, so the external
    //    kicad-cli check sees the very copper we are about to deliver, and
    //    audits that copper rather than the engine's partial result.
    let mut drc = audit(final_board, Some(&complete), None);
    if let Some(emi) = &emi_report {
        for message in &emi.violations {
            drc.violations.push(crate::drc::DrcViolation {
                rule: "D04_EMI".into(),
                message: message.clone(),
                at: None,
                severity: Some("warning".into()),
            });
        }
    }
    if config.run_kicad_cli_drc && kicad_board_path.is_some() {
        match crate::drc::kicad_cli_drc(&kicad_path) {
            Ok(Some(external)) => drc = DrcReport::merge(drc, external),
            Err(e) => eprintln!("kicad-cli DRC failed: {e}"),
            _ => {}
        }
    }

    // Quality is reported separately from manufacturability. With assumed
    // currents it is advisory; with agent-supplied currents it becomes a hard
    // gate on `ok`, because then the numbers are real design constraints.
    let assumed_currents = config.measure_currents.is_none() && config.excitations.is_none();
    let electrical_ok = measurements.as_ref().is_none_or(|m| {
        m.violations
            .iter()
            .all(|v| v.rule == MeasureRule::Temperature)
    });
    let thermal_ok = measurements.as_ref().is_none_or(|m| {
        m.violations
            .iter()
            .all(|v| v.rule != MeasureRule::Temperature)
    }) && transient.as_ref().is_none_or(|t| {
        (t.peak_junction_c - config.thermal_params.ambient_c)
            <= config.measure_config.copper.max_temp_rise_c
    });
    let ac_ok = ac_report
        .as_ref()
        .is_none_or(|report| report.is_stable() && report.dc_gain_db.is_finite());
    let quality = QualityReport {
        assumed_currents,
        electrical_ok,
        thermal_ok,
        ac_ok,
        emi_ok: emi_report.as_ref().is_none_or(|r| r.is_ok()),
        violations: measurements
            .as_ref()
            .map(|m| m.violations.iter().map(|v| v.to_string()).collect::<Vec<String>>())
            .unwrap_or_default()
            .into_iter()
            .chain(
                emi_report
                    .as_ref()
                    .map(|r| r.violations.clone())
                    .unwrap_or_default(),
            )
            .collect(),
        schema_version: QUALITY_SCHEMA_VERSION.into(),
        model: ac_report
            .as_ref()
            .map(|r| r.model.clone())
            .unwrap_or_else(|| "dc-proxy".into()),
        confidence: ac_report
            .as_ref()
            .map(|r| r.confidence.clone())
            .unwrap_or_else(|| "low".into()),
        assumptions: ac_report
            .as_ref()
            .map(|r| r.assumptions.clone())
            .unwrap_or_default(),
        summary: format!(
            "electrical {} · thermal {} · AC {} · currents {}",
            if electrical_ok { "PASS" } else { "FAIL" },
            if thermal_ok { "PASS" } else { "FAIL" },
            if ac_ok { "PASS" } else { "FAIL" },
            if assumed_currents { "assumed (advisory)" } else { "supplied (gating)" }
        ),
    };

    let elapsed_ms = started.elapsed().as_millis() as u64;
    // A two-layer board is judged on the final copper: the single-layer engine
    // is only the first pass there, so its partial verdict is not final.
    let engine_ok = board.two_layer
        || routing.outcome == crate::routing_engine::EngineOutcome::Routed;
    // An explicit analysis_policy lets the agent decide which quality verdicts
    // are blocking; by default every supplied analysis gates production.
    let gate = |name: &str| {
        config.analysis_policy.is_empty()
            || config
                .analysis_policy
                .iter()
                .any(|p| p.eq_ignore_ascii_case(name))
    };
    let quality_ok = (assumed_currents
        || ((!gate("electrical") || quality.electrical_ok)
            && (!gate("thermal") || quality.thermal_ok)))
        && (!gate("ac") || quality.ac_ok)
        && (!gate("emi") || quality.emi_ok);
    let ok = engine_ok
        && complete.failed_nets.is_empty()
        && drc.is_ok()
        && !exported_files.is_empty()
        && kicad_board_path.is_some()
        && quality_ok;

    Ok(PipelineResult {
        board_name,
        width_mm: board.width_mm,
        height_mm: board.height_mm,
        footprints: board.footprints.len(),
        nets: board.nets.len(),
        mounting_holes: board.mounting_holes.len(),
        pre_rules,
        fixed_copper_mm,
        routing,
        final_copper: complete,
        jumpers,
        planned_jumpers,
        measurements,
        ac: ac_report,
        emi: emi_report,
        quality,
        drc,
        exported_files,
        kicad_board_path,
        thermal: thermal_field,
        transient,
        elapsed_ms,
        ok,
    })
}

/// Load a board from a file (KiCad PCB) and run the pipeline.
pub fn run_pipeline_from_file<P: AsRef<Path>>(
    path: P,
    config: PipelineConfig,
    out_dir: &str,
) -> Result<PipelineResult> {
    let data = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read {}", path.as_ref().display()))?;
    let stem = path
        .as_ref()
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Imported board".into());
    let board = if data.trim_start().starts_with('{') {
        Board::from_json(&data)
            .with_context(|| format!("invalid board JSON in {}", path.as_ref().display()))?
    } else {
        import_kicad_pcb_str_named(&data, &stem)
            .with_context(|| format!("cannot import {}", path.as_ref().display()))?
            .board
    };
    run_pipeline(board, config, out_dir)
}

/// Run the pipeline on a board with automatically derived rules from its preset.
pub fn pipe(board: Board, grid_mm: f64, time_budget_s: f64) -> Result<PipelineResult> {
    let config = PipelineConfig {
        grid_mm,
        time_budget_s,
        rules: Some(rules_for_board(&board)),
        ..Default::default()
    };
    run_pipeline(board, config, "output")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boards::default_board;

    #[test]
    fn pipeline_runs_on_default_board() {
        let board = default_board();
        let config = PipelineConfig {
            grid_mm: 0.5,
            time_budget_s: 5.0,
            run_kicad_cli_drc: false,
            ..Default::default()
        };
        let tmp = tempfile::tempdir().unwrap();
        let res = run_pipeline(board, config, tmp.path().to_str().unwrap()).unwrap();
        assert!(res.elapsed_ms > 0);
        // Default board may fail some nets: then nothing is exported, and that
        // is the pipeline's contract, not a crash.
        if res.routing.result.failed_nets.is_empty() {
            assert!(!res.exported_files.is_empty());
        }
        assert!(res.kicad_board_path.is_some());
    }

    #[test]
    fn engine_routes_power_and_ground_with_clearance() {
        // A small board keeps this unit test fast; the big boards are covered
        // by tests/acceptance_100.rs.
        let board = crate::boards::build("lm386-mini").unwrap();
        let rail_names: Vec<String> = board
            .nets
            .iter()
            .filter(|n| matches!(n.class, NetClass::Ground | NetClass::Power))
            .map(|n| n.name.clone())
            .collect();
        let config = PipelineConfig {
            grid_mm: 0.5,
            time_budget_s: 60.0,
            run_kicad_cli_drc: false,
            ..Default::default()
        };
        let tmp = tempfile::tempdir().unwrap();
        let res = run_pipeline(board, config, tmp.path().to_str().unwrap()).unwrap();
        assert!(res.fixed_copper_mm > 0.0);
        let path = res.kicad_board_path.expect("KiCad board written");
        let text = std::fs::read_to_string(path).unwrap();
        for name in &rail_names {
            assert!(
                res.routing.result.routed_nets.contains(name),
                "rail `{name}` was left to the star bias instead of being routed"
            );
        }
        let segments = text.matches("(segment ").count();
        assert_eq!(segments, res.final_copper.segments.len());
        assert!(res.drc.is_ok(), "delivered copper must be DRC-clean");
    }

    #[test]
    fn quality_is_advisory_with_assumed_currents() {
        let board = crate::boards::build("lm386-mini").unwrap();
        let config = PipelineConfig {
            grid_mm: 0.5,
            time_budget_s: 30.0,
            ..Default::default()
        };
        let tmp = tempfile::tempdir().unwrap();
        let res = run_pipeline(board, config, tmp.path().to_str().unwrap()).unwrap();
        assert!(res.quality.assumed_currents);
        // The board is still manufacturable even though guessed 1 A rails
        // produce electrical warnings — those are advisory by design.
        assert!(res.ok);
    }

    #[test]
    fn supplied_currents_make_quality_a_hard_gate() {
        let board = crate::boards::build("lm386-mini").unwrap();
        let currents: Vec<NetCurrent> = board
            .nets
            .iter()
            .map(|n| NetCurrent::new(&n.name, 10.0))
            .collect();
        let config = PipelineConfig {
            grid_mm: 0.5,
            time_budget_s: 30.0,
            measure_currents: Some(currents),
            ..Default::default()
        };
        let tmp = tempfile::tempdir().unwrap();
        let res = run_pipeline(board, config, tmp.path().to_str().unwrap()).unwrap();
        assert!(!res.quality.assumed_currents);
        assert!(!res.quality.electrical_ok, "{}", res.quality.summary);
        assert!(!res.ok, "a supplied-current failure must reject the board");
    }

    #[test]
    fn measurements_are_populated_by_default() {
        let board = default_board();
        let config = PipelineConfig {
            grid_mm: 0.5,
            time_budget_s: 5.0,
            ..Default::default()
        };
        let tmp = tempfile::tempdir().unwrap();
        let res = run_pipeline(board, config, tmp.path().to_str().unwrap()).unwrap();
        let report = res.measurements.expect("measurement report");
        assert!(!report.nets.is_empty(), "no net was measured");
    }

    #[test]
    fn pipeline_from_file_accepts_board_json() {
        let board = default_board();
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("board.json");
        std::fs::write(&path, board.to_json().unwrap()).unwrap();
        let config = PipelineConfig {
            grid_mm: 0.5,
            time_budget_s: 5.0,
            ..Default::default()
        };
        let res = run_pipeline_from_file(&path, config, tmp.path().to_str().unwrap()).unwrap();
        assert_eq!(res.board_name, board.name);
        assert!(res.kicad_board_path.is_some());
    }

    #[test]
    fn pipeline_from_file_works() {
        let path = r"C:\Users\Pstuha\Documents\Default Project\amproute\amp_class_ab.kicad_pcb";
        if !std::path::Path::new(path).exists() {
            eprintln!("skipping: reference board not found");
            return;
        }
        let config = PipelineConfig {
            grid_mm: 0.5,
            time_budget_s: 10.0,
            run_kicad_cli_drc: false,
            ..Default::default()
        };
        let tmp = tempfile::tempdir().unwrap();
        let res = run_pipeline_from_file(path, config, tmp.path().to_str().unwrap()).unwrap();
        assert!(res.kicad_board_path.is_some());
        // Export only when routing succeeded.
        if res.routing.result.failed_nets.is_empty() {
            assert!(!res.exported_files.is_empty());
        }
    }
}
