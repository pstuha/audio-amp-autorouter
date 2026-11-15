"""Single-layer A* router with rip-up/reroute and iterative improvement.

The router routes nets in priority order (power/high-current first, signal last).
When a net cannot be routed, *rip-up* removes low-priority nets whose tracks block
the corridor, *reroutes* the victim net, then retries the removed nets.
A multi-pass *iterative improvement* loop keeps the best solution by a cost
function that strongly penalises jumpers and rewards short class-aware routes.
"""

from __future__ import annotations

import heapq
import math
import random
from dataclasses import dataclass, field
from typing import Dict, List, Optional, Set, Tuple

from .models import Board, DesignRules, Jumper, RouteSegment
from .rules import LutProfile, classify_net

Point = Tuple[int, int]

_IMPROVE_PASSES = 12
_RIP_LIMIT = 3
_RIP_DEPTH = 3
_POWER_ZONE = 3  # extra cells kept around power/high-current tracks

_COST = {"power": 3.0, "high_current": 2.5, "ground": 3.0, "feedback": 2.0,
         "compensation": 1.5, "signal": 1.0, "control": 1.0}


@dataclass
class RoutingSolution:
    segments: List[RouteSegment] = field(default_factory=list)
    jumpers: List[Jumper] = field(default_factory=list)
    failed_nets: List[str] = field(default_factory=list)
    routed_nets: List[str] = field(default_factory=list)
    total_length_mm: float = 0.0
    score: float = 0.0
    iterations: int = 0
    rip_ups: int = 0
    notes: List[str] = field(default_factory=list)

    @property
    def ok(self) -> bool:
        return not self.failed_nets

    @property
    def jumper_count(self) -> int:
        return len(self.jumpers)


def _dist(a: Point, b: Point) -> int:
    return abs(a[0] - b[0]) + abs(a[1] - b[1])


def _compress(path: List[Point]) -> List[Tuple[Point, Point]]:
    if len(path) < 2:
        return []
    out = []
    start = prev = path[0]
    direction = (path[1][0] - path[0][0], path[1][1] - path[0][1])
    for point in path[1:]:
        nd = (point[0] - prev[0], point[1] - prev[1])
        if nd != direction:
            out.append((start, prev))
            start = prev
            direction = nd
        prev = point
    out.append((start, prev))
    return out


def _astar(
    start: Point,
    goals: Set[Point],
    blocked: Set[Point],
    bounds: Tuple[int, int, int, int],
    max_nodes: int = 300_000,
    cost_weight: float = 1.0,
    avoid: Optional[Set[Point]] = None,
) -> Optional[List[Point]]:
    """Weighted A*. `cost_weight` scales the step cost so high-priority nets
    prefer short paths; `avoid` adds an extra penalty (keep signal out of the
    power keep-out zones) without hard-blocking them."""
    minx, miny, maxx, maxy = bounds
    openq = [(0.0, 0, start)]
    came: Dict[Point, Point] = {}
    cost = {start: 0.0}
    seen = 0
    avoid = avoid or set()
    while openq and seen < max_nodes:
        _, _, cur = heapq.heappop(openq)
        seen += 1
        if cur in goals:
            path = [cur]
            while path[-1] in came:
                path.append(came[path[-1]])
            path.reverse()
            return path
        x, y = cur
        for nxt in ((x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)):
            nx, ny = nxt
            if nx < minx or ny < miny or nx > maxx or ny > maxy or nxt in blocked:
                continue
            step = cost_weight * (4.0 if nxt in avoid else 1.0)
            new_cost = cost[cur] + step
            if new_cost >= cost.get(nxt, 1e12):
                continue
            cost[nxt] = new_cost
            came[nxt] = cur
            h = min(_dist(nxt, goal) for goal in goals)
            heapq.heappush(openq, (new_cost + h * cost_weight, new_cost, nxt))
    return None


