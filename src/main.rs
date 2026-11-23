//! AmpRoute Studio — entry point.
//!
//! Double-clicking the executable opens the GUI with no console window. Batch
//! export is explicit:
//!
//! ```text
//! amproute-studio.exe                       # GUI
//! amproute-studio.exe --list-boards         # list the built-in boards
//! amproute-studio.exe --cli --board lm386-mini -o out
//! amproute-studio.exe --cli -i my.kicad_pcb -o out
//! amproute-studio.exe --cli -i board.json -o out
//! ```
//!
//! The CLI exits non-zero when the board fails validation or any net fails to
//! route, so an automated pipeline never silently ships incomplete copper.

// Release builds are GUI applications: no console window appears when the
// executable is launched by double-click. `--cli` re-attaches to the parent
// terminal so batch output is still visible (see `attach_parent_console`).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use anyhow::Result;
use clap::Parser;
use serde_json;
use std::path::PathBuf;
use amproute_studio::pipeline::{PipelineConfig, run_pipeline};

/// Re-attach to the console that launched this process.
///
/// A GUI-subsystem executable starts detached from any terminal. Batch mode
/// still needs its output, so we borrow the parent's console before the first
/// write. Without a parent console (double-click) nothing happens and the GUI
/// proceeds silently.
#[cfg(windows)]
fn attach_parent_console() {
    const ATTACH_PARENT_PROCESS: u32 = 0xFFFF_FFFF;
    unsafe extern "system" {
        fn AttachConsole(process_id: u32) -> i32;
    }
    // SAFETY: AttachConsole only borrows the parent console; failure is benign
    // and simply means there is nothing to attach to.
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(windows))]
fn attach_parent_console() {}

#[derive(Parser, Debug)]
#[command(
    name = "amproute-studio",
    version,
    about = "Autorouter and 2-D board viewer for single-layer LUT audio amplifier PCBs"
)]
struct Args {
    /// Headless mode: route the board and write exports, no window.
    #[arg(long)]
    cli: bool,

    /// List the built-in test boards and exit.
    #[arg(long)]
    list_boards: bool,

    /// Built-in board to route (only with --cli). See --list-boards.
    #[arg(long)]
    board: Option<String>,

    /// Board file to route: `.kicad_pcb` or AmpRoute JSON (only with --cli).
    #[arg(short, long)]
    input: Option<PathBuf>,

    /// Output directory (only with --cli).
    #[arg(short, long, default_value = "output")]
    output: PathBuf,

    /// Router grid in millimetres. Smaller is finer but slower: 0.5 mm
    /// reproduces the tested routing results in about a second per small board.
    #[arg(short, long, default_value_t = 0.5)]
    grid: f64,

    /// Route as a two-layer board with plated vias (default: single-layer LUT).
    #[arg(long)]
    two_layer: bool,

    /// Agent-supplied net excitations (JSON array) for real electrical quality.
    #[arg(long)]
    currents: Option<PathBuf>,

    /// AC component models keyed by reference (JSON object).
    #[arg(long)]
    ac_models: Option<PathBuf>,

    /// Add the transient junction-temperature proxy to the report.
    #[arg(long)]
    transient: bool,

    /// Compare a 45°/octile routing pass with the stable Manhattan pass.
    #[arg(long)]
    diagonal: bool,

    /// Also write an ODB++-style topology profile.
    #[arg(long)]
    odb: bool,

    /// Also write an IPC-2581-style XML profile.
    #[arg(long)]
    ipc: bool,

    /// Opt-in: try 180° placement flips before routing.
    #[arg(long)]
    optimize_placement: bool,

    /// Emit the full pipeline result as JSON to stdout (for GUI integration).
    #[arg(long)]
    json: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    if args.list_boards || args.cli {
        attach_parent_console();
    }
    if args.list_boards {
        print_boards();
        return Ok(());
    }
    if args.cli {
        run_cli(&args)
    } else {
        amproute_studio::gui::run()
    }
}

fn print_boards() {
    println!("AmpRoute Studio {} — built-in boards", amproute_studio::VERSION);
    for spec in amproute_studio::BOARD_SPECS {
        println!("  {:<18} {}", spec.id, spec.summary);
    }
    println!("Agent-generated experimental boards:");
    for spec in amproute_studio::EXTRA_BOARD_SPECS {
        println!("  {:<18} {}", spec.id, spec.summary);
    }
}

/// Load a board from a file, accepting either format.
fn load_board(path: &PathBuf) -> Result<amproute_studio::Board> {
    let data = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", path.display()))?;

    // JSON is unambiguous: it starts with a brace.
    if data.trim_start().starts_with('{') {
        return amproute_studio::Board::from_json(&data)
            .map_err(|e| anyhow::anyhow!("invalid board JSON in {}: {e}", path.display()));
    }

    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Imported board".into());
    let imported = amproute_studio::import_kicad_pcb_str_named(&data, &stem)
        .map_err(|e| anyhow::anyhow!("cannot import {}: {e}", path.display()))?;
    if !imported.report.warnings.is_empty() {
        eprintln!("Import notes for {}:", path.display());
        for warning in &imported.report.warnings {
            eprintln!("  - {warning}");
        }
    }
    Ok(imported.board)
}

