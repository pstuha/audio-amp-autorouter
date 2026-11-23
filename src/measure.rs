//! Module 8 — Sim/Measure: electrical and thermal measurements of routed copper.
//!
//! Answers three questions the router cannot answer by itself:
//!
//! * **voltage drop** — worst-case IR drop along each measured net;
//! * **heating** — I²R losses in the copper, fed into the existing thermal
//!   solver together with the component heat sources;
//! * **width by current** — the minimum track width for a given current, using
//!   the IPC-2221 external-track relation for 1 oz copper.
//!
//! This is a first-order DC model, not SPICE: every net's current is treated as
//! flowing through the full length of all its segments (a conservative upper
//! bound for branched nets), copper is assumed uniform, and the temperature
//! field is the steady-state 2-D model from [`crate::thermal`]. The module is
//! read-only: it never changes the board or the routing result.

use crate::board::NetClass;
use crate::router::RoutingResult;
use crate::thermal::{self, HeatSource, ThermalField, ThermalParams};
use serde::{Deserialize, Serialize};

const MIL2_TO_MM2: f64 = 0.000_645_16;
const IPC_EXPONENT: f64 = 0.725;
const IPC_RISE_EXPONENT: f64 = 0.44;
const EPS: f64 = 1e-9;

/// Copper properties and the limits a board must satisfy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CopperSpec {
    /// Copper foil thickness in micrometres (35 µm = 1 oz).
    pub thickness_um: f64,
    /// Resistivity at `reference_c`, ohm·metres (copper: 1.724e-8).
    pub resistivity_ohm_m: f64,
    /// Linear temperature coefficient per kelvin (copper: 0.00393).
    pub temp_coeff_per_k: f64,
    /// Temperature at which `resistivity_ohm_m` is quoted, °C.
    pub reference_c: f64,
    /// Assumed conductor temperature for the resistance calculation, °C.
    pub operating_c: f64,
    /// IPC-2221 constant for external tracks.
    pub ipc_k_external: f64,
    /// Allowed self-heating of a track used to size its width, °C.
    pub allowed_rise_c: f64,
    /// Maximum tolerated worst-case IR drop of a net, volts.
    pub max_voltage_drop_v: f64,
    /// Maximum tolerated board temperature rise above ambient, °C.
    pub max_temp_rise_c: f64,
}

impl Default for CopperSpec {
    fn default() -> Self {
        Self {
            thickness_um: 35.0,
            resistivity_ohm_m: 1.724e-8,
            temp_coeff_per_k: 0.00393,
            reference_c: 20.0,
            operating_c: 40.0,
            ipc_k_external: 0.048,
            allowed_rise_c: 10.0,
            max_voltage_drop_v: 0.1,
            max_temp_rise_c: 30.0,
        }
    }
}

impl CopperSpec {
    /// All physical constants and limits must be finite and sane.
    pub fn is_valid(&self) -> bool {
        let positive = [
            self.thickness_um,
            self.resistivity_ohm_m,
            self.ipc_k_external,
            self.allowed_rise_c,
            self.max_voltage_drop_v,
            self.max_temp_rise_c,
        ];
        positive.iter().all(|v| v.is_finite() && *v > 0.0)
            && self.temp_coeff_per_k.is_finite()
            && self.temp_coeff_per_k >= 0.0
            && self.reference_c.is_finite()
            && self.operating_c.is_finite()
    }

    /// Resistance of one rectangular copper strip at `operating_c`.
    pub fn track_resistance_ohm(&self, length_mm: f64, width_mm: f64) -> f64 {
        if length_mm <= 0.0 || !length_mm.is_finite() {
            return 0.0;
        }
        if width_mm <= 0.0 || !width_mm.is_finite() {
            return f64::INFINITY;
        }
        let rho = self.resistivity_ohm_m
            * (1.0 + self.temp_coeff_per_k * (self.operating_c - self.reference_c));
        let area_m2 = (width_mm * 1e-3) * (self.thickness_um * 1e-6);
        rho * (length_mm * 1e-3) / area_m2
    }

    pub fn voltage_drop_v(&self, current_a: f64, length_mm: f64, width_mm: f64) -> f64 {
        current_a * self.track_resistance_ohm(length_mm, width_mm)
    }

