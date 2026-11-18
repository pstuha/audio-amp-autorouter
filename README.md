# AmpRoute Studio

Autorouter and 2-D board viewer for **single-layer LUT** (laser-iron / toner-transfer)
audio amplifier PCBs.

![AmpRoute Studio](docs/screenshot.png)

Double-click the executable and the board opens. No console, no command line.

This is a **standalone program**, not a KiCad plugin. It reads and writes
`.kicad_pcb` files, so it fits into a KiCad workflow without being installed
into KiCad.

---

## What it does

| Stage | Feature |
|-------|---------|
| **Boards** | Five built-in test amplifiers from 45 × 35 mm to 170 × 110 mm, plus any board you load: `.kicad_pcb` or the program's own JSON. |
| **Routing** | Grid A\* with audio net classes, exact-millimetre clearance, per-segment track width. Thermal and star-ground keep-outs bias paths away. Rip-up and reroute when a net is boxed in. |
| **Thermal** | A real steady-state 2-D conduction + convection solve, drawn as a heatmap. Not decorative blobs. |
| **Checks** | Rejects pads wired to two nets (shorts), dangling pad references, footprints outside the outline, geometry below the LUT preset. |
| **Export** | Gerber (RS-274X), SVG preview, PDF report, a **1:1 LUT print template**, and a **`.kicad_pcb`** you can open in KiCad. |

### The five test boards

Each is the **left channel** of a stereo amplifier — enough to prove the router
copes with different topologies and sizes.

| Board | Size | Stage |
|-------|------|-------|
| `lm386-mini` | 45 × 35 mm | LM386 single chip |
| `tda2030-compact` | 70 × 50 mm | TDA2030 power IC, split supply |
| `cmoy-headphone` | 80 × 50 mm | NE5532 headphone amp, virtual ground |
| `classic-ltp` | 130 × 90 mm | Class-AB LTP + VAS + triple output |
| `twin-pair-200` | 170 × 110 mm | Dual-pair discrete output |

All five follow the same physical rules: mounting holes in the corners with a
copper keep-out, copper clear of the board edge, every pad on exactly one net,
input nets kept away from output and power nets.

Every exported board passes KiCad's own DRC with **zero violations**
(`kicad-cli pcb drc --severity-all`). The routed results are checked into
[`boards/`](boards/) as `.kicad_pcb` files, one per test board, so you can open
any of them in KiCad without building or running anything:

```powershell
kicad-cli pcb drc boards\classic_ltp.kicad_pcb --severity-all
```

To route your own board, generate the same kind of file with the GUI (`Ctrl+K`)
or the CLI, open it in KiCad's PCB editor, and check it with that project's own
DRC.

### GUI

- Pick a built-in board, or open your own `.kicad_pcb` / JSON.
- Zoom with the wheel, pan by dragging.
- Net list with live routing status and class colours.
- Toggle copper, components, pads, grid, soft zones, and the thermal heatmap.
- `F5` routes, `Ctrl+K` writes a `.kicad_pcb`, `Ctrl+E` exports the rest.

### Headless

```powershell
# List the built-in boards
.\AmpRoute-Studio.exe --list-boards

# Route one of them and export everything
.\AmpRoute-Studio.exe --cli --board classic-ltp --output my_out

# Route your own board — KiCad PCB or the program's JSON
.\AmpRoute-Studio.exe --cli --input my.kicad_pcb --output my_out --grid 0.5
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
| `<board>.kicad_pcb` | The board as a KiCad project file: footprints, nets, pads, outline, mounting holes and the routed copper. |

---

## Honest limitations

This is a working engineering prototype, not a certified EDA tool. Read this
before etching anything:

- **Single layer, through-hole only.** No vias, no double-layer routing.
- **Import is deliberately partial.** Zones, vias, arcs on inner layers,
  teardrops and custom pad shapes are reduced or skipped, and every skipped
  feature is reported. Rotated footprints are read as 0°.
- **The thermal model is not FEM or CFD.** It is a steady-state 2-D conduction
  and convection solve on a ~2 mm grid. It answers "where does the heat go, and
  which tracks run through hot areas", not "what is the junction temperature".
  Devices bolted to a heatsink are modelled as coupling only a fraction of their
  dissipation into the board.
- **Not a manufacturing DRC.** The exported `.kicad_pcb` is checked with KiCad's
  DRC, but annular rings, current capacity and edge clearance are not certified.
- **Always open the generated Gerber in an independent viewer** (KiCad Gerber
  Viewer, Gerbv) and verify scale, orientation and connectivity before production.

The software refuses to write copper when validation fails or a net is
unrouted — but that is a guard against obvious mistakes, not a guarantee.

---

## Building from source

Requires Rust 1.75 or newer.

```bash
cargo build --release      # target/release/amproute-studio.exe
cargo test                 # 80 tests
```

On Linux, install the usual GUI development packages (`libxkbcommon`,
`libwayland-dev`, `libgl1-mesa-dev`) for `eframe`.

---

## Project layout

```
src/
  board.rs      Board model, net classes, LUT presets, validation
  boards.rs     The five built-in test amplifier boards
  sexpr.rs      Reader and writer for KiCad's S-expression format
  import.rs     Load an existing .kicad_pcb into the model
  kicad.rs      Write a .kicad_pcb that KiCad opens and checks
  occupancy.rs  Exact-millimetre copper clearance and slack
  router.rs     Grid A* router: audio costs, width taper, rip-up and reroute
  thermal.rs    Steady-state conduction + convection solve
  placement.rs  Auto-placement and placement checks
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