def _pad_points(board: Board) -> Dict[str, List[Tuple[str, str, float, float]]]:
    result: Dict[str, List[Tuple[str, str, float, float]]] = {}
    for comp in board.components.values():
        angle = math.radians(comp.rotation_deg)
        ca, sa = math.cos(angle), math.sin(angle)
        for pad in comp.pads:
            if not pad.net or pad.net in ("", "0"):
                continue
            x = comp.x_mm + pad.x_mm * ca - pad.y_mm * sa
            y = comp.y_mm + pad.x_mm * sa + pad.y_mm * ca
            result.setdefault(pad.net, []).append((comp.ref, pad.number, x, y))
    return result


def _blocked_cells(board: Board, rules: DesignRules, grid: float) -> Set[Point]:
    """Cells forbidden for ALL routing: clearance around every pad."""
    blocked: Set[Point] = set()
    radius = max(1, int(math.ceil(rules.min_clearance_mm / grid)))
    for comp in board.components.values():
        angle = math.radians(comp.rotation_deg)
        ca, sa = math.cos(angle), math.sin(angle)
        for pad in comp.pads:
            if not pad.net or pad.net in ("", "0"):
                continue
            px = comp.x_mm + pad.x_mm * ca - pad.y_mm * sa
            py = comp.y_mm + pad.x_mm * sa + pad.y_mm * ca
            px_cell = int(round(px / grid))
            py_cell = int(round(py / grid))
            for dx in range(-radius, radius + 1):
                for dy in range(-radius, radius + 1):
                    if dx * dx + dy * dy <= radius * radius:
                        blocked.add((px_cell + dx, py_cell + dy))
    return blocked


def _inflate(cells, radius: int) -> Set[Point]:
    out: Set[Point] = set()
    r2 = radius * radius
    for cx, cy in cells:
        for dx in range(-radius, radius + 1):
            if dx * dx > r2:
                continue
            dy_max = int(math.sqrt(r2 - dx * dx))
            for dy in range(-dy_max, dy_max + 1):
                out.add((cx + dx, cy + dy))
    return out


def _net_key(net: str) -> Tuple[int, float, str]:
    cls = classify_net(net)
    order = {"power": 0, "high_current": 1, "ground": 2, "feedback": 3,
             "compensation": 4, "signal": 5, "control": 6}
    return (order.get(cls, 9), -_COST.get(cls, 1.0), net)


def _walk(a: Point, b: Point) -> List[Point]:
    """All grid cells along an orthogonal run from a to b (inclusive)."""
    out: List[Point] = []
    ax, ay = a
    bx, by = b
    if ax == bx:
        step = 1 if by >= ay else -1
        for y in range(ay, by + step, step):
            out.append((ax, y))
    elif ay == by:
        step = 1 if bx >= ax else -1
        for x in range(ax, bx + step, step):
            out.append((x, ay))
    else:
        out.append(a)
        out.append(b)
    return out


def _net_order(pads: Dict[str, List[Tuple[str, str, float, float]]]) -> List[str]:
    return sorted((n for n, p in pads.items() if len(p) >= 2), key=_net_key)


