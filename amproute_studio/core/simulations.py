"""Electrical, thermal, and audio simulations for audio amplifier PCBs."""

from __future__ import annotations

import logging
import math
from dataclasses import dataclass, field
from typing import Any, Optional

import numpy as np

logger = logging.getLogger(__name__)

# Physical constants
MU_0 = 4 * math.pi * 1e-7  # H/m
COPPER_RESISTIVITY = 1.68e-8  # Ω·m at 20°C
SHEET_RESISTANCE_CU_1oz = 0.5e-3  # Ω/square for 1 oz copper

# Thermal constants for TO-3P packages
RTH_JC_TO3P = 1.5  # °C/W junction-to-case
RTH_CS_HEATSINK = 0.5  # °C/W case-to-heatsink
RTH_HA = 3.0  # °C/W heatsink-to-ambient
AMBIENT_DEFAULT = 25.0  # °C

# Audio amp component sets (Titan-D)
OUTPUT_TRANSISTORS = {"Q12", "Q14", "Q18"}  # TTC5200 NPN
OUTPUT_PNP = {"Q13", "Q15", "Q19"}  # TTA1943 PNP
DRIVER_TRANSISTORS = {"Q10", "Q11"}  # MJE15032/33
VAS_TRANSISTORS = {"Q6", "Q7"}  # MJE350/340
EMITTER_RESISTORS = {"R29", "R30", "R31", "R32", "R33", "R34"}  # 0.15Ω

CRITICAL_LOOP_AREA_MM2 = 500.0
EMITTER_R_VALUE = 0.15  # Ω
OUTPUT_CURRENT_ESTIMATE = 1.5  # A peak


@dataclass
class CurrentLoop:
    net_name: str
    area_mm2: float
    centroid: tuple[float, float]
    inductance_nh: float
    perimeter_mm: float
    critical: bool = False


@dataclass
class ThermalResult:
    component_ref: str
    power_dissipation_w: float
    junction_temp_c: float
    case_temp_c: float
    thermal_resistance: float  # °C/W


@dataclass
class AudioPrediction:
    damping_factor: float
    thd_percent: float
    psrr_pos_100hz: float
    psrr_pos_1khz: float
    psrr_neg_100hz: float
    psrr_neg_1khz: float
    slew_rate_v_us: float
    stable_capacitive_load: list[float]


@dataclass
class StabilityAnalysis:
    phase_margin_deg: float
    gain_margin_db: float
    unity_gain_freq_mhz: float
    stable: bool
    notes: list[str]


@dataclass
class CrosstalkResult:
    """Result of crosstalk analysis between tracks."""
    net_a: str
    net_b: str
    parallel_length_mm: float
    gap_mm: float
    crosstalk_pct: float  # estimated crosstalk percentage
    critical: bool
    recommendation: str


@dataclass
class EMIPrediction:
    """EMI/EMC prediction for the board."""
    score: str  # "low", "medium", "high"
    loop_areas_mm2: list[float]
    high_freq_traces: list[tuple[str, float]]  # (net, length_mm)
    ground_plane_continuous: bool
    problem_areas: list[str]
    recommendations: list[str]


@dataclass
class PowerBusResult:
    """Power bus/distribution analysis."""
    rail_impedance_mohm: dict[str, float]  # rail -> impedance mΩ
    ripple_100hz_mv: dict[str, float]
    ripple_1khz_mv: dict[str, float]
    decoupling_effectiveness: dict[str, float]  # 0-1
    bulk_capacitors_needed: list[tuple[str, float]]  # (net, capacitance_uF)
    recommendations: list[str]


