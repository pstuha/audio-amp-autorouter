//! Thermal simulation for a single-layer PCB.
//!
//! This is a steady-state 2-D heat-conduction model, not a CFD or FEM solver.
//! Power is injected at hot components (output devices, regulators, power
//! resistors), spreads through the board by conduction, and leaves through the
//! surfaces by convection. It is solved on a uniform grid with an explicit
//! relaxation sweep until the field stops changing.
//!
//! What it answers: *where does the heat go, and which nets run through hot
//! areas?* That is enough to bias the router away from sensitive nodes.
//! What it cannot answer: junction temperature, transient response, airflow in a
//! real enclosure. Those need a proper thermal stack-up.

use crate::board::{Board, Footprint};

/// Heat source at a component.
#[derive(Debug, Clone)]
pub struct HeatSource {
    /// Component reference.
    pub refdes: String,
    /// Centre in board millimetres.
    pub x: f64,
    pub y: f64,
    /// Dissipated power in watts.
    pub power_w: f64,
    /// Radius over which the power enters the copper, in millimetres.
    pub spread_mm: f64,
}

/// A solved temperature field.
#[derive(Debug, Clone)]
pub struct ThermalField {
    pub cols: usize,
    pub rows: usize,
    /// Cell size in millimetres.
    pub cell_mm: f64,
    /// Temperature rise in °C above ambient, row-major.
    pub temps: Vec<f32>,
    pub sources: Vec<HeatSource>,
    /// Ambient temperature the rise is measured from.
    pub ambient_c: f64,
    /// Highest rise anywhere on the board.
    pub peak_rise_c: f64,
    /// Location of the peak, in board millimetres.
    pub peak_at: (f64, f64),
}

impl ThermalField {
    /// Temperature rise at a board coordinate, in °C.
    pub fn rise_at(&self, x: f64, y: f64) -> f32 {
        if self.cols == 0 || self.rows == 0 || self.cell_mm <= 0.0 {
            return 0.0;
        }
        let cx = ((x / self.cell_mm).floor() as isize).clamp(0, self.cols as isize - 1) as usize;
        let cy = ((y / self.cell_mm).floor() as isize).clamp(0, self.rows as isize - 1) as usize;
        self.temps[cy * self.cols + cx]
    }

    /// Normalised value in `0..=1` used for colour mapping.
    pub fn normalized(&self, index: usize) -> f32 {
        if self.peak_rise_c <= 0.0 {
            0.0
        } else {
            (self.temps[index] / self.peak_rise_c as f32).clamp(0.0, 1.0)
        }
    }

    /// Hottest corner region, for reporting.
    pub fn mean_rise_c(&self) -> f64 {
        if self.temps.is_empty() {
            return 0.0;
        }
        let sum: f64 = self.temps.iter().map(|t| *t as f64).sum();
        sum / self.temps.len() as f64
    }
}

/// Conductivity and loss parameters of the model.
#[derive(Debug, Clone)]
pub struct ThermalParams {
    /// Effective in-plane conductivity of 1.6 mm FR-4 with copper, W/(m·K).
    pub conductivity: f64,
    /// Combined convection + radiation coefficient, W/(m²·K).
    pub surface_loss: f64,
    /// Board thickness in millimetres (reserved for a future 3-D model).
    pub thickness_mm: f64,
    /// Ambient air temperature in °C.
    pub ambient_c: f64,
}

impl Default for ThermalParams {
    fn default() -> Self {
        Self {
            // 1.6 mm FR-4 alone conducts ~0.3 W/(m·K), but a copper plane carries
            // heat far better in-plane: 35 µm of copper is worth k·t ≈ 0.0135 W/K
            // per square, against 4.8e-4 for the laminate. A lumped 8 W/(m·K)
            // reproduces that copper-dominated spreading without pretending to
            // model the stack-up layer by layer.
            conductivity: 8.0,
            // Free convection plus radiation from *both* faces of the board.
            surface_loss: 20.0,
            thickness_mm: 1.6,
            ambient_c: 25.0,
        }
    }
}

