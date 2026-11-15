"""Jumper (wire-link) planner for single-sided LUT boards."""

from __future__ import annotations

import math
from dataclasses import dataclass, field
from typing import List

from .models import Board, Jumper, RouteSegment
from .rules import LutProfile


@dataclass
class JumperPlan:
    jumpers: List[Jumper] = field(default_factory=list)
    notes: List[str] = field(default_factory=list)

    @property
    def count(self) -> int:
        return len(self.jumpers)

    @property
    def total_length_mm(self) -> float:
        return sum(j.length_mm for j in self.jumpers)


def _segments_cross(a: RouteSegment, b: RouteSegment) -> bool:
    def orient(p, q, r):
        return (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])

    p1, p2 = (a.x1_mm, a.y1_mm), (a.x2_mm, a.y2_mm)
    q1, q2 = (b.x1_mm, b.y1_mm), (b.x2_mm, b.y2_mm)
    o1 = orient(p1, p2, q1)
    o2 = orient(p1, p2, q2)
    o3 = orient(q1, q2, p1)
    o4 = orient(q1, q2, p2)
    return o1 * o2 < 0 and o3 * o4 < 0


def _pad_positions(board: Board):
    import math as _math

    result = {}
    for comp in board.components.values():
        angle = _math.radians(comp.rotation_deg)
        ca, sa = _math.cos(angle), _math.sin(angle)
        for pad in comp.pads:
            if not pad.net or pad.net in ("", "0"):
                continue
            x = comp.x_mm + pad.x_mm * ca - pad.y_mm * sa
            y = comp.y_mm + pad.x_mm * sa + pad.y_mm * ca
            result.setdefault(pad.net, []).append((comp.ref, pad.number, x, y))
    return result


def plan_wire_jumpers(board: Board, failed_nets, max_per_net: int = 4) -> List[Jumper]:
    """Convert unroutable nets to wire jumpers (legitimate on single-layer LUT boards)."""
    pads = _pad_positions(board)
    jumpers: List[Jumper] = []
    for net in failed_nets:
        pins = pads.get(net, [])
        if len(pins) < 2:
            continue
        ordered = sorted(pins, key=lambda p: (p[0], p[1]))
        for i in range(len(ordered) - 1):
            _, _, x1, y1 = ordered[i]
            _, _, x2, y2 = ordered[i + 1]
            length = math.hypot(x2 - x1, y2 - y1)
            jumpers.append(Jumper(net, x1, y1, x2, y2, length))
            if len([j for j in jumpers if j.net == net]) >= max_per_net:
                break
    return jumpers


def plan_jumpers(
    board: Board,
    segments: List[RouteSegment],
    lut: LutProfile | None = None,
    failed_nets: List[str] | None = None,
) -> JumperPlan:
    """Promote crossing segments to wire jumpers so the board stays single-layer.

    Nets that A* could not route are converted to sequential wire links, which is
    the standard workaround on single-layer LUT (toner transfer) boards.
    """
    lut = lut or LutProfile()
    plan = JumperPlan()

    for i in range(len(segments)):
        for j in range(i + 1, len(segments)):
            a, b = segments[i], segments[j]
            if a.net == b.net:
                continue
            if not _segments_cross(a, b):
                continue
            length = math.hypot(a.x2_mm - a.x1_mm, a.y2_mm - a.y1_mm)
            if length < lut.min_jumper_length_mm or length > lut.max_jumper_length_mm:
                continue
            plan.jumpers.append(Jumper(a.net, a.x1_mm, a.y1_mm, a.x2_mm, a.y2_mm, length))

    if failed_nets:
        wire = plan_wire_jumpers(board, failed_nets)
        plan.jumpers.extend(wire)
        plan.notes.append(f"wire jumpers for unrouted nets: {[j.net for j in wire]}")

    plan.notes.append(f"jumpers={plan.count} total_length={plan.total_length_mm:.1f}mm")
    return plan
