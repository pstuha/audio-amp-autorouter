//! `amproute-studio` — AmpRoute Studio: autorouter for single-layer THT audio
//! amplifier PCBs manufactured with the LUT (laser-iron / toner-transfer) method.
//!
//! The crate is organised in three layers:
//!
//! * [`board`] — the data model: board geometry, footprints, nets, terminals.
//! * [`router`] — grid-based multi-pass A\* routing with audio-aware cost
//!   weights, thermal and star-ground penalty zones.
//! * [`export`] — Gerber (RS-274X), SVG and PDF writers.
//!
//! Plus two non-library layers used by the binary:
//!
//! * [`placement`] — deterministic auto-placement of a typical audio chain.
//! * [`gui`] — an `egui`/`eframe` 2-D board viewer.

pub mod board;
pub mod export;
pub mod gui;
pub mod placement;
pub mod router;

pub use board::{Board, Footprint, NetClass, Pad};
pub use placement::{drc_report, PlacementResult};
pub use router::{RouteSegment, Router, RoutingResult};

/// Crate version, taken from `Cargo.toml` at compile time.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