/// Power estimates for the usual amplifier parts.
///
/// Values come from typical datasheet dissipation for a class-AB stage at
/// moderate output, or from I²R for resistors. They are estimates for layout
/// guidance, deliberately conservative — but they must stay physical: a small
/// chip like the LM386 never dissipates like a power transistor.
pub fn estimate_power(refdes: &str, value: &str) -> f64 {
    let up = format!("{refdes} {value}").to_ascii_uppercase();

    // Output devices. A power IC bolted to a heatsink sheds most of its heat
    // through the tab, so only part of it reaches the board.
    if up.contains("TDA") || up.contains("LM3886") || up.contains("LM1875") {
        return 3.0;
    }
    if up.contains("TIP") || up.contains("2N3055") || up.contains("MJE") || up.contains("K1058") {
        return 5.0;
    }
    if up.contains("IRF") {
        return 4.0;
    }
    // Small chip amplifiers.
    if up.contains("LM386") || up.contains("TBA") || up.contains("TEA") {
        return 0.7;
    }
    // Op-amps: quiescent dissipation only.
    if up.contains("NE5532") || up.contains("TL07") || up.contains("OPA") || up.contains("LM358") {
        return 0.2;
    }
    if up.contains("78") || up.contains("79") || up.contains("317") || up.contains("REG") {
        return 1.5;
    }

    // Power resistors: value like "10R" or "0R47", treat as a fraction of a watt.
    if up.contains("R_") || refdes.starts_with('R') {
        if let Some(watts) = parse_wattage_hint(value) {
            return watts;
        }
        // Emitter resistors carry real current; small ones dissipate little.
        if value.to_ascii_uppercase().contains("0R") || value.contains("0.4") {
            return 0.5;
        }
    }

    // Everything else is a small-signal part.
    if refdes.starts_with('Q') {
        return 0.4;
    }
    0.0
}

fn parse_wattage_hint(value: &str) -> Option<f64> {
    let up = value.to_ascii_uppercase();
    // "2W" / "5W" in the value string.
    if let Some(idx) = up.find('W') {
        let head = &up[..idx];
        let digits: String = head
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        if let Ok(w) = digits.parse::<f64>() {
            return Some(w);
        }
    }
    None
}

/// Collect heat sources for a board using the built-in power estimates.
///
/// Not all of a device's dissipation enters the board. A TO-220 or TO-3P output
/// transistor is bolted to a heatsink through its metal tab: that tab *is* the
/// thermal path, and only a small fraction spreads into the copper. Passing the
/// full dissipation to the board instead makes an ordinary 100 W amplifier look
/// like it runs at 270 °C, which is wrong and would mislead the router into
/// avoiding perfectly good areas.
pub fn sources_for(board: &Board) -> Vec<HeatSource> {
    board
        .footprints
        .iter()
        .filter_map(|fp| {
            let total = estimate_power(&fp.refdes, &fp.value);
            if total <= 0.0 {
                return None;
            }
            let (cx, cy) = fp.center();
            Some(HeatSource {
                refdes: fp.refdes.clone(),
                x: cx,
                y: cy,
                power_w: total * board_coupling(fp),
                // Power devices spread heat over a wider pad area.
                spread_mm: (fp.width_mm.max(fp.height_mm) * 0.5).clamp(2.0, 10.0),
            })
        })
        .collect()
}

/// Fraction of a part's dissipation that reaches the PCB.
///
/// A tabbed power package — TO-220 or TO-3P, which this model builds with three
/// or five pads on a wide body — is bolted to a heatsink through its metal tab.
/// That tab is the thermal path, so only a fraction of the heat reaches the
/// copper. Everything else, including a DIP-8 op-amp, sits on its own leads and
/// puts its whole dissipation into the board.
fn board_coupling(fp: &Footprint) -> f64 {
    let across = fp.width_mm.max(fp.height_mm);
    let tabbed = across >= 12.0 && matches!(fp.pads.len(), 3 | 5);
    if tabbed {
        0.15
    } else {
        1.0
    }
}

