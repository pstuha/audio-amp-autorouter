# Agent API (v1.0)

AmpRoute Studio exposes two machine-readable entry points. Both are stable
within API version 1.0: fields are only added, never removed or renamed.

## 1. Batch CLI — shells and scripts

```powershell
amproute-studio.exe --cli --board classic-ltp -o out --json
amproute-studio.exe --cli -i my.kicad_pcb -o out --grid 0.5 --json
```

With `--json`, **stdout is exactly one JSON document** — a serialized
`PipelineResult`. All human-readable progress goes to stderr. Exit codes:

| Code | Meaning |
|------|---------|
| 0 | success |
| 2 | invalid board / bad input |
| 3 | routing or export refused (`result.ok == false`) |

Important fields: `ok`, `routing` (outcome, routed/failed nets, deadline flag),
`final_copper` (every exported segment, with `layer`), `fixed_copper_mm`,
`measurements`, `drc`, `exported_files`, `kicad_board_path`, `thermal`,
`elapsed_ms`.

## 2. MCP server — agents with tool calls

```powershell
cargo build --release --bin amproute-mcp
# the agent launches: target\release\amproute-mcp.exe
```

Transport: newline-delimited JSON-RPC 2.0 over stdio. One request per line,
one response per line. Nothing is written to stdout except responses.

### Handshake

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}
```

### Tools (`tools/list`)

| Tool | Arguments | Result |
|------|-----------|--------|
| `get_capabilities` | – | API version, formats, layers, exit codes |
| `list_boards` | – | five built-in boards with dimensions and net counts |
| `list_extra_boards` | – | ten agent-designed experimental boards (see below) |
| `validate_board` | `board` (built-in id) or `path` (file) | validation issues, geometry, counts |
| `route_board` | `board` or `path`, optional `grid_mm`, `time_budget_s`, `run_kicad_cli_drc`, `two_layer`, `output_dir` | route summary and export paths |
| `get_result` | – | full `PipelineResult` of the last route in this session |
| `run_drc` | `board` or `path` | `DrcReport` |
| `run_ac` | `board`/`path`, optional `ac_models`, `ac_input_net`, `ac_output_net` | `AcReport` (gain/phase, bandwidth, phase margin, `model: proxy`) |
| `generate_netlist` | same as `run_ac` | ngspice deck text |
| `run_emi` | `board`/`path`, optional `emi` config | `EmiReport` (coupling dB, loop areas, violations) |
| `run_thermal` | `board`/`path` | `TransientThermal` (junction estimate, τ, copper coverage) |
| `optimize_placement` | `board` or `path` | THT placement candidates with 0/180 rotation and validation report |
| `set_lut_profile` | `min_track_mm`, `min_clearance_mm` (optional) | LUT profile accepted/rejected per rules |
| `suggest_jumpers` | `board` or `path` | Wire-jump endpoints of the routed board |

Call a tool with `tools/call`:

```json
{"jsonrpc":"2.0","id":2,"method":"tools/call",
 "params":{"name":"route_board",
           "arguments":{"board":"lm386-mini","output_dir":"C:\\tmp\\lut","time_budget_s":20}}}
```

The response contains both MCP text content and a machine-readable
`structuredContent`:

```json
{"jsonrpc":"2.0","id":2,
 "result":{"content":[{"type":"text","text":"{ ... }"}],
           "structuredContent":{"ok":true,"board":"LM386 Mini", ...},
           "isError":false}}
```

The same tools are callable directly as methods (`"method":"route_board"` with
the arguments in `params`), returning the raw result without the MCP envelope.

### Errors

Standard JSON-RPC codes: `-32700` parse error, `-32600` invalid request,
`-32601` unknown method/tool, `-32602` invalid arguments (missing board, bad
path, unknown board id), `-32603` pipeline failure.

### Quality checks

`ok` is the manufacturability verdict: every net routed, own DRC clean,
KiCad DRC (`drc.kicad_drc_ran`) clean, exports written. Raw routing geometry is
also checked by `routing.post_rules` (LUT width/clearance, acute corners,
parallel runs) before copper is shipped.

Electrical/thermal quality is reported separately in `quality`:

| Field | Meaning |
|-------|---------|
| `assumed_currents` | `true` when currents were guessed from net classes — the verdict is advisory |
| `electrical_ok` / `thermal_ok` | pass flags for IR drop, width-by-current and temperature rise |
| `violations` | codes `M02_TRACK_WIDTH`, `M03_VOLTAGE_DROP`, `M04_TEMPERATURE` |
| `ac_ok` / `emi_ok` | pass flags when those analyses were requested (they gate `ok`) |
| `model`, `confidence`, `assumptions` | honesty metadata: `proxy`/`spice`, `low`/`medium`, proxy assumptions |

`route_board` also returns top-level `ac`, `emi` and `transient` objects when
`ac_models`, `emi` or `transient: true` are supplied. CLI equivalents:
`--ac-models`, `--currents`, `--transient`.

When the agent supplies real currents, quality becomes a hard gate on `ok`:

```json
"measure_currents": [
  {"net": "V+", "current_a": 0.2, "peak_a": 0.4},
  {"net": "OUT", "current_a": 0.15, "peak_a": 0.3}
]
```

Currents default to conservative class guesses (1 A rails, 0.5 A output),
so advisory FAILs on a board that otherwise routes cleanly are expected until
the agent provides the real numbers.

### Agent-designed boards

The ten experimental boards in `src/extra_boards.rs` are the reference case for
the intended workflow: an agent invents a circuit, describes it as a board id
(or its own JSON/KiCad file), and the router returns a finished board plus the
LUT deliverables. They are not part of the five-board acceptance baseline.

Ids: `pam8403-mini`, `lm358-preamp`, `tda2822-bridge`, `ne555-tone`,
`classa-single`, `jfet-buffer`, `discrete-opamp`, `lm1875-mono`,
`ta2024-tripath`, `hybrid-output`.

All ten route 100 % with zero KiCad DRC violations and produce
`board.gbr`, `board.svg`, `board.pdf`, `lut_print.pdf` and `<board>.kicad_pcb`.
They are reachable from `list_extra_boards` / `route_board` and from the CLI
with `--board <id>`. An agent's own board is passed as `path` to `route_board`
(`.kicad_pcb` or AmpRoute JSON) — no GUI step is required.

### Typical agent flow

1. `list_boards` → pick a board id, or accept a `path` from the user.
2. `validate_board` → check geometry before spending time routing.
3. `route_board` → route and write Gerber / SVG / PDF / LUT template / KiCad board.
4. `get_result` → inspect DRC, measurements and exact output paths.
5. Hand `lut_print.pdf` and `board.gbr` to the user for the toner transfer.
   On two-layer boards (`two_layer: true`) also hand over `board_b.gbr`.

`ok: false` means do **not** manufacture: read `failed_nets` and `drc_violations`.
