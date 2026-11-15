"""Placement engine for single-layer audio amplifier boards."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Dict, List, Tuple

from .geometry import bbox_overlap, component_bbox
from .models import Board, Component, DesignRules
from .rules import AmpRules


@dataclass
class PlacementSolution:
    moves: Dict[str, Tuple[float, float, float]] = field(default_factory=dict)
    score: float = 0.0
    log: List[str] = field(default_factory=list)


class PlacementEngine:
    def __init__(self, rules: DesignRules, amp: AmpRules | None = None):
        self.rules = rules
        self.amp = amp or AmpRules()
        self.log: List[str] = []

    def _log(self, msg: str) -> None:
        self.log.append(msg)

    def is_fixed(self, comp: Component) -> bool:
        if comp.locked or comp.fixed:
            return True
        r = comp.ref.upper()
        if r.startswith(self.amp.fixed_ref_prefixes):
            return True
        if r.startswith(self.amp.fixed_connector_prefixes):
            return True
        fp = f"{comp.footprint} {comp.value}".lower()
        return "mounting" in fp or "fiducial" in fp

    def classify(self, board: Board) -> Dict[str, List[str]]:
        groups: Dict[str, List[str]] = {
            "fixed": [], "input": [], "output": [], "power": [],
            "amp": [], "feedback": [], "decoupling": [], "passive": [], "other": [],
        }
        for ref, comp in board.components.items():
            if self.is_fixed(comp):
                groups["fixed"].append(ref)
                continue
            nets = [n.lower() for n in board.net_names_for(ref)]
            blob = " ".join(nets)
            rtype = comp.ref_type
            if rtype == "U":
                groups["amp"].append(ref)
            elif rtype == "C" and any(k in blob for k in ("vcc", "vdd", "vee", "vss", "gnd")):
                groups["decoupling"].append(ref)
            elif rtype == "R" and any(k in blob for k in ("fb", "feedback", "inv")):
                groups["feedback"].append(ref)
            elif rtype in ("R", "C", "L"):
                groups["passive"].append(ref)
            else:
                groups["other"].append(ref)
        return groups

    def _board_area(self, board: Board) -> Tuple[float, float, float, float]:
        # Determine usable board area. If the PCB file provides explicit bounds, use them with a safety margin.
        if board.bounds_mm:
            minx, miny, maxx, maxy = board.bounds_mm
            m = self.rules.edge_clearance_mm + 2.0
            return minx + m, miny + m, maxx - m, maxy - m
        # Fallback: derive bounds from component courtyards (or component extents) to avoid placing components outside.
        if board.components:
            xs = []
            ys = []
            for comp in board.components.values():
                # Use courtyard extents if available, else component size.
                half_w = (comp.courtyard_width_mm or comp.width_mm) / 2.0
                half_h = (comp.courtyard_height_mm or comp.height_mm) / 2.0
                xs.extend([comp.x_mm - half_w, comp.x_mm + half_w])
                ys.extend([comp.y_mm - half_h, comp.y_mm + half_h])
            minx, maxx = min(xs), max(xs)
            miny, maxy = min(ys), max(ys)
            # Add a generous margin so placement has space.
            margin = max(self.rules.edge_clearance_mm, self.rules.grid_mm) * 3
            return minx + margin, miny + margin, maxx - margin, maxy - margin
        # Ultimate fallback to a generic area.
        return 0.0, 0.0, 100.0, 80.0

    def _place_on_grid(self, board: Board, groups: Dict[str, List[str]]) -> None:
        minx, miny, maxx, maxy = self._board_area(board)
        order = ["input", "amp", "feedback", "decoupling", "power", "passive", "other", "output"]
        movable = [r for name in order for r in groups.get(name, [])]
        grid = self.rules.grid_mm
        x, y, row_h = minx, miny, 0.0
        for ref in movable:
            comp = board.components[ref]
            w = comp.courtyard_width_mm + self.rules.component_gap_mm
            h = comp.courtyard_height_mm + self.rules.component_gap_mm
            if x + w > maxx:
                x = minx
                y += row_h
                row_h = 0.0
            comp.x_mm = round((x + w / 2) / grid) * grid
            comp.y_mm = round((y + h / 2) / grid) * grid
            x += w
            row_h = max(row_h, h)
        self._log(f"Размещено компонентов: {len(movable)}")

    def _resolve_overlaps(self, board: Board, iterations: int = 120) -> int:
        comps = [c for c in board.components.values() if not self.is_fixed(c)]
        fixed = [c for c in board.components.values() if self.is_fixed(c)]
        for _ in range(iterations):
            moved = False
            for i in range(len(comps)):
                for j in range(i + 1, len(comps)):
                    a, b = comps[i], comps[j]
                    ba = component_bbox(a.x_mm, a.y_mm, a.rotation_deg, a.courtyard_width_mm, a.courtyard_height_mm)
                    bb = component_bbox(b.x_mm, b.y_mm, b.rotation_deg, b.courtyard_width_mm, b.courtyard_height_mm)
                    if bbox_overlap(ba, bb, self.rules.courtyard_gap_mm):
                        b.x_mm += self.rules.courtyard_gap_mm + 0.5
                        moved = True
                for f in fixed:
                    ba = component_bbox(a.x_mm, a.y_mm, a.rotation_deg, a.courtyard_width_mm, a.courtyard_height_mm)
                    bf = component_bbox(f.x_mm, f.y_mm, f.rotation_deg, f.courtyard_width_mm, f.courtyard_height_mm)
                    if bbox_overlap(ba, bf, self.rules.courtyard_gap_mm):
                        a.x_mm += self.rules.courtyard_gap_mm + 0.5
                        moved = True
            if not moved:
                break
        return 0

    def _rotate(self, board: Board, groups: Dict[str, List[str]]) -> None:
        for ref in groups.get("passive", []):
            comp = board.components[ref]
            if comp.width_mm > comp.height_mm:
                comp.rotation_deg = 0.0
            else:
                comp.rotation_deg = 90.0

    def run(self, board: Board) -> PlacementSolution:
        self.log = []
        groups = self.classify(board)
        self._log(f"Классификация компонентов: {groups}")
        
        # 1. Инициализация: сохраняем исходные позиции для подвижных компонентов
        moves = {}
        for ref, comp in board.components.items():
            if not self.is_fixed(comp):
                moves[ref] = (comp.x_mm, comp.y_mm, comp.rotation_deg)
                
        solution = PlacementSolution(moves=moves, log=list(self.log))

        # 2. Устранение конфликтов (перемещения)
        initial_overlaps = self._resolve_overlaps(board)
        solution.log.append(f"Начальные конфликты: {initial_overlaps}")

        # 3. Принудительное вращение (если применимо)
        self._rotate(board, groups)
        
        self._log(f"Проверка конфликтов после коррекции: {self._resolve_overlaps(board)}")
        
        return solution
