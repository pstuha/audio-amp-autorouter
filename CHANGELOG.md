# Changelog

All notable changes to AmpRoute Studio are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.1.0] — 2026-11-18

Both `.kicad_pcb` and the program's own JSON are now first-class inputs, the
thermal view is a real simulation, and five fully routed, DRC-clean test boards
ship with the program.

### Added

- **Five built-in test amplifier boards**: `lm386-mini` (45 × 35 mm),
  `tda2030-compact` (70 × 50 mm), `cmoy-headphone` (80 × 50 mm), `classic-ltp`
  (130 × 90 mm) and `twin-pair-200` (170 × 110 mm). Each is the left channel of a
  stereo amplifier, with corner mounting holes and their copper keep-outs.
- **Board picker in the GUI** and an **Open…** dialog that accepts `.kicad_pcb`
  or the program's JSON.
- **KiCad writer**: exports a `.kicad_pcb` with real footprints, pads carrying
  their nets, the outline on `Edge.Cuts`, non-plated mounting holes and the routed
  copper. Targets file version `20250114` (KiCad 10), with deterministic UUIDs so
  repeated exports are byte-identical.
- **KiCad reader**: loads an existing `.kicad_pcb`, including mounting holes, and
  reports every feature it had to skip.
- **Thermal simulation**: a steady-state 2-D finite-volume conduction and
  convection solve on a ~2 mm grid, drawn as a heatmap in the GUI. Power is
  estimated per part type; tabbed power packages couple only a fraction of their
  dissipation into the board, because the heatsink is their thermal path.
- **CLI**: `--list-boards`, `--board <id>`, `-i` accepting either format, and a
  `.kicad_pcb` written alongside the Gerber set.
- 80 tests: unit tests plus acceptance tests for all five boards.

### Changed

- Routing is judged with **exact millimetre geometry** instead of stamped grid
  cells, which is what finally closed the dense boards.
- Per-segment **track width**: the class width is tried first and narrowed only
  where it cannot pass, never below the process minimum.
- Net class widths reduced to 0.8 mm (power/output), 0.6 mm (ground) and 0.4 mm
  (signals) — values a laser-iron transfer actually holds.
- A\* rewritten on flat arrays with a precomputed cost grid, about 5× faster.
- **Rip-up and reroute**: a stuck net is promoted ahead of the net that walls it
  in, or to the front of the order when the blocker is unknown.
- Software zones are softer by default (`penalty_weight` 1.5) so a thermal zone
  biases copper without wrecking a dense single-layer board.
- Release builds use the Windows GUI subsystem and re-attach to the parent
  console only for `--cli`, so double-clicking the executable shows no console.

### Fixed

- Footprint courtyards are drawn from the pads, not the bounding box, so parts
  that are physically clear no longer report DRC overlap.
- Reference and value text moved to the fabrication layer: silkscreen text across
  a neighbouring pad is a DRC warning about nothing.
- Mounting holes round-trip through the writer and reader.

### Verified

All five exported boards pass KiCad 10's own DRC with **zero violations**
(`kicad-cli pcb drc --severity-all`) and have zero unconnected pads.

## [1.0.0] — 2026-11-18

First release under the name **AmpRoute Studio**. The project was rewritten from
scratch in Rust, replacing the earlier Python/PySide6 prototype.

### Added

- **Desktop GUI** (`eframe`/`egui`) that opens on launch — double-clicking the
  executable shows the board, no console detour.
  - Zoom (mouse wheel), pan (drag), 10 mm grid, layer toggles.
  - Net list with per-net routing status and class colours.
  - Component inspector, statistics panel, status bar.
  - Keyboard shortcuts: `F5` routes, `Ctrl+E` exports.
- **Auto-placement** of a classic audio chain: input on the left, gain stages in
  the centre, output on the right, power along the top, ground star at the bottom.
- **Grid A\* router** with audio-aware net classes, thermal penalty zones and a
  star-ground keep-out. Copper of other nets is treated as a hard obstacle with
  track width plus LUT clearance, and several deterministic net orders are tried.
- **Export**: Gerber (RS-274X), SVG preview, PDF report, and a 1:1 LUT print
  template sized for laser-iron / toner-transfer production.
- **Model validation** that rejects pads wired to two nets (shorts), dangling pad
  references, footprints outside the outline, and geometry below the LUT preset.
- **Headless CLI** (`--cli`) for batch runs, with a non-zero exit code when
  validation or routing fails.
- 34 unit tests covering the board model, router, placement and export writers.

### Safety

Export is **refused** when the board fails validation or any net fails to route.
Incomplete copper is never written, because a partially connected layout is not
safe to manufacture.

### Known limitations

- Single-layer, through-hole only. No vias, no double-layer routing yet.
- The router is a grid-based prototype: it does not rip up and reroute nets, so a
  dense board may report failures instead of finding a path.
- Structural validation is **not** a manufacturing DRC. Copper clearance,
  annular rings, current capacity and thermal behaviour are not certified —
  always review the generated Gerber in an independent viewer before etching.
- Velocity of autorouting on a 0.25 mm grid is roughly 4 seconds for the demo
  board; larger boards scale with area.

[1.1.0]: https://github.com/pstuha/audio-amp-autorouter/releases/tag/v1.1.0
[1.0.0]: https://github.com/pstuha/audio-amp-autorouter/releases/tag/v1.0.0