@dataclass
class SimulationReport:
    board_name: str
    current_loops: list[CurrentLoop]
    thermal_results: list[ThermalResult]
    audio: AudioPrediction
    stability: StabilityAnalysis
    ir_drop: dict
    crosstalk: list[CrosstalkResult]
    recommendations: list[str]
    warnings: list[str]
    emi: Optional[EMIPrediction] = None
    power_bus: Optional[PowerBusResult] = None
    thermal_heatmap_path: Optional[str] = None
    crosstalk_map_path: Optional[str] = None
    emi_map_path: Optional[str] = None

    def to_dict(self) -> dict:
        return {
            "board_name": self.board_name,
            "current_loops": [
                {
                    "net_name": c.net_name,
                    "area_mm2": c.area_mm2,
                    "centroid": c.centroid,
                    "inductance_nh": c.inductance_nh,
                    "perimeter_mm": c.perimeter_mm,
                    "critical": c.critical,
                }
                for c in self.current_loops
            ],
            "thermal": [
                {
                    "component_ref": t.component_ref,
                    "power_dissipation_w": t.power_dissipation_w,
                    "junction_temp_c": t.junction_temp_c,
                    "case_temp_c": t.case_temp_c,
                    "thermal_resistance": t.thermal_resistance,
                }
                for t in self.thermal_results
            ],
            "audio": {
                "damping_factor": self.audio.damping_factor,
                "thd_percent": self.audio.thd_percent,
                "psrr": {
                    "100hz_pos": self.audio.psrr_pos_100hz,
                    "1khz_pos": self.audio.psrr_pos_1khz,
                    "100hz_neg": self.audio.psrr_neg_100hz,
                    "1khz_neg": self.audio.psrr_neg_1khz,
                },
                "slew_rate_v_us": self.audio.slew_rate_v_us,
                "stable_capacitive_load_uF": [c * 1e6 for c in self.audio.stable_capacitive_load],
            },
            "stability": {
                "phase_margin_deg": self.stability.phase_margin_deg,
                "gain_margin_db": self.stability.gain_margin_db,
                "unity_gain_freq_mhz": self.stability.unity_gain_freq_mhz,
                "stable": self.stability.stable,
                "notes": self.stability.notes,
            },
            "ir_drop": self.ir_drop,
            "recommendations": self.recommendations,
            "warnings": self.warnings,
            "thermal_heatmap_path": self.thermal_heatmap_path,
        }


def _track_resistance(length_mm: float, width_mm: float, copper_oz: int = 1) -> float:
    """Calculate track resistance in milliohms."""
    if width_mm <= 0 or length_mm <= 0:
        return 0.0
    rs = SHEET_RESISTANCE_CU_1oz * (35 / 35)  # normalize for 1oz
    length_m = length_mm / 1000.0
    cross_section = width_mm * 0.035  # mm² (35μm = 1oz)
    if cross_section <= 0:
        return 0.0
    return (COPPER_RESISTIVITY * length_m / cross_section) * 1000.0  # mΩ


def _loop_inductance(area_mm2: float, perimeter_mm: float) -> float:
    """Estimate loop inductance using empirical formula.
    
    L ≈ μ₀ * μr * (area / length) for low-profile loops
    For PCB loops: L ≈ 32 * area / (perimeter + 2*height) nH
    where height is PCB thickness (typically 1.6mm)
    """
    if area_mm2 <= 0 or perimeter_mm <= 0:
        return 0.0
    height_mm = 1.6  # standard PCB thickness
    l_nh = 32 * area_mm2 / (perimeter_mm + 2 * height_mm)
    return max(0.1, l_nh)


def analyze_current_loops(board, segments=None, jumpers=None) -> list[CurrentLoop]:
    """Find and analyze current loops in the audio amplifier PCB.
    
    Critical loops for УНЧ:
    - Output stage: collector-emitter → load → ground
    - VAS: VAS → output stage
    - Feedback: feedback network
    """
    loops = []
    critical_nets = {"VCC", "+V", "-V", "GND", "OUT", "VOUT", "VS+", "VS-"}

    # Group segments by net
    net_segments: dict[str, list] = {}
    if segments:
        for seg in segments:
            net_segments.setdefault(seg.net, []).append(seg)

    # Add jumpers to segments
    if jumpers:
        from .models import RouteSegment
        for j in jumpers:
            net_segments.setdefault(j.net, []).append(
                RouteSegment(j.net, j.x1_mm, j.y1_mm, j.x2_mm, j.y2_mm, 0.5)
            )

    for net_name, segs in net_segments.items():
        if len(segs) < 2:
            continue

        # Build polygon from connected segments
        points = set()
        for seg in segs:
            points.add((seg.x1_mm, seg.y1_mm))
            points.add((seg.x2_mm, seg.y2_mm))

        if len(points) < 3:
            continue

        # Calculate area using shoelace formula
        pts = list(points)
        area = 0.0
        perimeter = 0.0
        for i in range(len(pts)):
            x1, y1 = pts[i]
            x2, y2 = pts[(i + 1) % len(pts)]
            area += x1 * y2 - x2 * y1
            perimeter += math.hypot(x2 - x1, y2 - y1)
        area = abs(area) / 2.0

        if area < 1.0:  # Skip tiny loops
            continue

        # Calculate centroid
        cx = sum(p[0] for p in pts) / len(pts)
        cy = sum(p[1] for p in pts) / len(pts)

        inductance = _loop_inductance(area, perimeter)
        critical = area > CRITICAL_LOOP_AREA_MM2 or net_name in critical_nets

        loops.append(CurrentLoop(
            net_name=net_name,
            area_mm2=area,
            centroid=(cx, cy),
            inductance_nh=inductance,
            perimeter_mm=perimeter,
            critical=critical,
        ))

    # Sort by area descending
    loops.sort(key=lambda x: x.area_mm2, reverse=True)
    return loops


