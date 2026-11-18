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
use std::path::PathBuf;

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
    println!("AmpRoute Studio {} — headless", amproute_studio::VERSION);

    if args.input.is_some() && args.board.is_some() {
        eprintln!("Use either --input or --board, not both.");
        std::process::exit(2);
    }

    let board = match (&args.input, &args.board) {
        (Some(path), _) => load_board(path)?,
        (None, Some(id)) => amproute_studio::build_board(id).ok_or_else(|| {
            anyhow::anyhow!("unknown board id `{id}` — try --list-boards")
        })?,
        (None, None) => amproute_studio::default_board(),
    };

    let issues = board.validate();
    if !issues.is_empty() {
        eprintln!("Board validation failed:");
        for issue in &issues {
            eprintln!("  - {issue}");
        }
        std::process::exit(2);
    }

    let grid = args.grid;
    let mut router = amproute_studio::Router::new(grid, board.width_mm, board.height_mm);
    for (x, y, radius) in board.thermal_hotspots() {
        router.add_penalty_circle(x, y, radius);
    }
    if let Some((x, y)) = board.star_ground_point() {
        router.add_penalty_circle(x, y, 8.0);
    }

    let started = std::time::Instant::now();
    let result = router.route_board(&board);
    let elapsed = started.elapsed();

    println!(
        "Board {} — {:.0}x{:.0} mm, {} footprints, {} nets, {} mounting holes, grid {:.2} mm",
        board.name,
        board.width_mm,
        board.height_mm,
        board.footprints.len(),
        board.nets.len(),
        board.mounting_holes.len(),
        grid
    );
    println!(
        "Routed {}/{} nets, {:.1} mm of copper, {:.1} s",
        result.routed_nets.len(),
        result.total_nets(),
        result.total_length_mm,
        elapsed.as_secs_f64()
    );

    // Thermal report, from the same model the GUI shades.
    let sources = amproute_studio::thermal::sources_for(&board);
    let field = amproute_studio::thermal::solve(
        &board,
        &sources,
        &amproute_studio::ThermalParams::default(),
    );
    println!(
        "Thermal: peak rise {:.1} °C at ({:.1}, {:.1}) mm, mean {:.2} °C, {} heat sources",
        field.peak_rise_c,
        field.peak_at.0,
        field.peak_at.1,
        field.mean_rise_c(),
        sources.len()
    );

    if !result.failed_nets.is_empty() {
        eprintln!("Routing incomplete:");
        for failure in &result.failed_nets {
            eprintln!("  - {failure}");
        }
        eprintln!("Nothing was exported: incomplete copper is not safe to manufacture.");
        std::process::exit(3);
    }

    let dir = args.output.to_string_lossy().to_string();
    let files = match amproute_studio::export::write_all(&board, &result, &dir) {
        Ok(files) => files,
        Err(e) => {
            eprintln!("Export refused: {e}");
            std::process::exit(3);
        }
    };
    for file in &files {
        println!("  wrote {file}");
    }

    // The KiCad board is the deliverable a user actually opens and checks.
    let stem = board.name.replace(' ', "_").to_lowercase();
    let kicad_path = format!("{dir}/{stem}.kicad_pcb");
    let fab = amproute_studio::kicad::FabOptions::default();
    match amproute_studio::kicad::write_kicad_pcb(&board, &result, &fab, &kicad_path) {
        Ok(()) => println!("  wrote {kicad_path}"),
        Err(e) => {
            eprintln!("KiCad export refused: {e}");
            std::process::exit(3);
        }
    }

    Ok(())
}
