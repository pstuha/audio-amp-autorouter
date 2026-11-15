"""Validation rules: geometry, electrical, LUT manufacturability."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import List, Optional

from .geometry import bbox_contains, bbox_overlap, component_bbox
from .models import Board, DesignRules, RouteSegment
from .rules import LutProfile


@dataclass
class Violation:
    code: str
    severity: str  # "error" | "warning"
    message: str
    refs: List[str] = field(default_factory=list)
    x_mm: Optional[float] = None
    y_mm: Optional[float] = None


@dataclass
class ValidationReport:
    violations: List[Violation] = field(default_factory=list)

    @property
    def errors(self) -> List[Violation]:
        return [v for v in self.violations if v.severity == "error"]

    @property
    def warnings(self) -> List[Violation]:
        return [v for v in self.violations if v.severity == "warning"]

    @property
    def ok(self) -> bool:
        return not self.errors

    def add(self, violation: Violation) -> None:
        self.violations.append(violation)


def validate_geometry(board: Board, rules: DesignRules) -> ValidationReport:
    report = ValidationReport()
    if board.bounds_mm is None:
        report.add(Violation("board_bounds_missing", "error", "Контур платы не задан"))
        return report

    bounds = board.bounds_mm
    comps = list(board.components.values())

    for comp in comps:
        bbox = component_bbox(
            comp.x_mm, comp.y_mm, comp.rotation_deg,
            comp.courtyard_width_mm, comp.courtyard_height_mm,
        )
        if not bbox_contains(bounds, bbox, rules.edge_clearance_mm):
            report.add(Violation(
                "component_outside_board", "error",
                f"{comp.ref} выходит за контур платы или нарушает edge clearance",
                refs=[comp.ref], x_mm=comp.x_mm, y_mm=comp.y_mm,
            ))

    for i in range(len(comps)):
        for j in range(i + 1, len(comps)):
            a, b = comps[i], comps[j]
            ba = component_bbox(a.x_mm, a.y_mm, a.rotation_deg, a.courtyard_width_mm, a.courtyard_height_mm)
            bb = component_bbox(b.x_mm, b.y_mm, b.rotation_deg, b.courtyard_width_mm, b.courtyard_height_mm)
            if bbox_overlap(ba, bb, rules.courtyard_gap_mm):
                report.add(Violation(
                    "courtyard_overlap", "error",
                    f"Пересечение courtyard: {a.ref} <-> {b.ref}",
                    refs=[a.ref, b.ref],
                ))

    for hx, hy, hr in board.mounting_holes:
        for comp in comps:
            bbox = component_bbox(
                comp.x_mm, comp.y_mm, comp.rotation_deg,
                comp.courtyard_width_mm, comp.courtyard_height_mm,
            )
            if bbox[0] - hr <= hx <= bbox[2] + hr and bbox[1] - hr <= hy <= bbox[3] + hr:
                report.add(Violation(
                    "mounting_hole_collision", "error",
                    f"Монтажное отверстие ({hx:.1f}, {hy:.1f}) пересекает {comp.ref}",
                    refs=[comp.ref], x_mm=hx, y_mm=hy,
                ))

    return report


def validate_routing(
    board: Board,
    segments: List[RouteSegment],
    rules: DesignRules,
    lut: LutProfile,
    jumpered_nets: Optional[set] = None,
) -> ValidationReport:
    report = ValidationReport()
    routed_nets = {s.net for s in segments}
    jumpered = jumpered_nets or set()
    for net in board.nets.values():
        if len(net.pins) >= 2 and net.name not in routed_nets and net.name not in jumpered:
            report.add(Violation(
                "net_not_routed", "error",
                f"Сеть {net.name} не проложена",
                refs=[net.name],
            ))
        elif net.name in jumpered and net.name not in routed_nets:
            report.add(Violation(
                "net_jumpered", "warning",
                f"Сеть {net.name} проложена навесной перемычкой",
                refs=[net.name],
            ))

    for seg in segments:
        if seg.layer != rules.copper_layer:
            report.add(Violation(
                "wrong_layer", "error",
                f"Дорожка {seg.net} на слое {seg.layer}, ожидается {rules.copper_layer}",
                refs=[seg.net],
            ))
        if seg.width_mm < lut.min_track_mm - 1e-6:
            report.add(Violation(
                "track_too_thin", "error",
                f"Дорожка {seg.net} уже {lut.min_track_mm} мм ({seg.width_mm:.2f})",
                refs=[seg.net],
            ))

    for i in range(len(segments)):
        for j in range(i + 1, len(segments)):
            a, b = segments[i], segments[j]
            if a.net == b.net:
                continue
            if _segments_too_close(a, b, lut.min_clearance_mm):
                report.add(Violation(
                    "clearance", "error",
                    f"Слишком близко: {a.net} и {b.net}",
                    refs=[a.net, b.net],
                ))

    return report


def _segments_too_close(a: RouteSegment, b: RouteSegment, clearance: float) -> bool:
    from .geometry import seg_distance

    edge_gap = seg_distance(
        (a.x1_mm, a.y1_mm), (a.x2_mm, a.y2_mm),
        (b.x1_mm, b.y1_mm), (b.x2_mm, b.y2_mm),
    ) - (a.width_mm + b.width_mm) / 2
    return edge_gap < clearance - 1e-6