/// Solve the steady-state temperature field.
///
/// Iterates a finite-volume relaxation until the change per sweep falls below
/// `tolerance` °C or `max_sweeps` is reached, so a pathological board cannot
/// hang the GUI.
pub fn solve(board: &Board, sources: &[HeatSource], params: &ThermalParams) -> ThermalField {
    // Roughly 2 mm cells: fine enough to show gradients, coarse enough to solve
    // in a few milliseconds for hobby-sized boards.
    let target_cell = 2.0_f64;
    let cols = ((board.width_mm / target_cell).ceil() as usize).clamp(4, 200);
    let rows = ((board.height_mm / target_cell).ceil() as usize).clamp(4, 200);
    let cell_mm = (board.width_mm / cols as f64).max(0.5);

    let n = cols * rows;
    let mut temps = vec![0.0_f32; n];

    // Inject power: each source deposits its watts evenly over its spread disc.
    let mut power_density = vec![0.0_f64; n];
    for src in sources {
        if src.power_w <= 0.0 {
            continue;
        }
        let r_cells = (src.spread_mm / cell_mm).max(1.0);
        let cx = src.x / cell_mm - 0.5;
        let cy = src.y / cell_mm - 0.5;
        let mut touched = 0.0;
        let x0 = ((cx - r_cells).floor() as isize).max(0) as usize;
        let x1 = ((cx + r_cells).ceil() as isize).min(cols as isize - 1) as usize;
        let y0 = ((cy - r_cells).floor() as isize).max(0) as usize;
        let y1 = ((cy + r_cells).ceil() as isize).min(rows as isize - 1) as usize;
        for cy_i in y0..=y1 {
            for cx_i in x0..=x1 {
                let dx = cx_i as f64 - cx;
                let dy = cy_i as f64 - cy;
                if dx * dx + dy * dy <= r_cells * r_cells {
                    touched += 1.0;
                }
            }
        }
        if touched == 0.0 {
            // Fall back to the nearest cell so a tiny part still heats something.
            let idx = (cy.round().clamp(0.0, rows as f64 - 1.0) as usize) * cols
                + (cx.round().clamp(0.0, cols as f64 - 1.0) as usize);
            power_density[idx] += src.power_w;
            continue;
        }
        for cy_i in y0..=y1 {
            for cx_i in x0..=x1 {
                let dx = cx_i as f64 - cx;
                let dy = cy_i as f64 - cy;
                if dx * dx + dy * dy <= r_cells * r_cells {
                    power_density[cy_i * cols + cx_i] += src.power_w / touched;
                }
            }
        }
    }

    // Finite-volume coefficients for a cell of size h on a plate of thickness t.
    let h = cell_mm;
    let t = params.thickness_mm;
    let k = params.conductivity;
    let conv = params.surface_loss;
    // Conductance between neighbours (W/K), and loss to ambient per cell (W/K).
    let g_neighbour = k * t * 1e-3; // k * (t[m] * h[m]) / h[m]
    let g_ambient = conv * (h * 1e-3) * (h * 1e-3);

    let tolerance = 1e-3_f64;
    let max_sweeps = 3000;
    let mut sweeps = 0;

    for sweep in 0..max_sweeps {
        let mut max_delta = 0.0_f64;
        for idx in 0..n {
            let cx = idx % cols;
            let cy = idx / cols;

            let mut sum_g = g_ambient;
            let mut sum_gt = 0.0_f64;
            for (nx, ny) in neighbours(cx, cy, cols, rows) {
                let nidx = ny * cols + nx;
                sum_g += g_neighbour;
                sum_gt += g_neighbour * temps[nidx] as f64;
            }

            let t_new = (power_density[idx] + sum_gt) / sum_g;
            let delta = (t_new - temps[idx] as f64).abs();
            if delta > max_delta {
                max_delta = delta;
            }
            // Under-relaxation keeps the explicit sweep stable.
            temps[idx] = (temps[idx] as f64 + 0.6 * (t_new - temps[idx] as f64)) as f32;
        }
        sweeps = sweep + 1;
        if max_delta < tolerance {
            break;
        }
    }
    let _ = sweeps;

    let mut peak = 0.0_f32;
    let mut peak_idx = 0;
    for (i, t) in temps.iter().enumerate() {
        if *t > peak {
            peak = *t;
            peak_idx = i;
        }
    }
    let peak_at = (
        ((peak_idx % cols) as f64 + 0.5) * cell_mm,
        ((peak_idx / cols) as f64 + 0.5) * cell_mm,
    );

    ThermalField {
        cols,
        rows,
        cell_mm,
        temps,
        sources: sources.to_vec(),
        ambient_c: params.ambient_c,
        peak_rise_c: peak as f64,
        peak_at,
    }
}

fn neighbours(cx: usize, cy: usize, cols: usize, rows: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::with_capacity(4);
    if cx > 0 {
        out.push((cx - 1, cy));
    }
    if cx + 1 < cols {
        out.push((cx + 1, cy));
    }
    if cy > 0 {
        out.push((cx, cy - 1));
    }
    if cy + 1 < rows {
        out.push((cx, cy + 1));
    }
    out
}

