# AmpRoute Studio

Autorouter and 2-D board viewer for **single-layer LUT** (laser-iron / toner-transfer)
audio amplifier PCBs.

![AmpRoute Studio](docs/screenshot.png)

Double-click the executable and the board opens. No console, no command line.

---

## What it does

| Stage | Feature |
|-------|---------|
| **Placement** | Auto-places a classic audio chain — input left, gain stages centre, output right, power rail along the top, ground star at the bottom. |
| **Routing** | Grid A\* with audio net classes. Copper of other nets is a hard obstacle at track width + clearance; thermal and star-ground keep-outs bias paths away. |
| **Checks** | Rejects pads wired to two nets (shorts), dangling pad references, footprints outside the outline, geometry below the LUT preset. |
| **Export** | Gerber (RS-274X), SVG preview, PDF report, and a **1:1 LUT print template**. |

### GUI

- Zoom with the wheel, pan by dragging.
- Net list with live routing status and class colours.
- Toggle copper, components, pads, grid, thermal overlay.
- `F5` routes, `Ctrl+E` exports to `output/`.

### Headless

```powershell
# Route the built-in demo and export
.\AmpRoute-Studio.exe --cli

# Route your own board
.\AmpRoute-Studio.exe --cli --input board.json --output my_out --grid 0.25
```

Exit codes: `0` success, `2` invalid board, `3` routing or export refused.

---

## Output files

| File | Purpose |
|------|---------|
| `board.gbr` | Gerber RS-274X copper layer, format 4.6, millimetres. |
| `board.svg` | Colour preview with per-net colours. |
| `board.pdf` | Summary report with statistics. |
| `lut_print.pdf` | 1:1 template for the iron-on transfer. Print at 100% — never "fit to page". |

---

## Honest limitations

This is a working engineering prototype, not a certified EDA tool. Read this
before etching anything:

- **Single layer, through-hole only.** No vias, no double-layer routing.
- **No rip-up and reroute.** If a net cannot find a path, it is reported as failed
  and the export is blocked. Dense boards will fail rather than invent a route.
- **Not a manufacturing DRC.** Copper clearance between *pads and tracks* of
  different nets is enforced by the router, but annular rings, current capacity,
  thermal behaviour and edge clearance are not certified.
- **Always open the generated Gerber in an independent viewer** (KiCad Gerber
  Viewer, Gerbv) and verify scale, orientation and connectivity before production.

The software refuses to write copper when validation fails or a net is
unrouted — but that is a guard against obvious mistakes, not a guarantee.

---

## Building from source

Requires Rust 1.75 or newer.

```bash
cargo build --release      # target/release/amproute-studio.exe
cargo test                 # 34 tests
```

On Linux, install the usual GUI development packages (`libxkbcommon`,
`libwayland-dev`, `libgl1-mesa-dev`) for `eframe`.

---

## Project layout

```
src/
  board.rs      Board model, net classes, LUT presets, validation
  placement.rs  Auto-placement and placement checks
  router.rs     Grid A* router with obstacles and penalty zones
  export.rs     Gerber / SVG / PDF / LUT template writers
  gui.rs        egui desktop application
  main.rs       Entry point: GUI by default, --cli for batch
tests/          Integration tests
```

---

## License

MIT — see [LICENSE](LICENSE).

Development followed the Russian-language technical specification supplied with
the project (ТЗ): GUI with 2-D board view (3), auto-placement (3.1), PDF/Gerber/SVG
export (4.4), star ground (5.1), input/power/output isolation (5.2), thermal
awareness (5.3), and LUT presets (6).