    /// Resistive power dissipated in one strip.
    pub fn track_loss_w(&self, current_a: f64, length_mm: f64, width_mm: f64) -> f64 {
        current_a * current_a * self.track_resistance_ohm(length_mm, width_mm)
    }

    /// Minimum copper width for `current_a`, from IPC-2221 external tracks:
    /// `A[mil²] = (I / (k · ΔT^0.44))^(1/0.725)`.
    pub fn required_width_mm(&self, current_a: f64) -> f64 {
        if !current_a.is_finite() || current_a <= 0.0 {
            return 0.0;
        }
        let mil2 = (current_a
            / (self.ipc_k_external * self.allowed_rise_c.powf(IPC_RISE_EXPONENT)))
        .powf(1.0 / IPC_EXPONENT);
        let area_mm2 = mil2 * MIL2_TO_MM2;
        area_mm2 / (self.thickness_um * 1e-3)
    }
}

/// Which measurement limit was broken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MeasureRule {
    Input,
    TrackWidth,
    VoltageDrop,
    Temperature,
}

impl MeasureRule {
    pub fn code(self) -> &'static str {
        match self {
            MeasureRule::Input => "M01_INPUT",
            MeasureRule::TrackWidth => "M02_TRACK_WIDTH",
            MeasureRule::VoltageDrop => "M03_VOLTAGE_DROP",
            MeasureRule::Temperature => "M04_TEMPERATURE",
        }
    }
}

/// One failed measurement limit.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MeasureViolation {
    pub rule: MeasureRule,
    pub net: String,
    pub value: f64,
    pub limit: f64,
    pub message: String,
}

impl std::fmt::Display for MeasureViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}: {}", self.rule.code(), self.net, self.message)
    }
}

/// Current assigned to a net for the DC measurement, in amperes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NetCurrent {
    pub net: String,
    /// Measured (RMS) current, in amperes.
    pub current_a: f64,
    /// Peak current the M02 width rule sizes for (sinusoidal audio ≈ 2× RMS);
    /// when 0, sizing uses `current_a`.
    #[serde(default)]
    pub peak_a: f64,
}

impl NetCurrent {
    pub fn new(net: &str, current_a: f64) -> Self {
        Self {
            net: net.into(),
            current_a,
            peak_a: 0.0,
        }
    }

    /// Set the peak current (falls back to `current_a` when 0).
    pub fn with_peak(mut self, peak_a: f64) -> Self {
        self.peak_a = peak_a;
        self
    }

    /// The current the M02 width rule sizes for: `peak_a` when given, else `current_a`.
    pub fn sizing_current_a(&self) -> f64 {
        if self.peak_a > 0.0 {
            self.peak_a
        } else {
            self.current_a
        }
    }
}

/// One frequency bin of an agent-supplied current spectrum.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpectrumPoint {
    pub hz: f64,
    /// RMS amps at this frequency.
    pub rms_a: f64,
}

/// Full electrical excitation of a net, supplied by the agent that designed
/// the circuit. This is the real-data counterpart to the class guesses used
/// when no excitation is given.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NetExcitation {
    pub net: String,
    /// Total RMS current in amperes.
    pub rms_a: f64,
    /// Peak current used for width sizing; falls back to `rms_a * 2`.
    #[serde(default)]
    pub peak_a: f64,
    /// Net that carries the return current (used by the EMI analysis).
    #[serde(default)]
    pub return_net: Option<String>,
    /// Spectral content. Empty means "flat / DC equivalent".
    #[serde(default)]
    pub spectrum: Vec<SpectrumPoint>,
    /// Source impedance in ohms, when known.
    #[serde(default)]
    pub source_impedance_ohm: f64,
}

impl NetExcitation {
    /// Conservative peak: explicit value, spectral sum × √2, or 2 × RMS.
    pub fn sizing_peak_a(&self) -> f64 {
        if self.peak_a > 0.0 {
            return self.peak_a;
        }
        if !self.spectrum.is_empty() {
            let rms: f64 = self.spectrum.iter().map(|p| p.rms_a.powi(2)).sum::<f64>().sqrt();
            if rms > 0.0 {
                return rms * 2.0_f64.sqrt();
            }
        }
        self.rms_a * 2.0
    }

