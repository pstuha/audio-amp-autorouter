//! Small linear AC solver for agent-supplied amplifier models.
//!
//! This is a proxy, not SPICE: it uses modified nodal analysis over R, C, L,
//! ideal voltage sources and op-amp macromodels. It answers gain, bandwidth
//! and phase margin well enough for a first design pass, and never claims to
//! model transistor nonlinearity, distortion or parasitics. For a real SPICE
//! run see [`crate::spice`].

use crate::board::Board;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ops::{Add, Div, Mul, Neg, Sub};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct C {
    pub re: f64,
    pub im: f64,
}

impl C {
    pub fn new(re: f64, im: f64) -> Self {
        Self { re, im }
    }
    pub fn abs(self) -> f64 {
        self.re.hypot(self.im)
    }
    pub fn arg_deg(self) -> f64 {
        self.im.atan2(self.re).to_degrees()
    }
    fn conj(self) -> Self {
        Self::new(self.re, -self.im)
    }
}

impl From<f64> for C {
    fn from(v: f64) -> Self {
        Self::new(v, 0.0)
    }
}
impl Add for C {
    type Output = C;
    fn add(self, o: C) -> C {
        C::new(self.re + o.re, self.im + o.im)
    }
}
impl Sub for C {
    type Output = C;
    fn sub(self, o: C) -> C {
        C::new(self.re - o.re, self.im - o.im)
    }
}
impl Mul for C {
    type Output = C;
    fn mul(self, o: C) -> C {
        C::new(
            self.re * o.re - self.im * o.im,
            self.re * o.im + self.im * o.re,
        )
    }
}
impl Div for C {
    type Output = C;
    fn div(self, o: C) -> C {
        let d = o.re * o.re + o.im * o.im;
        let n = self * o.conj();
        C::new(n.re / d, n.im / d)
    }
}
impl Neg for C {
    type Output = C;
    fn neg(self) -> C {
        C::new(-self.re, -self.im)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AcModel {
    Resistor { ohm: f64 },
    Capacitor { farad: f64 },
    Inductor { henry: f64 },
    /// Simple voltage-controlled amplifier: a three-terminal macromodel.
    OpAmp {
        plus_pin: usize,
        minus_pin: usize,
        out_pin: usize,
        gain: f64,
        #[serde(default = "default_input_ohm")]
        input_impedance_ohm: f64,
    },
}

fn default_input_ohm() -> f64 {
    1.0e6
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcConfig {
    /// Models keyed by footprint reference.
    #[serde(default)]
    pub models: HashMap<String, AcModel>,
    /// Net used as the AC source (default: first Input-class net).
    #[serde(default)]
    pub input_net: Option<String>,
    /// Net measured as the output (default: first Output-class net).
    #[serde(default)]
    pub output_net: Option<String>,
    #[serde(default = "default_f_start")]
    pub f_start_hz: f64,
    #[serde(default = "default_f_end")]
    pub f_end_hz: f64,
    #[serde(default = "default_points")]
    pub points: usize,
    /// Load applied to the output net, in ohms.
    #[serde(default)]
    pub load_ohm: Option<f64>,
}

fn default_f_start() -> f64 {
    10.0
}
fn default_f_end() -> f64 {
    100_000.0
}
fn default_points() -> usize {
    60
}

impl Default for AcConfig {
    fn default() -> Self {
        Self {
            models: HashMap::new(),
            input_net: None,
            output_net: None,
            f_start_hz: default_f_start(),
            f_end_hz: default_f_end(),
            points: default_points(),
            load_ohm: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcPoint {
    pub hz: f64,
    pub gain_db: f64,
    pub phase_deg: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcReport {
    pub points: Vec<AcPoint>,
    pub dc_gain_db: f64,
    /// First frequency where gain is 3 dB below the low-frequency plateau.
    pub bandwidth_hz: Option<f64>,
    /// Phase margin at the unity-gain crossing, in degrees.
    pub phase_margin_deg: Option<f64>,
    /// Always `proxy` for this solver.
    pub model: String,
    pub confidence: String,
    pub assumptions: Vec<String>,
}

impl AcReport {
    pub fn is_stable(&self) -> bool {
        self.phase_margin_deg.map_or(true, |m| m > 30.0)
    }
}

#[derive(Debug, Clone)]
enum Element {
    Passive(String, String, C),
    VSource(String, C),
    VcvS {
        a: String,
        b: String,
        p: String,
        m: String,
        gain: f64,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct Circuit {
    nets: Vec<String>,
    elements: Vec<Element>,
    ground: String,
}

impl Circuit {
    fn index(&self, net: &str) -> Option<usize> {
        if net == self.ground {
            return None;
        }
        self.nets.iter().position(|n| n == net)
    }

    fn solve(&self, omega: f64) -> Option<HashMap<String, C>> {
        let n = self.nets.len();
        let sources = self
            .elements
            .iter()
            .filter(|e| matches!(e, Element::VSource(..) | Element::VcvS { .. }))
            .count();
        let size = n + sources;
        let mut a = vec![vec![C::new(0.0, 0.0); size]; size];
        let mut b = vec![C::new(0.0, 0.0); size];
        let mut src = 0usize;

        for element in &self.elements {
            match element {
                Element::Passive(p, q, value) => {
                    // R/C/L are all admittances at this frequency. Admittance
                    // is supplied by the caller: 1/R, jωC or 1/(jωL).
                    let y = *value;
                    if let Some(i) = self.index(p) {
                        a[i][i] = a[i][i] + y;
                    }
                    if let Some(j) = self.index(q) {
                        a[j][j] = a[j][j] + y;
                    }
                    if let (Some(i), Some(j)) = (self.index(p), self.index(q)) {
                        a[i][j] = a[i][j] - y;
                        a[j][i] = a[j][i] - y;
                    }
                }
                Element::VSource(net, voltage) => {
                    let k = n + src;
                    src += 1;
                    if let Some(i) = self.index(net) {
                        a[i][k] = a[i][k] + C::new(1.0, 0.0);
                        a[k][i] = a[k][i] + C::new(1.0, 0.0);
                    }
                    b[k] = *voltage;
                }
                Element::VcvS {
                    a: pa,
                    b: pb,
                    p,
                    m,
                    gain,
                } => {
                    let k = n + src;
                    src += 1;
                    if let Some(i) = self.index(pa) {
                        a[i][k] = a[i][k] + C::new(1.0, 0.0);
                        a[k][i] = a[k][i] + C::new(1.0, 0.0);
                    }
                    if let Some(j) = self.index(pb) {
                        a[j][k] = a[j][k] - C::new(1.0, 0.0);
                        a[k][j] = a[k][j] - C::new(1.0, 0.0);
                    }
                    if let Some(i) = self.index(p) {
                        a[k][i] = a[k][i] - C::new(*gain, 0.0);
                    }
                    if let Some(j) = self.index(m) {
                        a[k][j] = a[k][j] + C::new(*gain, 0.0);
                    }
                }
            }
        }
        let _ = omega;
        let x = gaussian(a, b)?;
        let mut out = HashMap::new();
        for (i, net) in self.nets.iter().enumerate() {
            out.insert(net.clone(), x[i]);
        }
        Some(out)
    }
}

fn gaussian(mut a: Vec<Vec<C>>, mut b: Vec<C>) -> Option<Vec<C>> {
    let n = b.len();
    for col in 0..n {
        let mut pivot = col;
        for row in (col + 1)..n {
            if a[row][col].abs() > a[pivot][col].abs() {
                pivot = row;
            }
        }
        if a[pivot][col].abs() < 1e-18 {
            return None;
        }
        a.swap(col, pivot);
        b.swap(col, pivot);
        let inv = C::new(1.0, 0.0) / a[col][col];
        for cell in a[col].iter_mut().skip(col) {
            *cell = *cell * inv;
        }
        b[col] = b[col] * inv;
        for row in 0..n {
            if row == col {
                continue;
            }
            let factor = a[row][col];
            if factor.abs() < 1e-18 {
                continue;
            }
            for j in col..n {
                a[row][j] = a[row][j] - factor * a[col][j];
            }
            b[row] = b[row] - factor * b[col];
        }
    }
    Some(b)
}

/// Parse a passive component value such as `10k`, `1k5`, `100n`, `2u2`, `10uH`.
fn parse_value(value: &str) -> Option<(char, f64)> {
    let v = value.trim().replace(' ', "");
    let lower = v.to_ascii_lowercase();
    let (kind, suffix) = if let Some(s) = lower.strip_suffix("uh") {
        ('L', s)
    } else if let Some(s) = lower.strip_suffix('h') {
        ('L', s)
    } else if let Some(s) = lower.strip_suffix('f') {
        ('C', s)
    } else if let Some(s) = lower.strip_suffix('n') {
        ('C', s)
    } else if let Some(s) = lower.strip_suffix('u') {
        ('C', s)
    } else if let Some(s) = lower.strip_suffix('p') {
        ('C', s)
    } else if let Some(s) = lower.strip_suffix('r') {
        ('R', s)
    } else if let Some(s) = lower.strip_suffix('k') {
        ('R', s)
    } else {
        let s = lower.strip_suffix('m')?;
        ('R', s)
    };
    let number = suffix.replace(',', ".");
    let base: f64 = number.parse().ok()?;
    let multiplier = match lower.chars().last().unwrap_or(' ') {
        'p' => 1e-12,
        'n' => 1e-9,
        'u' => 1e-6,
        'k' => 1e3,
        'm' => 1e6, // "4m7" resistor notation means 4.7 MΩ
        _ => 1.0,
    };
    Some((kind, base * multiplier))
}

fn net_of(board: &Board, refdes: &str, pin: usize) -> Option<String> {
    board
        .nets
        .iter()
        .find(|net| net.nodes.iter().any(|n| n.refdes == refdes && n.pad == pin))
        .map(|net| net.name.clone())
}

/// Build a linear circuit from board connectivity + agent models.
pub(crate) fn circuit_from_board(board: &Board, config: &AcConfig) -> (Circuit, Vec<String>) {
    let ground = board
        .nets
        .iter()
        .find(|n| n.class == crate::board::NetClass::Ground)
        .map(|n| n.name.clone())
        .unwrap_or_else(|| "0".into());
    // Ground is the reference node: it has no MNA unknown.
    let mut nets: Vec<String> = board
        .nets
        .iter()
        .filter(|n| n.name != ground)
        .map(|n| n.name.clone())
        .collect();
    let mut elements = Vec::new();
    let mut assumptions = Vec::new();

    for fp in &board.footprints {
        if let Some(model) = config.models.get(&fp.refdes) {
            match model {
                AcModel::Resistor { ohm } => {
                    if let (Some(a), Some(b)) = (net_of(board, &fp.refdes, 0), net_of(board, &fp.refdes, 1)) {
                        elements.push(Element::Passive(a, b, C::new(1.0 / ohm, 0.0)));
                    }
                }
                AcModel::Capacitor { farad } => {
                    // Filled per frequency below by the marker ` capacitance`.
                    if let (Some(a), Some(b)) = (net_of(board, &fp.refdes, 0), net_of(board, &fp.refdes, 1)) {
                        elements.push(Element::Passive(a, b, C::new(*farad, f64::NAN)));
                    }
                }
                AcModel::Inductor { henry } => {
                    if let (Some(a), Some(b)) = (net_of(board, &fp.refdes, 0), net_of(board, &fp.refdes, 1)) {
                        elements.push(Element::Passive(a, b, C::new(*henry, f64::INFINITY)));
                    }
                }
                AcModel::OpAmp {
                    plus_pin,
                    minus_pin,
                    out_pin,
                    gain,
                    input_impedance_ohm,
                } => {
                    let (p, m, out) = (
                        net_of(board, &fp.refdes, *plus_pin),
                        net_of(board, &fp.refdes, *minus_pin),
                        net_of(board, &fp.refdes, *out_pin),
                    );
                    if let (Some(p), Some(m), Some(out)) = (p, m, out) {
                        elements.push(Element::Passive(p.clone(), ground.clone(), C::new(1.0 / input_impedance_ohm, 0.0)));
                        elements.push(Element::Passive(m.clone(), ground.clone(), C::new(1.0 / input_impedance_ohm, 0.0)));
                        elements.push(Element::VcvS {
                            a: out,
                            b: ground.clone(),
                            p,
                            m,
                            gain: *gain,
                        });
                    }
                }
            }
        } else if let Some((kind, value)) = parse_value(&fp.value) {
            if let (Some(a), Some(b)) = (net_of(board, &fp.refdes, 0), net_of(board, &fp.refdes, 1)) {
                match kind {
                    'R' => elements.push(Element::Passive(a, b, C::new(1.0 / value, 0.0))),
                    'C' => elements.push(Element::Passive(a, b, C::new(value, f64::NAN))),
                    'L' => elements.push(Element::Passive(a, b, C::new(value, f64::INFINITY))),
                    _ => {}
                }
            }
        } else {
            assumptions.push(format!("{} ({}) has no AC model", fp.refdes, fp.value));
        }
    }

    let input = config.input_net.clone().or_else(|| {
        board
            .nets
            .iter()
            .find(|n| n.class == crate::board::NetClass::Input)
            .map(|n| n.name.clone())
    });
    let output = config.output_net.clone().or_else(|| {
        board
            .nets
            .iter()
            .find(|n| n.class == crate::board::NetClass::Output)
            .map(|n| n.name.clone())
    });
    if let Some(input) = &input {
        if !nets.contains(input) {
            nets.push(input.clone());
        }
        elements.push(Element::VSource(input.clone(), C::new(1.0, 0.0)));
    }
    if let (Some(output), Some(load)) = (&output, config.load_ohm) {
        if !nets.contains(output) {
            nets.push(output.clone());
        }
        elements.push(Element::Passive(output.clone(), ground.clone(), C::new(1.0 / load, 0.0)));
    }
    (Circuit { nets, elements, ground }, assumptions)
}

/// Resolve frequency-dependent admittances, then run the sweep.
pub fn analyze(board: &Board, config: &AcConfig) -> AcReport {
    let (circuit, mut assumptions) = circuit_from_board(board, config);
    assumptions.push("linear small-signal proxy: no transistor nonlinearity or parasitics".into());
    let input = config.input_net.clone().or_else(|| {
        board
            .nets
            .iter()
            .find(|n| n.class == crate::board::NetClass::Input)
            .map(|n| n.name.clone())
    });
    let output = config.output_net.clone().or_else(|| {
        board
            .nets
            .iter()
            .find(|n| n.class == crate::board::NetClass::Output)
            .map(|n| n.name.clone())
    });
    let mut points = Vec::new();
    let n = config.points.max(2);
    for i in 0..n {
        let t = i as f64 / (n - 1) as f64;
        let hz = config.f_start_hz * (config.f_end_hz / config.f_start_hz).powf(t);
        let omega = std::f64::consts::TAU * hz;
        let resolved: Vec<Element> = circuit
            .elements
            .iter()
            .map(|e| match e {
                Element::Passive(a, b, v) if v.im.is_nan() => {
                    Element::Passive(a.clone(), b.clone(), C::new(0.0, omega * v.re))
                }
                Element::Passive(a, b, v) if v.im.is_infinite() => Element::Passive(
                    a.clone(),
                    b.clone(),
                    C::new(0.0, -1.0 / (omega * v.re)),
                ),
                other => other.clone(),
            })
            .collect();
        let live = Circuit {
            nets: circuit.nets.clone(),
            elements: resolved,
            ground: circuit.ground.clone(),
        };
        let gain = if let (Some(input), Some(output)) = (&input, &output) {
            if let Some(solution) = live.solve(omega) {
                let vin = solution.get(input).copied().unwrap_or(C::new(1.0, 0.0));
                let vout = solution.get(output).copied().unwrap_or(C::new(0.0, 0.0));
                vout / vin
            } else {
                C::new(0.0, 0.0)
            }
        } else {
            C::new(0.0, 0.0)
        };
        points.push(AcPoint {
            hz,
            gain_db: if gain.abs() > 1e-12 {
                20.0 * gain.abs().log10()
            } else {
                -240.0
            },
            phase_deg: gain.arg_deg(),
        });
    }
    let dc_gain_db = points.first().map_or(0.0, |p| p.gain_db);
    let bandwidth_hz = points
        .iter()
        .find(|p| p.gain_db <= dc_gain_db - 3.0)
        .map(|p| p.hz);
    let phase_margin_deg = points
        .windows(2)
        .find(|w| w[0].gain_db >= 0.0 && w[1].gain_db < 0.0)
        .map(|w| 180.0 + w[1].phase_deg);
    AcReport {
        points,
        dc_gain_db,
        bandwidth_hz,
        phase_margin_deg,
        model: "proxy".into(),
        confidence: "low".into(),
        assumptions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{Footprint, LutPreset, Net, NetClass, Pad, PadRef};

    fn board() -> Board {
        let mut b = Board::new("ac-test", 60.0, 40.0, LutPreset::reliable());
        b.add_footprint(
            Footprint::new("R1", "1k", NetClass::Signal, 10.0, 10.0, 10.0, 4.0)
                .with_pads(vec![Pad::new(1.0, 2.0, 1.6, 0.8), Pad::new(9.0, 2.0, 1.6, 0.8)]),
        );
        b.add_footprint(
            Footprint::new("C1", "100n", NetClass::Signal, 30.0, 10.0, 10.0, 4.0)
                .with_pads(vec![Pad::new(1.0, 2.0, 1.6, 0.8), Pad::new(9.0, 2.0, 1.6, 0.8)]),
        );
        b.add_net(Net::new("IN", NetClass::Input, vec![PadRef::new("R1", 0)]));
        b.add_net(Net::new("OUT", NetClass::Output, vec![PadRef::new("R1", 1), PadRef::new("C1", 0)]));
        b.add_net(Net::new("GND", NetClass::Ground, vec![PadRef::new("C1", 1)]));
        b
    }

    #[test]
    fn complex_arithmetic_works() {
        let a = C::new(0.0, 1.0);
        assert!((a * a - C::new(-1.0, 0.0)).abs() < 1e-12);
        assert!((C::new(1.0, 0.0) / C::new(0.0, 1.0) - C::new(0.0, -1.0)).abs() < 1e-12);
    }

    #[test]
    fn rc_lowpass_cutoff_matches_analytic_value() {
        let config = AcConfig {
            input_net: Some("IN".into()),
            output_net: Some("OUT".into()),
            f_start_hz: 100.0,
            f_end_hz: 100_000.0,
            points: 80,
            load_ohm: Some(1e9),
            ..Default::default()
        };
        let report = analyze(&board(), &config);
        let measured = report.bandwidth_hz.expect("cutoff found");
        let expected = 1.0 / (std::f64::consts::TAU * 1e3 * 100e-9);
        assert!(
            (measured / expected - 1.0).abs() < 0.15,
            "measured {measured:.0} Hz, expected {expected:.0} Hz"
        );
        assert_eq!(report.model, "proxy");
    }

    #[test]
    fn opamp_macromodel_follower_has_unity_gain() {
        let mut b = Board::new("opamp-test", 60.0, 40.0, LutPreset::reliable());
        b.add_footprint(
            Footprint::new("U1", "LM358", NetClass::Signal, 20.0, 20.0, 14.0, 12.0).with_pads(
                vec![
                    Pad::new(2.0, 6.0, 1.6, 0.8),
                    Pad::new(6.0, 6.0, 1.6, 0.8),
                    Pad::new(10.0, 6.0, 1.6, 0.8),
                ],
            ),
        );
        b.add_net(Net::new("IN", NetClass::Input, vec![PadRef::new("U1", 0)]));
        b.add_net(Net::new("OUT", NetClass::Output, vec![PadRef::new("U1", 2), PadRef::new("U1", 1)]));
        b.add_net(Net::new("GND", NetClass::Ground, vec![]));
        let config = AcConfig {
            models: HashMap::from([(
                "U1".to_string(),
                AcModel::OpAmp {
                    plus_pin: 1,
                    minus_pin: 0,
                    out_pin: 2,
                    gain: 1e5,
                    input_impedance_ohm: 1e9,
                },
            )]),
            input_net: Some("IN".into()),
            output_net: Some("OUT".into()),
            f_start_hz: 10.0,
            f_end_hz: 10.0,
            points: 2,
            ..Default::default()
        };
        let report = analyze(&b, &config);
        assert!(report.dc_gain_db.abs() < 3.0, "gain {:.1} dB", report.dc_gain_db);
    }
}
