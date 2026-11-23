//! `amproute-studio` — AmpRoute Studio: autorouter for single-layer THT audio
//! amplifier PCBs manufactured with the LUT (laser-iron / toner-transfer) method.
//!
//! The crate is organised in layers:
//!
//! * [`board`] — the data model: board geometry, footprints, nets, terminals.
//! * [`sexpr`] — minimal reader/writer for KiCad's S-expression format.
//! * [`import`] — load an existing `.kicad_pcb` board into the model.
//! * [`io`] — the rest of module 2: Specctra DSN export, Specctra session
//!   (.ses) import, KiCad netlist export — format conversion only.
//! * [`thermal`] — steady-state heat spreading simulation.
//! * [`drc`] — module 9: DRC/audit — `kicad-cli` DRC + own geometry audit
//!   (single-layer, no vias, clearance).
//! * [`rules`] — module 4: the LUT hard-constraint profile, applied before and
//!   after routing.
//! * [`jumper`] — module 7: post-router pad pairs and insulated wire instead of vias.
//! * [`measure`] — module 8: DC voltage drop, copper loss and width-by-current.
//! * [`router`] — grid-based multi-pass A\* routing with audio-aware cost
//!   weights, thermal and star-ground penalty zones.
//! * [`export`] — Gerber (RS-274X), SVG, PDF and KiCad writers.
//!
//! Plus the layers used by the binary:
//!
//! * [`placement`] — the built-in example boards.
//! * [`preroute`] — module 5: pre-route power and ground before the router.
//! * [`gui`] — an `egui`/`eframe` 2-D board viewer.

pub mod ac;
pub mod agent;
pub mod board;
pub mod boards;
pub mod drc;
pub mod emi;
pub mod export;
pub mod extra_boards;
pub mod gui;
pub mod import;
pub mod io;
pub mod jumper;
pub mod kicad;
pub mod measure;
pub mod occupancy;
pub mod placement;
pub mod preroute;
pub mod router;
// Module 6: the FreeRouting wrapper. Executive only — it converts, launches and
// re-checks; every decision belongs to modules 4 and 5.
pub mod routing_engine;
pub mod rules;
pub mod sexpr;
pub mod spice;
pub mod thermal;
pub mod two_layer;
pub mod pipeline;

pub use ac::{analyze as analyze_ac, AcConfig, AcModel, AcPoint, AcReport, C as Complex};
pub use spice::{find_ngspice, generate_netlist, run_ngspice};
pub use board::{Board, Footprint, NetClass, Pad, LutPreset};
pub use boards::{all_boards, build as build_board, default_board, BoardSpec, BOARD_SPECS};
pub use extra_boards::{build as build_extra_board, ExtraSpec, EXTRA_BOARD_SPECS};

/// Resolve a board id from either the five shipped boards or the ten
/// agent-generated experimental boards.
pub fn build_any(id: &str) -> Option<Board> {
    build_board(id).or_else(|| build_extra_board(id))
}
pub use drc::{audit, kicad_cli_drc, own_audit, DrcReport, DrcViolation};
pub use emi::{analyze as analyze_emi, Coupling, EmiConfig, EmiReport, Loop};
pub use import::{
    import_kicad_pcb, import_kicad_pcb_str, import_kicad_pcb_str_named, ImportReport, ImportedBoard,
};
pub use io::{
    export_dsn, export_netlist, import_ses, write_dsn, write_netlist, SesImport, SesTrack, SesVia,
};
pub use jumper::{
    apply_jumpers, bridge_failed_net, BreakRequest, Jumper, JumperConfig, JumperHandler,
    JumperRejection, JumperResult,
};
pub use measure::{
    CopperSpec, Measure, MeasureConfig, MeasureReport, MeasureRule, MeasureViolation, NetCurrent,
    NetExcitation, NetMeasurement, SpectrumPoint,
};
pub use preroute::{is_pre_routed_class, preroute, PreRoute, PreRouteTrace};
pub use placement::PlacementResult;
pub use router::{CopperLayer, RouteSegment, Router, RoutingResult, Via};
pub use two_layer::{route_unfinished, TwoLayerResult};
pub use rules::{LutRules, RuleId, RuleReport, RuleViolation};
pub use thermal::{
    copper_coverage, solve_transient, ThermalField, ThermalParams, TransientThermal,
};

/// Crate version, taken from `Cargo.toml` at compile time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