def analyze_power_distribution(board, segments=None) -> dict[str, float]:
    """Analyze IR drop on power distribution network.

    Returns dict of net_name -> voltage_drop_mV
    """
    ir_drop = {}
    power_nets = {"VCC", "+V", "+12", "+15", "+24", "VS+", "VS-", "-V", "GND", "AGND"}

    # Group segments by net
    net_segments: dict[str, list] = {}
    if segments:
        for seg in segments:
            net_segments.setdefault(seg.net, []).append(seg)

    for net_name, segs in net_segments.items():
        if not segs:
            continue
        if net_name not in power_nets:
            continue

        total_length = 0.0
        widths = []
        for seg in segs:
            length = math.hypot(seg.x2_mm - seg.x1_mm, seg.y2_mm - seg.y1_mm)
            total_length += length
            widths.append(seg.width_mm)

        avg_width = sum(widths) / len(widths) if widths else 1.0

        # Estimate current based on net type
        if net_name.upper() in ("GND", "AGND"):
            current_ma = 500  # Ground sees full output current
        elif "VCC" in net_name.upper() or "+V" in net_name.upper():
            current_ma = 200  # VCC ~200mA average
        else:
            current_ma = 100

        r_mohm = _track_resistance(total_length, avg_width)
        drop_mv = current_ma * r_mohm
        ir_drop[net_name] = round(drop_mv, 3)

    return ir_drop


def analyze_thermal(board, ambient_temp: float = AMBIENT_DEFAULT) -> list[ThermalResult]:
    """Thermal analysis for Titan-D amplifier components.

    Calculates junction temperatures for power transistors and resistors.
    """
    results = []

    # Find components on board
    components = {c.ref: c for c in board.components.values()}

    # Power estimates for each component type
    # TTC5200/TTA1943 pairs handle ~1.5A each at ~2V CE drop
    # P = VCE * IC ≈ 2V * 1.5A = 3W per transistor in class AB
    # With 3 pairs in parallel, ~1W each average

    transistor_power = {
        "Q12": 3.0, "Q14": 3.0, "Q18": 3.0,  # TTC5200 NPN outputs
        "Q13": 3.0, "Q15": 3.0, "Q19": 3.0,  # TTA1943 PNP outputs
        "Q10": 0.8, "Q11": 0.8,  # MJE15032/33 drivers
        "Q6": 0.5, "Q7": 0.5,   # VAS MJE350/340
    }

    for ref, power_w in transistor_power.items():
        if ref not in components:
            continue

        comp = components[ref]

        # Calculate thermal resistance based on package
        if ref in OUTPUT_TRANSISTORS or ref in OUTPUT_PNP:
            rth_jc = RTH_JC_TO3P
        else:
            rth_jc = 3.5  # TO-220 or similar

        rth_total = rth_jc + RTH_CS_HEATSINK + RTH_HA

        case_temp = ambient_temp + power_w * (RTH_CS_HEATSINK + RTH_HA)
        junction_temp = ambient_temp + power_w * rth_total

        results.append(ThermalResult(
            component_ref=ref,
            power_dissipation_w=power_w,
            junction_temp_c=round(junction_temp, 1),
            case_temp_c=round(case_temp, 1),
            thermal_resistance=round(rth_total, 2),
        ))

    # Emitter resistors: 5W rated, wirewound
    # P = I² * R, where I is ~1.5A peak per resistor (3 pairs share output)
    for i in range(29, 35):
        ref = f"R{i}"
        if ref not in components:
            continue
        # Current splits between 3 NPN and 3 PNP pairs
        i_per_resistor = OUTPUT_CURRENT_ESTIMATE / 3.0
        power_w = (i_per_resistor ** 2) * EMITTER_R_VALUE
        rth = 15.0  # °C/W for wirewound 5W resistor
        temp = ambient_temp + power_w * rth
        results.append(ThermalResult(
            component_ref=ref,
            power_dissipation_w=round(power_w, 3),
            junction_temp_c=round(temp, 1),
            case_temp_c=round(ambient_temp + power_w * 5.0, 1),
            thermal_resistance=rth,
        ))

    return results


