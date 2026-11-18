//! `amproute-studio` — AmpRoute Studio: autorouter for single-layer THT audio
//! amplifier PCBs manufactured with the LUT (laser-iron / toner-transfer) method.
//!
//! The crate is organised in layers:
//!
//! * [`board`] — the data model: board geometry, footprints, nets, terminals.
//! * [`sexpr`] — minimal reader/writer for KiCad's S-expression format.
//! * [`import`] — load an existing `.kicad_pcb` board into the model.
//! * [`thermal`] — steady-state heat spreading simulation.
//! * [`router`] — grid-based multi-pass A\* routing with audio-aware cost
//!   weights, thermal and star-ground penalty zones.
//! * [`export`] — Gerber (RS-274X), SVG, PDF and KiCad writers.
//!
//! Plus the layers used by the binary:
//!
//! * [`placement`] — the built-in example boards.
//! * [`gui`] — an `egui`/`eframe` 2-D board viewer.

pub mod board;
pub mod boards;
pub mod export;
pub mod gui;
pub mod import;
pub mod kicad;
pub mod occupancy;
pub mod placement;
pub mod router;
pub mod sexpr;
pub mod thermal;

pub use board::{Board, Footprint, NetClass, Pad};
pub use boards::{all_boards, build as build_board, default_board, BoardSpec, BOARD_SPECS};
pub use import::{
    import_kicad_pcb, import_kicad_pcb_str, import_kicad_pcb_str_named, ImportReport, ImportedBoard,
};
pub use placement::PlacementResult;
pub use router::{RouteSegment, Router, RoutingResult};
pub use thermal::{ThermalField, ThermalParams};

/// Crate version, taken from `Cargo.toml` at compile time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
