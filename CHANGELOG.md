# Changelog

All notable changes to AmpRoute Studio are documented here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

[1.0.0]: https://github.com/pstuha/audio-amp-autorouter/releases/tag/v1.0.0
