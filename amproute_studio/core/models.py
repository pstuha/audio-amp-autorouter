"""Normalized board model shared by every layer of the application."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Dict, List, Optional, Tuple


@dataclass
class Pad:
    number: str
    net: str
    x_mm: float
    y_mm: float
    width_mm: float = 1.6
    height_mm: float = 1.6
    drill_mm: float = 0.8
    shape: str = "circle"
    layers: List[str] = field(default_factory=list)


@dataclass
class Component:
    ref: str
    value: str
    footprint: str
    x_mm: float
    y_mm: float
    rotation_deg: float = 0.0
    layer: str = "F.Cu"
    pads: List[Pad] = field(default_factory=list)
    width_mm: float = 5.0
    height_mm: float = 5.0
    courtyard_width_mm: float = 6.0
    courtyard_height_mm: float = 6.0
    locked: bool = False
    fixed: bool = False
    attributes: Dict[str, str] = field(default_factory=dict)

    @property
    def ref_type(self) -> str:
        prefix = ""
        for ch in self.ref:
            if ch.isalpha():
                prefix += ch
            else:
                break
        return prefix.upper()

    @property
    def movable(self) -> bool:
        return not (self.locked or self.fixed)


@dataclass
class Net:
    name: str
    code: int = 0
    pins: List[Tuple[str, str]] = field(default_factory=list)
    net_class: str = "signal"


@dataclass
class RouteSegment:
    net: str
    x1_mm: float
    y1_mm: float
    x2_mm: float
    y2_mm: float
    width_mm: float
    layer: str = "F.Cu"


@dataclass
class Jumper:
    """A wire link on the top side that bridges two copper points on the bottom layer."""

    net: str
    x1_mm: float
    y1_mm: float
    x2_mm: float
    y2_mm: float
    length_mm: float = 0.0


@dataclass
class Board:
    components: Dict[str, Component] = field(default_factory=dict)
    nets: Dict[str, Net] = field(default_factory=dict)
    bounds_mm: Optional[Tuple[float, float, float, float]] = None
    mounting_holes: List[Tuple[float, float, float]] = field(default_factory=list)
    notes: List[str] = field(default_factory=list)
    segments: List["RouteSegment"] = field(default_factory=list)  # Routing segments
    vias: List["Via"] = field(default_factory=list)  # Via definitions

    def net_names_for(self, ref: str) -> List[str]:
        comp = self.components.get(ref)
        if not comp:
            return []
        return sorted({p.net for p in comp.pads if p.net and p.net not in ("", "0")})


@dataclass
class DesignRules:
    copper_layer: str = "B.Cu"
    min_clearance_mm: float = 0.3
    min_track_mm: float = 0.4
    power_track_mm: float = 1.2
    high_current_track_mm: float = 1.5
    edge_clearance_mm: float = 1.0
    grid_mm: float = 0.25
    component_gap_mm: float = 1.2
    courtyard_gap_mm: float = 0.6
    max_jumpers: int = 80
    allowed_rotations: Tuple[int, ...] = (0, 90, 180, 270)