fn run_cli(args: &Args) -> Result<()> {
    if !args.json {
        println!("AmpRoute Studio {} — headless", amproute_studio::VERSION);
    }

    if args.input.is_some() && args.board.is_some() {
        eprintln!("Use either --input or --board, not both.");
        std::process::exit(2);
    }

    let mut board = if let Some(path) = &args.input {
        load_board(path)?
    } else if let Some(id) = &args.board {
        amproute_studio::build_any(id).ok_or_else(|| {
            anyhow::anyhow!("unknown board id `{id}` — try --list-boards")
        })?
    } else {
        amproute_studio::default_board()
    };
    if args.two_layer {
        board.two_layer = true;
    }
    if args.optimize_placement {
        board = amproute_studio::PlacementResult::optimize(&board);
    }
    // Validate board before running pipeline.
    let issues = board.validate();
    if !issues.is_empty() {
        eprintln!("Board validation failed:");
        for issue in &issues {
            eprintln!("  - {issue}");
        }
        std::process::exit(2);
    }
    // Build pipeline config
    let mut config = PipelineConfig {
        grid_mm: args.grid,
        time_budget_s: 300.0,
        run_kicad_cli_drc: true,
        include_transient: args.transient,
        diagonal_search: true,
        ..Default::default()
    };
    if let Some(path) = &args.ac_models {
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", path.display()))?;
        let models: std::collections::HashMap<String, amproute_studio::AcModel> =
            serde_json::from_str(&text)?;
        config.ac = Some(amproute_studio::AcConfig {
            models,
            ..Default::default()
        });
    }
    if let Some(path) = &args.currents {
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", path.display()))?;
        if let Ok(excitations) = serde_json::from_str::<Vec<amproute_studio::NetExcitation>>(&text) {
            config.excitations = Some(excitations);
        } else {
            config.measure_currents =
                Some(serde_json::from_str::<Vec<amproute_studio::NetCurrent>>(&text)?);
        }
    }
    let grid = config.grid_mm; // keep for printing
    // The board was loaded once above (JSON or KiCad); do not reload it here.
    let res = run_pipeline(board.clone(), config, &args.output.to_string_lossy())?;
    // Official CAM formats come from KiCad's own exporters, not from a
    // hand-written approximation: IPC-2581 Rev C and ODB++ (zip).
    if args.odb || args.ipc {
        let cli = amproute_studio::drc::find_kicad_cli();
        let pcb = res
            .kicad_board_path
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("KiCad board was not written"))?;
        if args.ipc {
            let out = args.output.join("board.xml");
            let status = std::process::Command::new(&cli)
                .args(["pcb", "export", "ipc2581", "--version", "C"])
                .arg("-o")
                .arg(&out)
                .arg(pcb)
                .status()?;
            if !status.success() {
                anyhow::bail!("kicad-cli IPC-2581 export failed");
            }
        }
        if args.odb {
            let out = args.output.join("board_odb.zip");
            let status = std::process::Command::new(&cli)
                .args(["pcb", "export", "odb", "--compression", "zip"])
                .arg("-o")
                .arg(&out)
                .arg(pcb)
                .status()?;
            if !status.success() {
                anyhow::bail!("kicad-cli ODB++ export failed");
            }
        }
    }

    // `--json` is the machine-readable contract for agents and the GUI:
    // stdout carries exactly one JSON document, all human text goes to stderr.
    if args.json {
        println!("{}", serde_json::to_string(&res)?);
    } else {
        println!(
            "Board {} — {:.0}x{:.0} mm, {} footprints, {} nets, {} mounting holes, grid {:.2} mm",
            res.board_name,
            res.width_mm,
            res.height_mm,
            res.footprints,
            res.nets,
            res.mounting_holes,
            grid
        );
        println!(
            "Routed {}/{} nets, {:.1} mm of copper, {:.1} s, deadline_reached={}",
            res.routing.result.routed_nets.len(),
            res.nets,
            res.routing.result.total_length_mm,
            res.elapsed_ms as f64 / 1000.0,
            res.routing.deadline_reached
        );
        if let Some(j) = &res.jumpers {
            println!("Jumpers: {}", j.summary());
        }
        let field = &res.thermal;
        println!(
            "Thermal: peak rise {:.1} °C at ({:.1}, {:.1}) mm, mean {:.2} °C, {} heat sources",
            field.peak_rise_c,
            field.peak_at.0,
            field.peak_at.1,
            field.mean_rise_c(),
            field.sources.len()
        );
        println!(
            "DRC: {} violation(s) — {}",
            res.drc.violations.len(),
            if res.drc.is_ok() { "PASS" } else { "FAIL" }
        );
        println!("Export: {} file(s) written", res.exported_files.len());
        if let Some(path) = &res.kicad_board_path {
            println!("  wrote {path}");
        }
    }

    if !res.ok {
        eprintln!(
            "Pipeline incomplete: {} routed nets, {} failed, {} DRC violations",
            res.routing.result.routed_nets.len(),
            res.routing.result.failed_nets.len(),
            res.drc.violations.len()
        );
        for v in &res.drc.violations {
            eprintln!("  DRC: {} — {}", v.rule, v.message);
        }
        std::process::exit(3);
    }

    Ok(())
}