/// Colour for a temperature rise, from cool blue-green to hot red.
pub fn colour_for_rise(rise_c: f64) -> (u8, u8, u8) {
    // 0 °C → dark blue, 10 °C → teal, 25 °C → yellow, 50 °C+ → red.
    let t = (rise_c / 50.0).clamp(0.0, 1.0);
    let stops: [(f64, (u8, u8, u8)); 4] = [
        (0.0, (24, 48, 120)),
        (0.25, (30, 150, 150)),
        (0.55, (230, 200, 70)),
        (1.0, (220, 60, 45)),
    ];
    for w in stops.windows(2) {
        let (t0, c0) = w[0];
        let (t1, c1) = w[1];
        if t <= t1 {
            let f = if (t1 - t0).abs() < 1e-9 {
                0.0
            } else {
                (t - t0) / (t1 - t0)
            };
            let lerp = |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * f) as u8;
            return (lerp(c0.0, c1.0), lerp(c0.1, c1.1), lerp(c0.2, c1.2));
        }
    }
    stops[3].1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{Board, Footprint, LutPreset, NetClass};

    fn board_with_hot_part() -> Board {
        let mut b = Board::new("thermal", 80.0, 60.0, LutPreset::reliable());
        b.add_footprint(Footprint::new(
            "Q1",
            "IRF540",
            NetClass::Output,
            40.0,
            30.0,
            10.0,
            8.0,
        ));
        b
    }

    #[test]
    fn hottest_point_is_at_the_power_device() {
        let board = board_with_hot_part();
        let sources = sources_for(&board);
        assert_eq!(sources.len(), 1);
        let field = solve(&board, &sources, &ThermalParams::default());

        assert!(field.peak_rise_c > 1.0, "device should heat the board");
        let dx = field.peak_at.0 - 45.0;
        let dy = field.peak_at.1 - 34.0;
        assert!(
            (dx * dx + dy * dy).sqrt() < 12.0,
            "peak at {:?} should be near the transistor centre",
            field.peak_at
        );
    }

    #[test]
    fn temperature_falls_away_from_the_source() {
        let board = board_with_hot_part();
        let field = solve(&board, &sources_for(&board), &ThermalParams::default());
        let hot = field.rise_at(45.0, 34.0);
        let cold = field.rise_at(5.0, 5.0);
        assert!(
            hot > cold * 2.0,
            "near the device {hot:.1} °C should clearly exceed the far corner {cold:.1} °C"
        );
    }

    #[test]
    fn a_board_without_power_parts_stays_cool() {
        let mut b = Board::new("cool", 50.0, 40.0, LutPreset::reliable());
        b.add_footprint(Footprint::new(
            "R1",
            "10k",
            NetClass::Signal,
            20.0,
            20.0,
            6.0,
            4.0,
        ));
        let field = solve(&b, &sources_for(&b), &ThermalParams::default());
        assert!(field.peak_rise_c < 0.01, "got {}", field.peak_rise_c);
    }

    #[test]
    fn more_power_means_higher_temperature() {
        let board = board_with_hot_part();
        let cool = solve(
            &board,
            &[HeatSource {
                refdes: "Q1".into(),
                x: 45.0,
                y: 34.0,
                power_w: 2.0,
                spread_mm: 5.0,
            }],
            &ThermalParams::default(),
        );
        let hot = solve(
            &board,
            &[HeatSource {
                refdes: "Q1".into(),
                x: 45.0,
                y: 34.0,
                power_w: 8.0,
                spread_mm: 5.0,
            }],
            &ThermalParams::default(),
        );
        assert!(
            hot.peak_rise_c > cool.peak_rise_c * 3.0,
            "4x power should raise the peak substantially: {} vs {}",
            hot.peak_rise_c,
            cool.peak_rise_c
        );
    }

    #[test]
    fn solve_is_deterministic() {
        let board = board_with_hot_part();
        let sources = sources_for(&board);
        let a = solve(&board, &sources, &ThermalParams::default());
        let b = solve(&board, &sources, &ThermalParams::default());
        assert_eq!(a.temps, b.temps);
    }

    #[test]
    fn hot_colours_are_reddish_and_cold_ones_bluish() {
        let (r_cold, _, b_cold) = colour_for_rise(0.0);
        let (r_hot, _, b_hot) = colour_for_rise(60.0);
        assert!(b_cold > r_cold, "cold should be blue-dominant");
        assert!(r_hot > b_hot, "hot should be red-dominant");
    }

    #[test]
    fn rise_lookup_is_clamped_to_the_board() {
        let board = board_with_hot_part();
        let field = solve(&board, &sources_for(&board), &ThermalParams::default());
        // Out-of-range coordinates must not panic.
        let _ = field.rise_at(-50.0, -50.0);
        let _ = field.rise_at(5000.0, 5000.0);
    }
}