class _State:
    """Mutable routing state for one pass: grid, occupancy and per-net paths."""

    def __init__(self, board: Board, rules: DesignRules, lut: LutProfile):
        self.board = board
        self.rules = rules
        self.grid = max(0.25, rules.grid_mm)
        bounds_mm = board.bounds_mm or (0.0, 0.0, 160.0, 90.0)
        minx, miny, maxx, maxy = [int(round(v / self.grid)) for v in bounds_mm]
        self.bounds = (minx + 1, miny + 1, maxx - 1, maxy - 1)
        self.pad_block = _blocked_cells(board, rules, self.grid)
        self.net_pads = _pad_points(board)
        self.terminal_cells: Dict[str, Set[Point]] = {
            net: {(int(round(x / self.grid)), int(round(y / self.grid))) for _, _, x, y in pins}
            for net, pins in self.net_pads.items()
        }
        self.paths: Dict[str, List[Tuple[Point, Point]]] = {}
        self.occ: Dict[str, Set[Point]] = {}
        self.busy: Set[Point] = set()
        self.rip_ups = 0

    def rul_width(self, net: str) -> float:
        cls = classify_net(net)
        if cls == "high_current":
            return max(self.rules.high_current_track_mm, self.rules.min_track_mm)
        if cls in ("power", "ground"):
            return max(self.rules.power_track_mm, self.rules.min_track_mm)
        return max(self.rules.min_track_mm, 0.25)

    def claim(self, net: str, runs: List[Tuple[Point, Point]]) -> None:
        self.paths[net] = runs
        max_track = max(self.rules.power_track_mm, self.rules.high_current_track_mm)
        # centerline separation of two full-width tracks must be >= clearance + width
        inflate = max(1, int(math.ceil((self.rules.min_clearance_mm + max_track) / self.grid)))
        cells: Set[Point] = set()
        for a, b in runs:
            for pt in _walk(a, b):
                cells.add(pt)
        occ = cells
        occ |= _inflate(cells, inflate) - self.terminal_cells[net]
        self.occ[net] = occ
        self._rebuild()

    def rip(self, net: str) -> None:
        self.occ.pop(net, None)
        self.paths.pop(net, None)
        self.rip_ups += 1
        self._rebuild()

    def _rebuild(self) -> None:
        busy: Set[Point] = set()
        for occ in self.occ.values():
            busy |= occ
        self.busy = busy

    def blocked_for(self, net: str) -> Set[Point]:
        blocked = self.busy | self.pad_block
        # release the clearance halo around this net's own pads so the path
        # can actually start/end there
        radius = max(1, int(math.ceil(self.rules.min_clearance_mm / self.grid)))
        blocked -= _inflate(self.terminal_cells[net], radius)
        return blocked


