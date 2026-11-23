# Quality roadmap — from "routeable" to "electrically trustworthy"

Status: **all five stages implemented** (2026-11-22). Hard gates remain
routing completeness, LUT rules, own DRC and `kicad-cli` DRC; the electrical
verdicts are separate in `quality` and only gate `ok` when the agent supplies
the corresponding inputs. Every analysis report is labelled `model: "proxy"`
(except an external ngspice run) and carries `confidence` + `assumptions`.

Implemented modules: `src/ac.rs` (MNA solver), `src/spice.rs` (netlist +
optional ngspice), `src/emi.rs` (coupling + loop areas), `src/thermal.rs`
(`solve_transient`), plus the `NetExcitation` schema in `src/measure.rs`.

Verified with a **real ngspice 47** run (`AMPROUTE_NGSPICE` points at
`ngspice_con.exe`; the GUI build writes nothing in `-b` mode):
`tests/spice_golden.rs` checks the RC −3 dB point against
`1/(2πRC)` and an inverting op-amp gain against the 100k/10k ratio.

The goal is to let an agent supply an electrical description of its own design
and get a quantitative quality verdict back through the same JSON/MCP contract.

## ✅ Stage 1 — real currents instead of class guesses (done)

- Extend `NetCurrent` into `NetExcitation`:
  `net`, `rms_a`, `peak_a`, `return_net`, `spectrum_hz` (frequency/amplitude pairs),
  `duty_cycle`, `source_impedance_ohm`.
- CLI: `--currents agent_currents.json`; MCP `route_board.measure_currents`
  accepts the same schema. Quality already becomes a hard gate when currents
  are supplied (`src/pipeline.rs::QualityReport`).
- Keep class-based guesses as `assumed_currents: true` (advisory).
- Tests: schema roundtrip, advisory vs gating, per-net IR drop with real
  spectrum-weighted peak.

## ✅ Stage 2 — AC / SPICE class analysis (done)

Two complementary tracks:

1. **Self-contained linear AC solver** (`src/ac.rs`, no external tool):
   - Modified nodal analysis with complex arithmetic over R, C, L, ideal
     VCVS/CCVS and simple op-amp macromodels.
   - Inputs: board connectivity + an agent-supplied model library
     (`models/<value>.json`, e.g. `10k`, `100n`, `LM358`, `2N3904`).
   - Outputs: gain/phase vs frequency, −3 dB bandwidth, phase margin for
     feedback nets, input-referred noise estimate.
2. **External SPICE bridge** (`src/spice.rs`): generate a netlist from the
   board + models, run `ngspice -b` or LTspice when installed, parse AC/TRAN
   tables into the same `AcReport`. No simulator is installed on this machine
   today, so track 1 is the one that can be finished and tested locally.

Tests: RC low-pass cutoff, inverting op-amp gain, integrator phase, netlist
roundtrip. `AcReport` joins `quality` as `ac_ok`.

## ✅ Stage 3 — crosstalk and ground-loop EMI from geometry (done, `src/emi.rs`)

Geometry we already have: every routed segment, its layer, width, net and
class; board outline; star point. Build from it:

- **Parallel coupling**: for each pair of different nets on the same layer,
  integrate overlap where gap < 1.2 mm. Report coupling capacitance
  (`ε·L·w/g`) and mutual-inductance proxy, expressed as estimated crosstalk dB.
- **Loop area**: trace each signal net's return path through its `return_net`
  to the star point; compute polygon area (shoelace). Flag large loops and
  input loops that enclose output/rail copper.
- **Isolation score**: quantitative version of the current heuristic in
  `board.rs::NetClass::proximity_penalty` and `router.rs::cost_grid`.
- Output `EmiReport { crosstalk_db, loop_areas_mm2, violations }`, wired into
  `quality` (advisory first, hard once thresholds are agent-supplied).

This is a geometry proxy, not a full-wave field solve. A later optional
backend can use openEMS/FastHenry, but it must stay opt-in: solve time is
minutes, not seconds.

## ✅ Stage 4 — thermal upgrade (done, `src/thermal.rs`)

- Use actual routed copper density per cell as anisotropic conductivity
  (today the solve is uniform 2-D).
- Two-layer vertical coupling where vias connect F.Cu/B.Cu.
- Transient step response with package/heatsink time constants, producing
  `peak_junction_c` rather than only a board-average rise.
- Cross-validate against the I²R loss from `measure.rs` (already fed in).
- Keep advisory until bench thermocouple data exists; publish `thermal_ok`
  only when the agent supplies ambient and heatsink parameters.

## ✅ Stage 5 — validation and honesty (done, except bench data — BLOCKED)

- Golden cases: RC/opamp circuits with analytic answers; one amplifier with
  bench gain, bandwidth, THD and temperature measurements.
- Whenever a model is a proxy (DC lumping, 2-D heat, geometric crosstalk), the
  JSON must say so: `model: "proxy"`, `confidence`, `assumptions`.
- `ok` stays manufacturability. A separate `quality` object carries verdicts;
  an agent explicitly chooses which verdicts gate production via
  `analysis_policy`.

## Order of work

1 → 2 (self-contained solver) → 3 → 4 → 5. Each stage lands with unit tests
and an MCP/CLI field, so the tool stays usable after every step.

### BLOCKED: bench validation of one real amplifier

Not done and **not claimed done**: the roadmap calls for one amplifier with
measured gain, bandwidth, THD and temperature. Those numbers cannot be
synthesised in software — they require a physical board, bench instruments and
the user's measurements. Nothing in this repository is real bench data.

Required to unblock (per board revision, supply and load):

- gain vs frequency (e.g. 1 kHz and −3 dB point),
- THD+N at a stated output power and load,
- steady-state case/board temperature at a stated ambient and dissipation,
- the exact board JSON + ngspice deck used, so the simulation can be compared
  to the same configuration.

Until that data is supplied, every comparison remains analytic (RC cutoff,
op-amp resistor ratio) or a labelled proxy — never a claim about real sound.
