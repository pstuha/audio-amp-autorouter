//! AmpRoute Studio — entry point.
//!
//! Double-clicking the executable opens the GUI. Batch export is explicit:
//!
//! ```text
//! amproute-studio.exe                 # GUI
//! amproute-studio.exe --cli           # route the demo board and export
//! amproute-studio.exe --cli -i b.json -o out
//! ```
//!
//! The CLI exits non-zero when the board fails validation or any net fails to
//! route, so an automated pipeline never silently ships incomplete copper.

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

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

    /// Board project JSON (only with --cli). Omit to use the built-in demo.
    #[arg(short, long)]
    input: Option<PathBuf>,

    /// Output directory (only with --cli).
    #[arg(short, long, default_value = "output")]
    output: PathBuf,

    /// Router grid in millimetres. Smaller is finer but slower.
    #[arg(short, long, default_value_t = 0.25)]
    grid: f64,
}

fn main() -> Result<()> {
    let args = Args::parse();
    if args.cli {
        run_cli(&args)
    } else {
        amproute_studio::gui::run()
    }
}

fn run_cli(args: &Args) -> Result<()> {
    println!("AmpRoute Studio {} — headless", amproute_studio::VERSION);

    let board = match &args.input {
        Some(path) => {
            let data = std::fs::read_to_string(path)
                .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", path.display()))?;
            amproute_studio::Board::from_json(&data)
                .map_err(|e| anyhow::anyhow!("invalid board JSON in {}: {e}", path.display()))?
        }
        None => amproute_studio::PlacementResult::auto_amplifier().board,
    };

    let issues = board.validate();
    if !issues.is_empty() {
        eprintln!("Board validation failed:");
        for issue in &issues {
            eprintln!("  - {issue}");
        }
        std::process::exit(2);
    }

    let mut router = amproute_studio::Router::new(args.grid, board.width_mm, board.height_mm);
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
        "Board {} — {:.0}x{:.0} mm, {} footprints, {} nets",
        board.name,
        board.width_mm,
        board.height_mm,
        board.footprints.len(),
        board.nets.len()
    );
    println!(
        "Routed {}/{} nets, {:.1} mm of copper, {:.1} s",
        result.routed_nets.len(),
        result.total_nets(),
        result.total_length_mm,
        elapsed.as_secs_f64()
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
    match amproute_studio::export::write_all(&board, &result, &dir) {
        Ok(files) => {
            for file in files {
                println!("  wrote {file}");
            }
            Ok(())
        }
        Err(e) => {
            eprintln!("Export refused: {e}");
            std::process::exit(3);
        }
    }
}