    pub fn to_current(&self) -> NetCurrent {
        NetCurrent::new(&self.net, self.rms_a).with_peak(self.sizing_peak_a())
    }
}

/// Measurement of one net's copper.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NetMeasurement {
    pub net: String,
    pub class: NetClass,
    pub current_a: f64,
    pub segments: usize,
    pub copper_length_mm: f64,
    pub min_width_mm: f64,
    pub resistance_ohm: f64,
    pub voltage_drop_v: f64,
    pub loss_w: f64,
    pub required_width_mm: f64,
    /// Peak current the width rules were sized for (`NetCurrent::sizing_current_a`).
    pub peak_a: f64,
    /// Where the DC drop comes from: segment midpoints with per-segment drop.
    pub segment_drops: Vec<SegmentDrop>,
}

/// One segment's share of a net's DC drop.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SegmentDrop {
    /// Midpoint of the segment, board millimetres.
    pub x_mm: f64,
    /// Midpoint of the segment, board millimetres.
    pub y_mm: f64,
    /// Drop on this segment, volts.
    pub drop_v: f64,
}

/// Result of one measurement run.
#[derive(Debug, Serialize)]
pub struct MeasureReport {
    pub nets: Vec<NetMeasurement>,
    /// Steady-state temperature field including copper losses.
    pub field: Option<ThermalField>,
    pub violations: Vec<MeasureViolation>,
    pub total_loss_w: f64,
    pub log: Vec<String>,
    /// Smallest margin over the measured nets: `max_voltage_drop_v − drop`;
    /// positive = every net fits with headroom, negative = worst net over.
    pub drop_margin_v: f64,
    /// Peak temperature-rise margin: `max_temp_rise_c − peak_rise_c`.
    pub temp_margin_c: f64,
}

impl MeasureReport {
    pub fn is_ok(&self) -> bool {
        self.violations.is_empty()
    }

    pub fn passed(&self) -> bool {
        self.is_ok()
    }

    pub fn count(&self, rule: MeasureRule) -> usize {
        self.violations.iter().filter(|v| v.rule == rule).count()
    }

    pub fn peak_temp_rise_c(&self) -> f64 {
        self.field.as_ref().map(|f| f.peak_rise_c).unwrap_or(0.0)
    }

    pub fn summary(&self) -> String {
        format!(
            "{} net(s) measured, {:.3} W copper loss, {:.1} °C peak rise, {} violation(s) — {}",
            self.nets.len(),
            self.total_loss_w,
            self.peak_temp_rise_c(),
            self.violations.len(),
            if self.is_ok() { "PASS" } else { "FAIL" }
        )
    }
}

/// Configuration of a measurement run.
#[derive(Debug, Clone)]
pub struct MeasureConfig {
    pub copper: CopperSpec,
    pub thermal: ThermalParams,
    /// Add the component dissipation from `thermal::sources_for` to the field.
    pub include_component_heat: bool,
}

impl Default for MeasureConfig {
    fn default() -> Self {
        Self {
            copper: CopperSpec::default(),
            thermal: ThermalParams::default(),
            include_component_heat: true,
        }
    }
}

/// Read-only DC and thermal measurement engine.
#[derive(Debug, Clone)]
pub struct Measure {
    pub config: MeasureConfig,
}

impl Measure {
    pub fn new(config: MeasureConfig) -> Self {
        Self { config }
    }

    pub fn reliable() -> Self {
        Self::new(MeasureConfig::default())
    }