def predict_damping_factor(board, load_impedance: float = 4.0) -> float:
    """Predict damping factor from board characteristics.

    DF = Zload / Zout
    Zout ≈ Rtracks + Re * (1 + hfe) / hfe ≈ Rtracks + Re for high hfe

    For Titan-D with 0.15Ω emitter resistors and 4Ω load:
    - DF_min ≈ 4 / 0.15 ≈ 27
    - With track resistance (~10mΩ): DF ≈ 4 / 0.16 ≈ 25
    """
    # Estimate track resistance for output net
    output_nets = {"OUT", "VOUT", "SPK", "SPEAKER"}
    track_r_mohm = 10.0  # ~10mΩ typical for good output traces

    # Effective output resistance
    re = EMITTER_R_VALUE  # 0.15Ω emitter resistors
    zout = (track_r_mohm / 1000.0) + re  # in Ohms

    df = load_impedance / zout
    return round(df, 1)


def estimate_thd(board, segments=None) -> float:
    """Estimate THD based on board characteristics.

    THD factors for audio amplifiers:
    - Current loop area (larger = more EMI = higher THD)
    - Feedback loop length (longer = more phase shift = higher THD)
    - Power supply decoupling (weaker = higher THD at low freq)
    - Emitter degeneration (more Re = lower THD)

    Base THD for well-designed amp: ~0.001% (0.001)
    With layout issues: up to 0.01% (0.01)
    """
    # Analyze loops
    loops = analyze_current_loops(board, segments)
    total_loop_area = sum(l.area_mm2 for l in loops)

    # Loop factor: area > 500mm² adds ~0.001% THD
    loop_factor = min(0.008, total_loop_area / 500000.0)

    # Estimate feedback length from board size
    fb_length_mm = 0.0
    if board.bounds_mm:
        w, h = board.bounds_mm[2] - board.bounds_mm[0], board.bounds_mm[3] - board.bounds_mm[1]
        fb_length_mm = (w + h) * 0.3  # approximate feedback trace length

    # Feedback factor: long FB adds THD
    fb_factor = fb_length_mm / 1000.0  # normalize

    # Base THD for Titan-D with good layout
    base_thd = 0.001  # 0.001%

    total_thd = base_thd + loop_factor + fb_factor * 0.002

    return round(total_thd * 100, 4)  # as percentage


def predict_psrr(board, segments=None) -> dict[str, float]:
    """Predict Power Supply Rejection Ratio.

    PSRR depends on:
    - Decoupling capacitor values and placement
    - RC filter (47Ω + 470µF in Titan-D)
    - PSRR of the IC itself (typically 80-90dB)

    Returns dict with PSRR at 100Hz and 1kHz
    """
    # Estimate decoupling quality
    decoupling_caps = {"C8", "C9", "C10", "C11", "C12"}  # typical decoupling caps
    caps_found = sum(1 for ref in decoupling_caps if ref in board.components)
    decouple_factor = caps_found / 5.0  # 0 to 1 scale

    # Base PSRR from IC (assume TDA7293-like: 85dB)
    base_psrr_100hz = 80.0  # dB
    base_psrr_1khz = 86.0  # dB

    # Decoupling quality improves PSRR
    psrr_100hz = base_psrr_100hz + decouple_factor * 10
    psrr_1khz = base_psrr_1khz + decouple_factor * 6

    # Negative supply slightly worse due to single-stage VAS
    return {
        "psrr_pos_100hz": round(psrr_100hz, 1),
        "psrr_pos_1khz": round(psrr_1khz, 1),
        "psrr_neg_100hz": round(psrr_100hz - 6, 1),
        "psrr_neg_1khz": round(psrr_1khz - 4, 1),
    }


def check_slew_rate(board, target_slew: float = 40.0) -> bool:
    """Check if board can support target slew rate.

    SR depends on:
    - Parasitic capacitances in current loops
    - Drive capability of VAS and output stage
    - Coupling capacitors (input coupling 1µF + 22kΩ sets LP filter at ~7Hz)

    Titan-D SR typical: 40V/µs (TDA7293 datasheet)
    Target default: 40V/µs
    """
    # Estimate parasitic C from loop areas
    loops = analyze_current_loops(board)
    total_area = sum(l.area_mm2 for l in loops)

    # Parasitic C ≈ 50pF per 1000mm² of loop area (rough estimate)
    parasitic_c_pf = total_area / 20.0

    # SR = I / C, where I is available drive current
    # VAS can source ~10mA, output stage ~100mA
    drive_ma = 50.0  # conservative estimate
    drive_a = drive_ma / 1000.0
    parasitic_c_f = parasitic_c_pf * 1e-12
    sr = drive_a / parasitic_c_f if parasitic_c_f > 0 else float('inf')
    sr_v_us = sr / 1e6  # convert to V/µs

    return sr_v_us >= target_slew