class Router:
    def __init__(self, rules: DesignRules, lut: LutProfile | None = None):
        self.rules = rules
        self.lut = lut or LutProfile()
        self.log: List[str] = []

    def _log(self, msg: str) -> None:
        self.log.append(msg)

    # -- single net routing with rip-up/reroute ------------------------------

    def _find_net_path(self, state: _State, net: str) -> List[Tuple[Point, Point]]:
        """A* spanning all terminals; returns a list of straight runs."""
        terminals = list(state.terminal_cells[net])
        if len(terminals) < 2:
            return []
        connected = {terminals[0]}
        remaining = set(terminals[1:])
        runs: List[Tuple[Point, Point]] = []
        cls = classify_net(net)
        cost_weight = _COST.get(cls, 1.0)
        avoid: Optional[Set[Point]] = None
        if cls in ("signal", "control", "feedback"):
            power_zone: Set[Point] = set()
            for other, occ in state.occ.items():
                if classify_net(other) in ("power", "high_current"):
                    power_zone |= _inflate(occ, _POWER_ZONE)
            avoid = power_zone
        while remaining:
            target = min(remaining, key=lambda p: min(_dist(p, c) for c in connected))
            segment = _astar(target, connected, state.blocked_for(net), state.bounds,
                             cost_weight=cost_weight, avoid=avoid)
            if not segment:
                return []
            connected.update(segment)
            remaining.discard(target)
            runs.extend(_compress(segment))
        return runs

    def _corridor(self, state: _State, net: str) -> Set[Point]:
        """Inflated bounding box of the net's terminals — where rip-up should look."""
        cells = list(state.terminal_cells[net])
        xs = [c[0] for c in cells]
        ys = [c[1] for c in cells]
        r = max(2, int(math.ceil((state.rules.power_track_mm + 2 * state.rules.min_clearance_mm) / state.grid)))
        corridor: Set[Point] = set()
        for x in range(min(xs) - r, max(xs) + r + 1):
            for y in range(min(ys) - r, max(ys) + r + 1):
                if min(_dist((x, y), c) for c in cells) <= r * 2:
                    corridor.add((x, y))
        return corridor

    def _route_net(self, state: _State, net: str, depth: int = 0) -> bool:
        runs = self._find_net_path(state, net)
        if runs:
            state.claim(net, runs)
            return True
        if depth >= _RIP_DEPTH:
            return False

        corridor = self._corridor(state, net)
        victims = []
        for other, occ in state.occ.items():
            if other != net and occ.intersection(corridor):
                victims.append((_net_key(other), -len(occ & corridor), other))
        victims.sort()
        ripped: List[str] = [other for _, _, other in victims[:_RIP_LIMIT]]
        if not ripped:
            return False
        for other in ripped:
            state.rip(other)
        runs = self._find_net_path(state, net)
        if not runs:
            return False
        state.claim(net, runs)
        for other in ripped:  # best-effort reroute of victims
            self._route_net(state, other, depth + 1)
        return True

    # -- full route ----------------------------------------------------------

    def route(self, board: Board, seed: int = 17, attempts: int = 6) -> RoutingSolution:
        self.log = []
        state = _State(board, self.rules, self.lut)
        order = _net_order(state.net_pads)
        best: Optional[RoutingSolution] = None
        rng = random.Random(seed)

        for attempt in range(max(1, attempts)):
            net_order = list(order)
            if attempt:
                tail = [n for n in net_order if _net_key(n)[0] >= 4]
                head = [n for n in net_order if n not in tail]
                rng.shuffle(tail)
                net_order = head + tail
            sol = self._run_improve(board, net_order, seed + attempt)
            if best is None or sol.score < best.score:
                best = sol
            self._log(f"attempt={attempt + 1} routed={len(sol.routed_nets)} failed={len(sol.failed_nets)} "
                      f"jumpers={sol.jumper_count} length={sol.total_length_mm:.1f}mm score={sol.score:.0f}")
            if sol.ok and sol.jumper_count == 0:
                break

        assert best is not None
        self._log(f"best: routed={len(best.routed_nets)} failed={len(best.failed_nets)} "
                  f"jumpers={best.jumper_count} length={best.total_length_mm:.1f}mm "
                  f"iterations={best.iterations} rip_ups={best.rip_ups}")
        return best

    def _run_improve(self, board: Board, net_order: List[str], seed: int) -> RoutingSolution:
        """Iterative improvement: replay the pass from a fresh state each round,
        keeping the cheapest solution (jumpers dominate, then total length)."""
        best_pass: Optional[RoutingSolution] = None
        rng = random.Random(seed)
        iteration = 0
        for iteration in range(_IMPROVE_PASSES):
            s = _State(board, self.rules, self.lut)
            order = list(net_order)
            if iteration:
                tail = [n for n in order if _net_key(n)[0] >= 4]
                head = [n for n in order if n not in tail]
                rng.shuffle(tail)
                order = head + tail
            sol = self._pass_once(s, order)
            if best_pass is None or sol.score < best_pass.score:
                best_pass = sol
                if sol.ok and sol.jumper_count == 0:
                    break
        best_pass.iterations = iteration + 1
        best_pass.rip_ups = best_pass.rip_ups
        return best_pass

    def _pass_once(self, state: _State, net_order: List[str]) -> RoutingSolution:
        solution = RoutingSolution()
        for net in net_order:
            if len(state.terminal_cells[net]) < 2:
                continue
            if self._route_net(state, net):
                solution.routed_nets.append(net)
            else:
                solution.failed_nets.append(net)
                pins = state.net_pads[net]
                for i in range(1, len(pins)):
                    _, _, x1, y1 = pins[0]
                    _, _, x2, y2 = pins[i]
                    length = math.hypot(x2 - x1, y2 - y1)
                    solution.jumpers.append(Jumper(net, x1, y1, x2, y2, length))
        solution.rip_ups = state.rip_ups
        for net in solution.routed_nets:
            width = state.rul_width(net)
            for a, b in state.paths[net]:
                solution.segments.append(RouteSegment(
                    net, a[0] * state.grid, a[1] * state.grid, b[0] * state.grid, b[1] * state.grid,
                    width, self.rules.copper_layer,
                ))
        solution.total_length_mm = sum(
            math.hypot(s.x2_mm - s.x1_mm, s.y2_mm - s.y1_mm) for s in solution.segments
        )
        solution.score = (len(solution.failed_nets) * 1_000_000
                          + solution.jumper_count * 1000
                          + solution.total_length_mm)
        return solution

    def _track_width(self, net: str) -> float:
        cls = classify_net(net)
        if cls in ("power", "ground"):
            return self.rules.power_track_mm
        if cls == "high_current":
            return self.rules.high_current_track_mm
        return max(self.rules.min_track_mm, self.lut.min_track_mm)