    /// Measure the requested nets of `routing`, then solve the temperature
    /// field with copper losses and (optionally) component dissipation.
    pub fn measure(
        &self,
        board: &crate::board::Board,
        routing: &RoutingResult,
        currents: &[NetCurrent],
    ) -> MeasureReport {
        let mut report = MeasureReport {
            nets: Vec::new(),
            field: None,
            violations: Vec::new(),
            total_loss_w: 0.0,
            log: Vec::new(),
            drop_margin_v: 0.0,
            temp_margin_c: 0.0,
        };
        if !self.config.copper.is_valid() {
            report.violations.push(MeasureViolation {
                rule: MeasureRule::Input,
                net: "-".into(),
                value: 0.0,
                limit: 0.0,
                message: "copper specification is not physically valid".into(),
            });
            report.log.push(report.summary());
            return report;
        }

        for request in currents {
            if !request.current_a.is_finite() || request.current_a < 0.0 {
                report.violations.push(MeasureViolation {
                    rule: MeasureRule::Input,
                    net: request.net.clone(),
                    value: request.current_a,
                    limit: 0.0,
                    message: format!(
                        "current {} A is not finite and non-negative",
                        request.current_a
                    ),
                });
                continue;
            }
            let Some(net) = board.nets.iter().find(|n| n.name == request.net) else {
                report.violations.push(MeasureViolation {
                    rule: MeasureRule::Input,
                    net: request.net.clone(),
                    value: request.current_a,
                    limit: 0.0,
                    message: "net does not exist on the board".into(),
                });
                continue;
            };
            let segments: Vec<&crate::router::RouteSegment> = routing
                .segments
                .iter()
                .filter(|s| s.net == request.net)
                .collect();
            if segments.is_empty() {
                report.violations.push(MeasureViolation {
                    rule: MeasureRule::Input,
                    net: request.net.clone(),
                    value: request.current_a,
                    limit: 0.0,
                    message: "net has no routed copper to measure".into(),
                });
                continue;
            }

            let copper_length_mm: f64 = segments.iter().map(|s| s.length_mm()).sum();
            let resistance_ohm: f64 = segments
                .iter()
                .map(|s| {
                    self.config
                        .copper
                        .track_resistance_ohm(s.length_mm(), s.width_mm)
                })
                .sum();
            let min_width_mm = segments
                .iter()
                .map(|s| s.width_mm)
                .fold(f64::INFINITY, f64::min);
            let voltage_drop_v = request.current_a * resistance_ohm;
            let loss_w = request.current_a * request.current_a * resistance_ohm;
            let sizing_current = request.sizing_current_a();
            let required_width_mm = self.config.copper.required_width_mm(sizing_current);
            report.total_loss_w += loss_w;

            if min_width_mm + EPS < required_width_mm {
                report.violations.push(MeasureViolation {
                    rule: MeasureRule::TrackWidth,
                    net: request.net.clone(),
                    value: min_width_mm,
                    limit: required_width_mm,
                    message: format!(
                        "narrowest copper {:.2} mm is below {:.2} mm required by {:.2} A",
                        min_width_mm, required_width_mm, request.current_a
                    ),
                });
            }
            if voltage_drop_v > self.config.copper.max_voltage_drop_v + EPS {
                report.violations.push(MeasureViolation {
                    rule: MeasureRule::VoltageDrop,
                    net: request.net.clone(),
                    value: voltage_drop_v,
                    limit: self.config.copper.max_voltage_drop_v,
                    message: format!(
                        "{:.3} V drop exceeds {:.3} V at {:.2} A",
                        voltage_drop_v, self.config.copper.max_voltage_drop_v, request.current_a
                    ),
                });
            }

            report.log.push(format!(
                "{}: {:.2} A, {:.1} mm, {:.3} Ω, {:.3} V, {:.3} W, min {:.2}/{:.2} mm",
                request.net,
                request.current_a,
                copper_length_mm,
                resistance_ohm,
                voltage_drop_v,
                loss_w,
                min_width_mm,
                required_width_mm
            ));
            let segment_drops: Vec<SegmentDrop> = segments
                .iter()
                .map(|s| SegmentDrop {
                    x_mm: (s.x1_mm + s.x2_mm) / 2.0,
                    y_mm: (s.y1_mm + s.y2_mm) / 2.0,
                    drop_v: self
                        .config
                        .copper
                        .voltage_drop_v(request.current_a, s.length_mm(), s.width_mm),
                })
                .collect();
            report.nets.push(NetMeasurement {
                net: request.net.clone(),
                class: net.class,
                current_a: request.current_a,
                segments: segments.len(),
                copper_length_mm,
                min_width_mm,
                resistance_ohm,
                voltage_drop_v,
                loss_w,
                required_width_mm,
                peak_a: sizing_current,
                segment_drops,
            });
        }

        // Margins: the worst headroom over all measured nets.
        let mut drop_margin_v = f64::INFINITY;
        for measurement in &report.nets {
            drop_margin_v =
                drop_margin_v.min(self.config.copper.max_voltage_drop_v - measurement.voltage_drop_v);
        }
        if drop_margin_v == f64::INFINITY {
            drop_margin_v = 0.0; // nothing was measured
        }
        report.drop_margin_v = drop_margin_v;

        let mut sources = Vec::new();
        if self.config.include_component_heat {
            sources.extend(thermal::sources_for(board));
        }
        for measurement in &report.nets {
            for segment in routing.segments.iter().filter(|s| s.net == measurement.net) {
                let (x, y) = (
                    (segment.x1_mm + segment.x2_mm) / 2.0,
                    (segment.y1_mm + segment.y2_mm) / 2.0,
                );
                let power_w = self.config.copper.track_loss_w(
                    measurement.current_a,
                    segment.length_mm(),
                    segment.width_mm,
                );
                if !power_w.is_finite() || power_w <= 0.0 {
                    continue;
                }
                sources.push(HeatSource {
                    refdes: segment.net.clone(),
                    x,
                    y,
                    power_w,
                    spread_mm: (segment.width_mm * 3.0).max(1.0),
                });
            }
        }

        let field = thermal::solve(board, &sources, &self.config.thermal);
        report.temp_margin_c = self.config.copper.max_temp_rise_c - field.peak_rise_c;
        if field.peak_rise_c > self.config.copper.max_temp_rise_c + EPS {
            report.violations.push(MeasureViolation {
                rule: MeasureRule::Temperature,
                net: "-".into(),
                value: field.peak_rise_c,
                limit: self.config.copper.max_temp_rise_c,
                message: format!(
                    "board rises {:.1} °C, limit {:.1} °C",
                    field.peak_rise_c, self.config.copper.max_temp_rise_c
                ),
            });
        }
        report.field = Some(field);
        report.log.push(report.summary());
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{Board, Footprint, LutPreset, Net, NetClass, Pad, PadRef};
    use crate::router::{RouteSegment, RoutingResult};

    fn sample_board() -> Board {
        let mut board = Board::new("measure-test", 80.0, 60.0, LutPreset::reliable());
        let fp =
            Footprint::new("R1", "10k", NetClass::Signal, 10.0, 30.0, 24.0, 2.0).with_pads(vec![
                Pad::new(0.0, 0.0, 1.6, 0.8),
                Pad::new(20.0, 0.0, 1.6, 0.8),
            ]);
        board.add_footprint(fp);
        board.add_net(Net::new(
            "SIG",
            NetClass::Signal,
            vec![PadRef::new("R1", 0), PadRef::new("R1", 1)],
        ));
        board
    }

    fn routing(segments: Vec<RouteSegment>) -> RoutingResult {
        RoutingResult {
            total_length_mm: segments.iter().map(|s| s.length_mm()).sum(),
            segments,
            routed_nets: vec!["SIG".into()],
            failed_nets: Vec::new(),
            vias: 0,
            via_objects: Vec::new(),
        }
    }

    fn long_track() -> RoutingResult {
        routing(vec![RouteSegment::new(
            10.0,
            30.0,
            30.0,
            30.0,
            "SIG",
            NetClass::Signal,
        )])
    }

    // --- copper model ------------------------------------------------------

    #[test]
    fn default_spec_is_physical() {
        assert!(CopperSpec::default().is_valid());
    }

    #[test]
    fn spec_rejects_bad_constants() {
        let base = CopperSpec::default();
        assert!(!CopperSpec {
            thickness_um: 0.0,
            ..base.clone()
        }
        .is_valid());
        assert!(!CopperSpec {
            resistivity_ohm_m: f64::NAN,
            ..base.clone()
        }
        .is_valid());
        assert!(!CopperSpec {
            temp_coeff_per_k: -0.1,
            ..base.clone()
        }
        .is_valid());
        assert!(!CopperSpec {
            max_voltage_drop_v: 0.0,
            ..base
        }
        .is_valid());
    }

    #[test]
    fn resistance_matches_ohm_law_for_copper_foil() {
        let spec = CopperSpec::default();
        // 100 mm of 0.4 mm, 35 um copper: R = rho·L/A, ~0.13 ohm at 40 C.
        let r = spec.track_resistance_ohm(100.0, 0.4);
        assert!((r - 0.128).abs() < 0.005, "R={r}");
        assert_eq!(spec.track_resistance_ohm(0.0, 0.4), 0.0);
        assert_eq!(spec.track_resistance_ohm(10.0, 0.0), f64::INFINITY);
    }

    #[test]
    fn temperature_raises_resistance() {
        let spec = CopperSpec::default();
        let cold = spec.track_resistance_ohm(100.0, 0.4);
        let hot_spec = CopperSpec {
            operating_c: 100.0,
            ..spec.clone()
        };
        let hot = hot_spec.track_resistance_ohm(100.0, 0.4);
        assert!(hot > cold);
        assert!((hot / cold - 1.219).abs() < 0.01);
    }

    #[test]
    fn voltage_drop_and_loss_follow_the_current() {
        let spec = CopperSpec::default();
        let r = spec.track_resistance_ohm(100.0, 0.4);
        assert!((spec.voltage_drop_v(2.0, 100.0, 0.4) - 2.0 * r).abs() < 1e-12);
        assert!((spec.track_loss_w(2.0, 100.0, 0.4) - 4.0 * r).abs() < 1e-12);
        assert_eq!(spec.voltage_drop_v(0.0, 100.0, 0.4), 0.0);
    }

    #[test]
    fn required_width_grows_with_current() {
        let spec = CopperSpec::default();
        let one_amp = spec.required_width_mm(1.0);
        let two_amp = spec.required_width_mm(2.0);
        assert!((one_amp - 0.30).abs() < 0.05, "{one_amp}");
        assert!(two_amp > one_amp);
        assert_eq!(spec.required_width_mm(0.0), 0.0);
        assert_eq!(spec.required_width_mm(f64::NAN), 0.0);
    }

    // --- measurement -------------------------------------------------------

    #[test]
    fn clean_net_is_measured_without_violations() {
        let board = sample_board();
        let report =
            Measure::reliable().measure(&board, &long_track(), &[NetCurrent::new("SIG", 0.05)]);
        assert!(report.is_ok(), "{}", report.violations[0]);
        assert!(report.passed());
        assert_eq!(report.nets.len(), 1);
        assert_eq!(report.nets[0].class, NetClass::Signal);
        assert_eq!(report.nets[0].segments, 1);
        assert!((report.nets[0].copper_length_mm - 20.0).abs() < 1e-9);
        assert!(report.nets[0].resistance_ohm > 0.0);
        assert!(report.field.is_some());
        assert!(report.peak_temp_rise_c() >= 0.0);
        assert!(report.summary().contains("PASS"));
    }

    #[test]
    fn invalid_spec_is_reported_before_measurement() {
        let config = MeasureConfig {
            copper: CopperSpec {
                thickness_um: -1.0,
                ..CopperSpec::default()
            },
            ..MeasureConfig::default()
        };
        let report = Measure::new(config).measure(
            &sample_board(),
            &long_track(),
            &[NetCurrent::new("SIG", 0.1)],
        );
        assert_eq!(report.count(MeasureRule::Input), 1);
        assert!(report.nets.is_empty());
        assert!(report.field.is_none());
    }

    #[test]
    fn invalid_currents_and_unknown_nets_are_refused() {
        let board = sample_board();
        let routed = long_track();
        let measure = Measure::reliable();
        let cases = [
            NetCurrent::new("SIG", -1.0),
            NetCurrent::new("SIG", f64::NAN),
            NetCurrent::new("NOPE", 0.1),
        ];
        for case in cases {
            let report = measure.measure(&board, &routed, &[case]);
            assert_eq!(report.count(MeasureRule::Input), 1);
            assert!(report.nets.is_empty());
        }
    }

    #[test]
    fn net_without_copper_is_refused() {
        let report = Measure::reliable().measure(
            &sample_board(),
            &routing(Vec::new()),
            &[NetCurrent::new("SIG", 0.1)],
        );
        assert_eq!(report.count(MeasureRule::Input), 1);
        assert!(report.violations[0].message.contains("no routed copper"));
    }

    #[test]
    fn narrow_track_is_flagged() {
        let mut routed = long_track();
        routed.segments[0] = routed.segments[0].clone().with_width(0.2);
        let report =
            Measure::reliable().measure(&sample_board(), &routed, &[NetCurrent::new("SIG", 1.0)]);
        assert_eq!(report.count(MeasureRule::TrackWidth), 1);
        assert!(report.violations[0].value < report.violations[0].limit);
        assert_eq!(report.nets[0].min_width_mm, 0.2);
    }

    #[test]
    fn excessive_voltage_drop_is_flagged() {
        let report = Measure::reliable().measure(
            &sample_board(),
            &long_track(),
            &[NetCurrent::new("SIG", 5.0)],
        );
        assert_eq!(report.count(MeasureRule::VoltageDrop), 1);
        let drop = report
            .violations
            .iter()
            .find(|v| v.rule == MeasureRule::VoltageDrop)
            .unwrap();
        assert!(drop.value > 0.1);
        assert!(drop.to_string().contains("M03_VOLTAGE_DROP"));
    }

    #[test]
    fn peak_current_drives_the_width_rule() {
        let mut routed = long_track();
        routed.segments[0] = routed.segments[0].clone().with_width(0.2);
        // RMS-only sizing: the IPC-2221 required width is far below 0.2 mm.
        let rms_only = Measure::reliable().measure(
            &sample_board(),
            &routed,
            &[NetCurrent::new("SIG", 0.05)],
        );
        assert_eq!(rms_only.count(MeasureRule::TrackWidth), 0);
        assert_eq!(rms_only.nets[0].peak_a, 0.05);
        // A 2 A peak requires a wider track although the RMS current is unchanged.
        let peaked = Measure::reliable().measure(
            &sample_board(),
            &routed,
            &[NetCurrent::new("SIG", 0.05).with_peak(2.0)],
        );
        assert_eq!(peaked.count(MeasureRule::TrackWidth), 1);
        assert_eq!(peaked.nets[0].peak_a, 2.0);
        assert!(peaked.nets[0].required_width_mm > 0.2);
    }

    #[test]
    fn drop_margins_are_reported() {
        let report = Measure::reliable().measure(
            &sample_board(),
            &long_track(),
            &[NetCurrent::new("SIG", 0.05)],
        );
        assert_eq!(report.nets.len(), 1);
        // Drop margin: limit minus the measured drop of the clean net.
        assert!((report.drop_margin_v - (0.1 - report.nets[0].voltage_drop_v)).abs() < 1e-9);
        assert!(report.drop_margin_v > 0.0);
        // Temperature margin: limit minus peak rise.
        assert!((report.temp_margin_c - (30.0 - report.peak_temp_rise_c())).abs() < 1e-9);
    }

    #[test]
    fn segment_drops_are_recorded() {
        let report = Measure::reliable().measure(
            &sample_board(),
            &long_track(),
            &[NetCurrent::new("SIG", 0.05)],
        );
        let drops = &report.nets[0].segment_drops;
        assert_eq!(drops.len(), 1);
        // The segment spans (10, 30) → (30, 30).
        assert!((drops[0].x_mm - 20.0).abs() < 1e-9);
        assert!((drops[0].y_mm - 30.0).abs() < 1e-9);
        assert!((drops[0].drop_v - report.nets[0].voltage_drop_v).abs() < 1e-12);
        assert!(drops[0].drop_v > 0.0);
    }

    #[test]
    fn temperature_limit_is_audited() {
        let config = MeasureConfig {
            copper: CopperSpec {
                max_temp_rise_c: 0.001,
                ..CopperSpec::default()
            },
            ..MeasureConfig::default()
        };
        let report = Measure::new(config).measure(
            &sample_board(),
            &long_track(),
            &[NetCurrent::new("SIG", 0.1)],
        );
        assert_eq!(report.count(MeasureRule::Temperature), 1);
        assert!(report.violations[0].value > report.violations[0].limit);
    }

    #[test]
    fn component_heat_can_be_excluded() {
        let mut board = sample_board();
        let hot = Footprint::new("U1", "TDA2030", NetClass::Output, 40.0, 15.0, 10.0, 10.0)
            .with_pads(vec![Pad::new(0.0, 0.0, 1.6, 0.8)]);
        board.add_footprint(hot);
        let config = MeasureConfig {
            include_component_heat: false,
            ..MeasureConfig::default()
        };
        let without = Measure::new(config).measure(&board, &long_track(), &[]);
        let with = Measure::reliable().measure(&board, &long_track(), &[]);
        assert!(with.peak_temp_rise_c() > without.peak_temp_rise_c());
    }

    #[test]
    fn multiple_nets_accumulate_loss_and_log() {
        let mut board = sample_board();
        board.add_net(Net::new("AUX", NetClass::Power, vec![PadRef::new("R1", 1)]));
        let mut routed = long_track();
        routed.segments.push(RouteSegment::new(
            10.0,
            20.0,
            30.0,
            20.0,
            "AUX",
            NetClass::Power,
        ));
        let report = Measure::reliable().measure(
            &board,
            &routed,
            &[NetCurrent::new("SIG", 0.05), NetCurrent::new("AUX", 0.05)],
        );
        assert_eq!(report.nets.len(), 2);
        assert!(report.total_loss_w > 0.0);
        assert!(
            (report.total_loss_w - report.nets.iter().map(|n| n.loss_w).sum::<f64>()).abs() < 1e-12
        );
        assert!(report.log.len() >= 3);
    }

    #[test]
    fn measurement_does_not_mutate_inputs() {
        let board = sample_board();
        let routed = long_track();
        let before = board.to_json().unwrap();
        let before_segments = routed.segments.len();
        let _ = Measure::reliable().measure(&board, &routed, &[NetCurrent::new("SIG", 0.05)]);
        assert_eq!(board.to_json().unwrap(), before);
        assert_eq!(routed.segments.len(), before_segments);
    }

    #[test]
    fn report_helpers_are_stable() {
        let report = Measure::reliable().measure(
            &sample_board(),
            &long_track(),
            &[NetCurrent::new("SIG", 0.05)],
        );
        assert_eq!(report.count(MeasureRule::VoltageDrop), 0);
        assert_eq!(report.count(MeasureRule::Temperature), 0);
        assert_eq!(
            report.summary(),
            format!(
                "1 net(s) measured, {:.3} W copper loss, {:.1} °C peak rise, 0 violation(s) — PASS",
                report.total_loss_w,
                report.peak_temp_rise_c()
            )
        );
        assert_eq!(MeasureRule::Input.code(), "M01_INPUT");
        assert_eq!(MeasureRule::TrackWidth.code(), "M02_TRACK_WIDTH");
    }

    #[test]
    fn excitation_spectrum_produces_a_conservative_peak() {
        let e = NetExcitation {
            net: "OUT".into(),
            rms_a: 0.5,
            peak_a: 0.0,
            return_net: Some("GND".into()),
            spectrum: vec![
                SpectrumPoint { hz: 100.0, rms_a: 0.3 },
                SpectrumPoint { hz: 1000.0, rms_a: 0.4 },
            ],
            source_impedance_ohm: 8.0,
        };
        let peak = e.sizing_peak_a();
        assert!((peak - 0.5 * 2.0_f64.sqrt()).abs() < 1e-9);
        let current = e.to_current();
        assert_eq!(current.net, "OUT");
        assert!((current.sizing_current_a() - peak).abs() < 1e-9);
    }

    #[test]
    fn explicit_peak_overrides_the_spectrum() {
        let e = NetExcitation {
            net: "V+".into(),
            rms_a: 0.2,
            peak_a: 0.9,
            return_net: None,
            spectrum: vec![SpectrumPoint { hz: 50.0, rms_a: 1.0 }],
            source_impedance_ohm: 0.0,
        };
        assert!((e.sizing_peak_a() - 0.9).abs() < 1e-9);
    }
}