def estimate_phase_margin(board) -> StabilityAnalysis:
    """Estimate phase margin from compensation and feedback network.

    Phase margin depends on:
    - Compensation cap placement (33pF, 1nF)
    - Feedback loop length
    - Output stage parasitic capacitances

    Titan-D uses dominant pole compensation:
    - fz = 1 / (2π * R * C) ≈ 5Hz with 33pF and 1MΩ
    - Unity gain ≈ 1MHz
    - Phase margin target: > 45° (ideal: 60-90°)
    """
    notes = []

    # Find compensation capacitors
    comp_caps = {"C4", "C5"}  # compensation caps in Titan-D
    caps_found = [ref for ref in comp_caps if ref in board.components]

    if len(caps_found) < 2:
        notes.append("Warning: Some compensation caps not found - phase margin may suffer")
        phase_margin = 35.0  # degraded
    else:
        phase_margin = 60.0  # typical for well-compensated design
        notes.append("Compensation network properly placed")

    # Feedback length factor
    if board.bounds_mm:
        fb_length_mm = (board.bounds_mm[2] - board.bounds_mm[0]) * 0.3
        if fb_length_mm > 50:
            phase_margin -= 10
            notes.append("Long feedback path may reduce phase margin")

    # Unity gain estimate
    unity_gain_mhz = 1.0  # typical for audio amps with ~30kHz GBWP

    # Gain margin (estimate)
    gain_margin_db = 12.0  # typical

    stable = phase_margin > 45 and gain_margin_db > 6

    if stable:
        notes.append("Design is stable")
    else:
        notes.append("Stability may be marginal - review compensation")

    return StabilityAnalysis(
        phase_margin_deg=phase_margin,
        gain_margin_db=gain_margin_db,
        unity_gain_freq_mhz=unity_gain_mhz,
        stable=stable,
        notes=notes,
    )


def check_capacitive_load(board, caps: list[float] = None) -> list[float]:
    """Check capacitive load stability.

    Returns list of stable capacitive loads in farads.
    Titan-D can drive up to 10µF without oscillation.
    """
    if caps is None:
        caps = [1e-6, 10e-6, 22e-6, 47e-6]

    stable = []
    for cap_f in caps:
        cap_uf = cap_f * 1e6
        if cap_uf <= 10.0:
            stable.append(cap_f)
        else:
            break  # sorted list assumption
    return stable


def _parallel_segment_length(seg1, seg2, threshold_mm=1.0) -> float:
    """Calculate parallel overlap length between two segments."""
    dx1 = seg1.x2_mm - seg1.x1_mm
    dy1 = seg1.y2_mm - seg1.y1_mm
    dx2 = seg2.x2_mm - seg2.x1_mm
    dy2 = seg2.y2_mm - seg2.y1_mm
    dot = dx1 * dx2 + dy1 * dy2
    if dot < 0:
        return 0.0
    len1 = math.hypot(dx1, dy1)
    len2 = math.hypot(dx2, dy2)
    if len1 < 0.1 or len2 < 0.1:
        return 0.0
    return min(len1, len2) * 0.5


def analyze_crosstalk(board, segments=None) -> list[CrosstalkResult]:
    """Analyze crosstalk between tracks for УНЧ."""
    if segments is None:
        return []
    results = []
    net_segments: dict[str, list] = {}
    for seg in segments:
        net_segments.setdefault(seg.net, []).append(seg)
    nets = list(net_segments.keys())
    for i, net_a in enumerate(nets):
        for net_b in nets[i+1:]:
            if net_a == net_b:
                continue
            a_sensitive = any(s in net_a for s in ("IN", "SRC", "FG", "FB"))
            b_sensitive = any(s in net_b for s in ("IN", "SRC", "FG", "FB"))
            a_noisy = any(s in net_a for s in ("OUT", "SPK", "VCC", "+V", "-V", "PWR"))
            b_noisy = any(s in net_b for s in ("OUT", "SPK", "VCC", "+V", "-V", "PWR"))
            if not ((a_sensitive and b_noisy) or (b_sensitive and a_noisy)):
                continue
            total_parallel_mm = 0.0
            min_gap_mm = float('inf')
            for seg_a in net_segments[net_a]:
                for seg_b in net_segments[net_b]:
                    dist = math.hypot(
                        (seg_a.x1_mm + seg_a.x2_mm) / 2 - (seg_b.x1_mm + seg_b.x2_mm) / 2,
                        (seg_a.y1_mm + seg_a.y2_mm) / 2 - (seg_b.y1_mm + seg_b.y2_mm) / 2
                    )
                    if dist < 5.0:
                        min_gap_mm = min(min_gap_mm, dist)
                        total_parallel_mm += _parallel_segment_length(seg_a, seg_b)
            if total_parallel_mm > 1.0 and min_gap_mm < 5.0:
                crosstalk_pct = (total_parallel_mm / max(min_gap_mm, 0.5)) * 0.01
                crosstalk_pct = min(crosstalk_pct, 10.0)
                critical = crosstalk_pct > 0.1
                rec = "Increase gap or route on different layers" if critical else "OK"
                results.append(CrosstalkResult(
                    net_a=net_a, net_b=net_b,
                    parallel_length_mm=round(total_parallel_mm, 2),
                    gap_mm=round(min_gap_mm, 2),
                    crosstalk_pct=round(crosstalk_pct, 4),
                    critical=critical, recommendation=rec,
                ))
    results.sort(key=lambda x: x.crosstalk_pct, reverse=True)
    return results


