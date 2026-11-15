"""KiCad .kicad_pcb read/write adapter (S-expression based)."""

from __future__ import annotations

import re
from pathlib import Path
from typing import List

from ..core.models import Board, Component, Jumper, Net, Pad, RouteSegment


def _tokenize(text: str) -> List[str]:
    return re.findall(r'\(|\)|"(?:\\.|[^"\\])*"|[^\s()"]+', text)


def _parse(text: str):
    tokens = _tokenize(text)
    stack: List[list] = []
    root = None
    for tok in tokens:
        if tok == "(":
            stack.append([])
        elif tok == ")":
            node = stack.pop()
            if stack:
                stack[-1].append(node)
            else:
                root = node
        else:
            if len(tok) >= 2 and tok[0] == '"' and tok[-1] == '"':
                tok = tok[1:-1].replace('\\"', '"').replace('\\\\', '\\')
            stack[-1].append(tok)
    return root


def _all(node, name):
    return [n for n in node if isinstance(n, list) and n and n[0] == name]


def _first(node, name, default=None):
    return next(iter(_all(node, name)), default)


def load_board(path: str | Path) -> Board:
    root = _parse(Path(path).read_text(encoding="utf-8-sig"))
    board = Board()

    for net in _all(root, "net"):
        if len(net) >= 3:
            try:
                board.nets[str(net[2])] = Net(str(net[2]), int(net[1]))
            except ValueError:
                pass

    for fp in _all(root, "footprint"):
        ref = val = ""
        layer = "F.Cu"
        x = y = rot = 0.0
        pads: List[Pad] = []

        for prop in _all(fp, "property"):
            if len(prop) >= 3:
                if prop[1] == "Reference":
                    ref = str(prop[2])
                elif prop[1] == "Value":
                    val = str(prop[2])

        layer_node = _first(fp, "layer")
        if layer_node:
            layer = str(layer_node[1])

        at = _first(fp, "at")
        if at:
            x = float(at[1])
            y = float(at[2])
            rot = float(at[3]) if len(at) > 3 else 0.0

        for pad in _all(fp, "pad"):
            if len(pad) < 4:
                continue
            at_pad = _first(pad, "at")
            size = _first(pad, "size")
            drill = _first(pad, "drill")
            net_node = _first(pad, "net")
            px = float(at_pad[1]) if at_pad else 0.0
            py = float(at_pad[2]) if at_pad else 0.0
            sx = float(size[1]) if size else 1.0
            sy = float(size[2]) if size and len(size) > 2 else sx
            dr = float(drill[1]) if drill and len(drill) > 1 else 0.0
            netname = str(net_node[2]) if net_node and len(net_node) > 2 else (str(net_node[1]) if net_node and len(net_node) > 1 else "")
            pads.append(Pad(str(pad[1]), netname, px, py, sx, sy, dr, str(pad[3])))

        comp = Component(
            ref=ref, value=val, footprint="", x_mm=x, y_mm=y, rotation_deg=rot,
            layer=layer, pads=pads,
        )
        if pads:
            xs = [p.x_mm for p in pads]
            ys = [p.y_mm for p in pads]
            comp.width_mm = max(xs) - min(xs) or 4.0
            comp.height_mm = max(ys) - min(ys) or 4.0
            comp.courtyard_width_mm = comp.width_mm + 2.0
            comp.courtyard_height_mm = comp.height_mm + 2.0
        board.components[ref] = comp
        for p in pads:
            if p.net:
                board.nets.setdefault(p.net, Net(p.net)).pins.append((ref, p.number))

    edge_points = []
    for name in ("gr_line", "gr_rect", "gr_circle", "gr_arc"):
        for shape in _all(root, name):
            for token in shape:
                if isinstance(token, list) and token and token[0] in ("start", "end", "center"):
                    try:
                        edge_points.append((float(token[1]), float(token[2])))
                    except (TypeError, ValueError, IndexError):
                        pass
    if edge_points:
        xs = [p[0] for p in edge_points]
        ys = [p[1] for p in edge_points]
        board.bounds_mm = (min(xs), min(ys), max(xs), max(ys))

    return board


def save_board(board: Board, path: str | Path, segments: List[RouteSegment], jumpers: List[Jumper]) -> None:
    """Write a minimal board outline plus routes and jumpers.

    The original adapter is intentionally conservative: it serialises the normalized
    model, not the full KiCad file. Use the KiCad adapter for full round-trips.
    """
    out = ["(kicad_pcb (version 20240108) (generator amprote)"]
    for comp in board.components.values():
        out.append(
            f'  (footprint "{comp.footprint}" (layer "{comp.layer}") (at {comp.x_mm:.3f} {comp.y_mm:.3f} {comp.rotation_deg:.1f})'
            f' (property "Reference" "{comp.ref}") (property "Value" "{comp.value}"))'
        )
    for seg in segments:
        out.append(
            f'  (segment (start {seg.x1_mm:.3f} {seg.y1_mm:.3f}) (end {seg.x2_mm:.3f} {seg.y2_mm:.3f})'
            f' (width {seg.width_mm:.3f}) (layer "{seg.layer}") (net "{seg.net}"))'
        )
    out.append(")")
    Path(path).write_text("\n".join(out), encoding="utf-8")
