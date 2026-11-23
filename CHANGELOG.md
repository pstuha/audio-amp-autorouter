# Changelog

All notable changes to AmpRoute Studio are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [2.0.0] — 2026-11-23

Major release: agent-first workflow, professional routing geometry and official CAM.

- Agent API v1.0 on CLI/MCP with stable JSON contract and quality schema.
- 8-connected/octile A*, line-of-sight simplification, 45° canonicalisation,
  LUT width taper; `non_canonical = 0` on routed boards.
- Star power/ground pre-route, jumper planning/insertion, single- and two-layer
  routing with vias.
- Official KiCad exporters for IPC-2581 Rev C and ODB++; Gerber/SVG/PDF/LUT/Excellon.
- 15 reference boards at 100 % routing with zero KiCad DRC violations.
- New KiCad-like egui editor, new README/About and cleaned repository layout.

## [1.3.2] — 2026-11-23

- CAM-форматы теперь создаются официальными экспортёрами KiCad 10: IPC-2581 Rev C
  (`--ipc`) и ODB++ zip (`--odb`), а не самописными приближениями.
- Перемычки стали DRC-совместимым представлением: JP footprint добавляется в
  плату, сеть убирается из netlist (провод изолированный), координаты — в
  `jumpers.csv`. `kicad-cli DRC` остаётся чистым.
- Финальный визуальный аудит: `non_canonical=0`, 7/7 цепей, DRC 0.

## [1.3.1] — 2026-11-22

- GUI starts with **no preloaded board**; the five built-in designs remain test
  fixtures only. File → Choose export folder and the Export panel let the user
  select the output directory; `Ctrl+K`/`Ctrl+E` honour it.
- Copper is drawn with round caps/joins in the 2-D viewer so corner pixels look
  like circular-aperture Gerber. The routed geometry itself is unchanged: an
  experimental geometric chamfer was validated in isolation but changed the
  clearance/parallel-run verdicts on shipped boards, so it stays disabled.

## [1.3.0] — 2026-11-22

All five stages of `docs/quality-roadmap.md` implemented; the external SPICE
path is now verified against a real ngspice 47 run on RC and op-amp golden
cases (`tests/spice_golden.rs`, enabled with `AMPROUTE_NGSPICE`). Bench
validation on a physical amplifier remains explicitly blocked — see the
roadmap.

### Added

- `NetExcitation` (RMS/peak, spectrum, return net, source impedance) with CLI
  `--currents` and MCP `measure_currents`/`excitations`.
- Built-in linear AC solver (`src/ac.rs`): complex MNA over R/C/L, sources and
  op-amp macromodels; gain/phase sweep, −3 dB bandwidth and phase margin.
  `src/spice.rs` emits a ngspice deck and runs it when the simulator exists.
- `src/emi.rs`: parallel-run coupling in dB and return-loop areas from the
  delivered copper; `src/thermal.rs::solve_transient`: junction estimate, time
  constant and copper-aware conductivity.
- MCP tools `run_ac`, `generate_netlist`, `run_emi`, `run_thermal`; CLI flags
  `--ac-models`, `--transient`; `quality` carries `model`, `confidence`,
  `assumptions` and per-analysis pass flags that gate `ok` when requested.

## [1.2.1] — 2026-11-21

- Ten agent-designed experimental amplifier boards (`src/extra_boards.rs`),
  reachable via `--board <id>`, `--list-boards` and the MCP tool
  `list_extra_boards`. All ten route 100 % with zero KiCad 10 DRC violations
  and produce the complete LUT file set — the reference case for the
  "agent invents a board → router returns finished files" workflow.

## [1.2.0] — 2026-11-21

Professional-tool pass: deterministic agent interface, a KiCad-like editor and
real two-layer capability. All five built-in boards now route 100 % with zero
DRC violations and a complete export set under the default settings.

### Added

- **MCP server** (`amproute-mcp`): JSON-RPC 2.0 over stdio with
  `get_capabilities`, `list_boards`, `validate_board`, `route_board`,
  `get_result` and `run_drc`; `--json` CLI now emits exactly one JSON document.
- **Two-layer routing**: `Board::two_layer`, plated `Via` objects, a layered
  A* completion pass for nets the single-layer engine cannot finish, B.Cu
  Gerber, KiCad vias, two-layer DRC and GUI layer/via display.
- `--two-layer` CLI flag and the GUI checkbox.
- Acceptance test for all five boards (100 % routing, clean own DRC, four
  exports) and two-layer export/KiCad tests. All five exported boards were
  verified with `kicad-cli 10.0.8 pcb drc --severity-all`: zero violations and
  zero unconnected items. The CLI now auto-detects `kicad-cli` even when it is
  not on PATH and includes the result in `drc.kicad_drc_ran`.

### Changed

- **New egui GUI** with KiCad-like menus, toolbars, layer/appearance tree,
  board canvas with rotation and mirroring, selection/inspector and status bar.
  The Python/pywebview mock is no longer part of the build.
- Module 5 star rails are now obstacle-aware soft biases; the engine routes
  every rail itself with exact clearance, which removes the clearance conflicts
  between fixed rails and signal copper.
- Finer-grid retry (0.5 → 0.25 mm) when nets remain unroutable.
- Footprint rotation is applied consistently in the model, importer, exporter
  and renderer.

### Fixed

- `ok` is derived from routing/DRC/export instead of being hardcoded; DRC runs
  after the KiCad file is written; measurements get per-class currents; JSON
  input works through the file pipeline; power and ground no longer share a
  star; via-free profiles no longer accept via budgets and vice versa.
- Reference tests now use finite time budgets; Rust 1.75 MSRV restored.

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
- 81 tests: unit tests plus acceptance tests for all five boards.
- The routed results are checked into `boards/` as `.kicad_pcb` files, one per
  test board, so they can be opened and checked in KiCad without running the
  program.

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