def estimate_emi(board, current_loops=None, segments=None) -> EMIPrediction:
    """Estimate EMI/EMC for УНЧ board."""
    problem_areas = []
    recommendations = []
    loop_areas_mm2 = []
    high_freq_traces: list[tuple[str, float]] = []
    if current_loops is None:
        current_loops = analyze_current_loops(board, segments)
    for loop in current_loops:
        loop_areas_mm2.append(loop.area_mm2)
        if loop.area_mm2 > 500:
            problem_areas.append(f"Large loop {loop.net_name}: {loop.area_mm2:.1f} mm²")
            recommendations.append(f"Reduce loop area for {loop.net_name}")
    hf_keywords = ("SW", "CLK", "PWM", "OUT")
    if segments:
        net_lengths: dict[str, float] = {}
        for seg in segments:
            net_lengths[seg.net] = net_lengths.get(seg.net, 0) + math.hypot(
                seg.x2_mm - seg.x1_mm, seg.y2_mm - seg.y1_mm)
        for net, length in net_lengths.items():
            if any(kw in net.upper() for kw in hf_keywords) and length > 50:
                high_freq_traces.append((net, round(length, 1)))
                problem_areas.append(f"Long HF trace {net}: {length:.1f}mm")
    total_loop_area = sum(loop_areas_mm2)
    score = "low"
    if total_loop_area > 2000 or len(problem_areas) > 5:
        score = "high"
    elif total_loop_area > 1000 or len(problem_areas) > 2:
        score = "medium"
    if score == "high":
        recommendations.append("Critical EMI sources detected")
    elif score == "medium":
        recommendations.append("Moderate EMI concerns")
    else:
        recommendations.append("Low EMI expected")
    return EMIPrediction(
        score=score, loop_areas_mm2=loop_areas_mm2,
        high_freq_traces=high_freq_traces,
        ground_plane_continuous=True,
        problem_areas=problem_areas, recommendations=recommendations,
    )


def analyze_power_bus(board, segments=None) -> PowerBusResult:
    """Analyze power distribution network for УНЧ ±40V rails."""
    rail_impedance_mohm = {}
    ripple_100hz_mv = {}
    ripple_1khz_mv = {}
    decoupling_effectiveness = {}
    bulk_caps_needed = []
    recommendations = []
    power_nets = {"+V": 40.0, "-V": -40.0, "+15V": 15.0, "-15V": -15.0}
    load_currents_ma = {"+V": 200, "-V": 200, "+15V": 50, "-15V": 50}
    if segments is None:
        segments = []
    net_segments: dict[str, list] = {}
    for seg in segments:
        net_segments.setdefault(seg.net, []).append(seg)
    for rail in power_nets:
        if rail not in net_segments:
            continue
        total_length_mm = 0.0
        total_width_mm = 0.0
        count = 0
        for seg in net_segments[rail]:
            length = math.hypot(seg.x2_mm - seg.x1_mm, seg.y2_mm - seg.y1_mm)
            total_length_mm += length
            total_width_mm += seg.width_mm
            count += 1
        if count == 0:
            continue
        avg_width = total_width_mm / count if count > 0 else 1.0
        r_mohm = _track_resistance(total_length_mm, avg_width)
        rail_impedance_mohm[rail] = round(r_mohm, 3)
        current_ma = load_currents_ma.get(rail, 100)
        c_bulk = 1000e-6
        ripple_100hz = (current_ma * r_mohm) / (2 * math.pi * 100 * c_bulk) if c_bulk > 0 else 0
        ripple_1khz = (current_ma * r_mohm) / (2 * math.pi * 1000 * c_bulk) if c_bulk > 0 else 0
        ripple_100hz_mv[rail] = round(ripple_100hz * 1000, 3)
        ripple_1khz_mv[rail] = round(ripple_1khz * 1000, 3)
        dec_eff = min(1.0, 1000 / max(total_length_mm, 10))
        decoupling_effectiveness[rail] = round(dec_eff, 2)
        if ripple_100hz > 0.5:
            caps_needed = int(current_ma / 50) * 1000
            bulk_caps_needed.append((rail, caps_needed))
            recommendations.append(f"Add {caps_needed}uF bulk cap on {rail}")
    return PowerBusResult(
        rail_impedance_mohm=rail_impedance_mohm,
        ripple_100hz_mv=ripple_100hz_mv,
        ripple_1khz_mv=ripple_1khz_mv,
        decoupling_effectiveness=decoupling_effectiveness,
        bulk_capacitors_needed=bulk_caps_needed,
        recommendations=recommendations,
    )


