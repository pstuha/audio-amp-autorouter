"""Audio-amplifier specific rules and LUT (toner transfer) manufacturing profile."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Dict, Tuple


@dataclass(frozen=True)
class NetClass:
    name: str
    priority: int
    track_mm: float
    length_cost: float
    max_length_mm: float | None = None
    keep_away_from: Tuple[str, ...] = ()


NET_CLASSES: Dict[str, NetClass] = {
    "power": NetClass("power", 1, 1.2, 6.0),
    "high_current": NetClass("high_current", 2, 1.5, 5.0),
    "ground": NetClass("ground", 3, 1.2, 4.0),
    "signal": NetClass("signal", 4, 0.4, 1.0, keep_away_from=("power", "high_current")),
    "feedback": NetClass("feedback", 5, 0.4, 4.0, keep_away_from=("power", "high_current")),
    "compensation": NetClass("compensation", 6, 0.4, 2.0),
    "control": NetClass("control", 7, 0.3, 1.0),
}

POWER_HINTS = ("vcc", "vdd", "vee", "vss", "+v", "-v", "+12", "-12", "+15", "-15", "+24", "+5", "vs+", "vs-")
GROUND_HINTS = ("gnd", "agnd", "pgnd", "dgnd", "earth")
OUTPUT_HINTS = ("out", "spk", "speaker", "load")
FEEDBACK_HINTS = ("fb", "feedback", "ninv", "inv")
COMP_HINTS = ("miller", "comp", "zobel", "bootstrap", "bias")


def classify_net(name: str) -> str:
    n = name.lower()
    if any(h in n for h in GROUND_HINTS):
        return "ground"
    if any(h in n for h in OUTPUT_HINTS):
        return "high_current"
    if any(h in n for h in POWER_HINTS):
        return "power"
    if any(h in n for h in FEEDBACK_HINTS):
        return "feedback"
    if any(h in n for h in COMP_HINTS):
        return "compensation"
    return "signal"


@dataclass(frozen=True)
class LutProfile:
    """Manufacturing profile for a single-sided toner-transfer board."""

    min_clearance_mm: float = 0.3
    min_track_mm: float = 0.4
    min_pad_annular_mm: float = 0.25
    min_jumper_length_mm: float = 2.0
    max_jumper_length_mm: float = 30.0
    min_copper_island_mm2: float = 0.5
    min_neck_width_mm: float = 0.35
    edge_clearance_mm: float = 1.0
    allowed_angles: Tuple[int, ...] = (0, 45, 90, 135, 180, 225, 270, 315)


@dataclass(frozen=True)
class AmpRules:
    """Placement and routing heuristics specific to audio amplifiers."""

    input_output_min_distance_mm: float = 40.0
    decoupling_max_distance_mm: float = 8.0
    feedback_max_length_mm: float = 25.0
    power_loop_length_cost: float = 6.0
    signal_detour_cost: float = 4.0
    thermal_keepout_mm: float = 3.0
    fixed_ref_prefixes: Tuple[str, ...] = ("H", "MH", "MP", "FID", "LOGO", "TP")
    fixed_connector_prefixes: Tuple[str, ...] = ("J", "P", "X")
    net_classes: Dict[str, NetClass] = field(default_factory=lambda: dict(NET_CLASSES))