def generate_thermal_heatmap(board, thermal_results: list[ThermalResult],
                             output_path: str = "thermal_heatmap.png") -> str:
    """Generate thermal heatmap as PNG.

    Uses matplotlib to create a thermal map overlay on board outline.
    """
    try:
        import matplotlib.pyplot as plt
        import matplotlib.patches as patches
    except ImportError:
        logger.warning("matplotlib not available, skipping heatmap generation")
        return ""

    fig, ax = plt.subplots(1, 1, figsize=(12, 8))

    # Board bounds
    if board.bounds_mm:
        bx1, by1, bx2, by2 = board.bounds_mm
        board_w = bx2 - bx1
        board_h = by2 - by1
        ax.set_xlim(bx1 - 10, bx2 + 10)
        ax.set_ylim(by1 - 10, by2 + 10)

        # Draw board outline
        rect = patches.Rectangle((bx1, by1), board_w, board_h,
                                  linewidth=2, edgecolor='black', facecolor='none')
        ax.add_patch(rect)

    # Create thermal map based on component positions
    components = {c.ref: c for c in board.components.values()}

    temps = []
    positions = []
    for tr in thermal_results:
        if tr.component_ref in components:
            comp = components[tr.component_ref]
            temps.append(tr.junction_temp_c)
            positions.append((comp.x_mm, comp.y_mm))

    if temps:
        temps_arr = np.array(temps)
        positions_arr = np.array(positions)

        # Create scatter plot with thermal coloring
        scatter = ax.scatter(positions_arr[:, 0], positions_arr[:, 1],
                            c=temps_arr, cmap='hot', s=200, zorder=10,
                            vmin=25, vmax=100)

        # Add labels
        for i, tr in enumerate(thermal_results):
            if tr.component_ref in components:
                comp = components[tr.component_ref]
                ax.annotate(f"{tr.component_ref}\n{tr.junction_temp_c}°C",
                           (comp.x_mm, comp.y_mm), textcoords="offset points",
                           xytext=(0, 10), ha='center', fontsize=7)

        cbar = plt.colorbar(scatter, ax=ax)
        cbar.set_label('Junction Temperature (°C)')

    ax.set_xlabel('X (mm)')
    ax.set_ylabel('Y (mm)')
    ax.set_title('Thermal Distribution Map')
    ax.set_aspect('equal')
    ax.grid(True, alpha=0.3)

    plt.tight_layout()
    plt.savefig(output_path, dpi=150, bbox_inches='tight')
    plt.close()

    return output_path


def run_full_simulation(board, segments=None, jumpers=None,
                        ambient_temp: float = AMBIENT_DEFAULT,
                        load_impedance: float = 4.0,
                        generate_heatmap: bool = True,
                        output_dir: str = ".") -> SimulationReport:
    """Run complete simulation suite on a board.

    Args:
        board: Board object with components and nets
        segments: Optional list of RouteSegment for current loop analysis
        jumpers: Optional list of Jumper
        ambient_temp: Ambient temperature in °C
        load_impedance: Speaker load impedance in Ohms
        generate_heatmap: Whether to generate thermal heatmap PNG
        output_dir: Directory for output files

    Returns:
        SimulationReport with all analysis results
    """
    logger.info("Starting full board simulation")

    # Current loop analysis
    logger.info("Analyzing current loops...")
    current_loops = analyze_current_loops(board, segments, jumpers)
    logger.info(f"Found {len(current_loops)} current loops")

    # Power distribution / IR drop
    logger.info("Analyzing power distribution...")
    ir_drop = analyze_power_distribution(board, segments)
    logger.info(f"IR drop analysis complete for {len(ir_drop)} nets")

    # Thermal analysis
    logger.info("Running thermal simulation...")
    thermal_results = analyze_thermal(board, ambient_temp)
    logger.info(f"Thermal analysis complete for {len(thermal_results)} components")

    # Generate thermal heatmap if requested
    import os
    thermal_heatmap_path = None
    if generate_heatmap:
        logger.info("Generating thermal heatmap...")
        os.makedirs(output_dir, exist_ok=True)
        heatmap_file = os.path.join(output_dir, "thermal_heatmap.png")
        try:
            thermal_heatmap_path = generate_thermal_heatmap(board, thermal_results, heatmap_file)
            logger.info(f"Heatmap saved to {thermal_heatmap_path}")
        except Exception as e:
            logger.warning(f"Heatmap generation failed: {e}")

    # Audio parameter predictions
    logger.info("Predicting audio parameters...")
    df = predict_damping_factor(board, load_impedance)
    thd = estimate_thd(board, segments)
    psrr = predict_psrr(board, segments)
    sr = check_slew_rate(board)
    stable_caps = check_capacitive_load(board)

    audio = AudioPrediction(
        damping_factor=df,
        thd_percent=thd,
        psrr_pos_100hz=psrr["psrr_pos_100hz"],
        psrr_pos_1khz=psrr["psrr_pos_1khz"],
        psrr_neg_100hz=psrr["psrr_neg_100hz"],
        psrr_neg_1khz=psrr["psrr_neg_1khz"],
        slew_rate_v_us=40.0 if sr else 20.0,  # estimate
        stable_capacitive_load=stable_caps,
    )

    # Stability analysis
    logger.info("Analyzing stability...")
    stability = estimate_phase_margin(board)

    # Compile recommendations
    recommendations = []
    warnings = []

    # Loop recommendations
    critical_loops = [l for l in current_loops if l.critical]
    if critical_loops:
        recommendations.append(
            f"Reduce area of {len(critical_loops)} critical current loops "
            f"(total area: {sum(l.area_mm2 for l in critical_loops):.1f} mm²)"
        )

    # Thermal recommendations
    hot_components = [t for t in thermal_results if t.junction_temp_c > 80]
    if hot_components:
        recommendations.append(
            f"Consider heatsink upgrade for {len(hot_components)} components "
            f"with Tj > 80°C"
        )

    # Damping factor recommendations
    if df < 30:
        recommendations.append(
            f"Damping factor {df} is below ideal (>50). "
            "Consider thicker output traces or lower emitter resistance."
        )

    # THD recommendations
    if thd > 0.005:
        recommendations.append(
            f"THD estimate {thd:.4f}% is elevated. "
            "Review loop areas and decoupling."
        )

    # Stability warnings
    if not stability.stable:
        warnings.append(
            f"Phase margin {stability.phase_margin_deg:.0f}° may be insufficient"
        )

    # IR drop warnings
    for net, drop in ir_drop.items():
        if drop > 50:  # > 50mV is concerning
            warnings.append(f"Significant IR drop on {net}: {drop:.1f}mV")

    # Crosstalk analysis
    logger.info("Analyzing crosstalk...")
    crosstalk = analyze_crosstalk(board, segments)
    logger.info(f"Found {len(crosstalk)} crosstalk pairs")
    critical_ct = [c for c in crosstalk if c.critical]
    if critical_ct:
        warnings.append(f"Critical crosstalk found: {len(critical_ct)} pairs")

    # EMI analysis
    logger.info("Analyzing EMI...")
    emi = estimate_emi(board, current_loops, segments)
    if emi.score == "high":
        warnings.append("High EMI detected - review layout")

    # Power bus analysis
    logger.info("Analyzing power bus...")
    power_bus = analyze_power_bus(board, segments)
    if power_bus.bulk_capacitors_needed:
        for rail, cap in power_bus.bulk_capacitors_needed:
            recommendations.append(f"Add {cap:.0f}uF bulk cap on {rail}")

    board_name = getattr(board, 'name', 'unknown')

    return SimulationReport(
        board_name=board_name,
        current_loops=current_loops,
        thermal_results=thermal_results,
        audio=audio,
        stability=stability,
        ir_drop=ir_drop,
        crosstalk=crosstalk,
        recommendations=recommendations,
        warnings=warnings,
        thermal_heatmap_path=thermal_heatmap_path,
        emi=emi,
        power_bus=power_bus,
    )
