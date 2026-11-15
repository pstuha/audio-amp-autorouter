"""Geometry helpers: rotated bounding boxes, overlaps, distances."""

from __future__ import annotations

import math
from typing import Iterable, Tuple

BBox = Tuple[float, float, float, float]  # minx, miny, maxx, maxy


def rotate_point(x: float, y: float, angle_deg: float) -> Tuple[float, float]:
    a = math.radians(angle_deg)
    ca, sa = math.cos(a), math.sin(a)
    return x * ca - y * sa, x * sa + y * ca


def component_bbox(
    x: float,
    y: float,
    rotation_deg: float,
    width_mm: float,
    height_mm: float,
) -> BBox:
    """Axis-aligned bbox of a rotated rectangle centred at (x, y)."""
    hw, hh = width_mm / 2.0, height_mm / 2.0
    corners = [(-hw, -hh), (hw, -hh), (hw, hh), (-hw, hh)]
    pts = []
    for cx, cy in corners:
        rx, ry = rotate_point(cx, cy, rotation_deg)
        pts.append((x + rx, y + ry))
    xs = [p[0] for p in pts]
    ys = [p[1] for p in pts]
    return min(xs), min(ys), max(xs), max(ys)


def bbox_overlap(a: BBox, b: BBox, gap: float = 0.0) -> bool:
    return not (
        a[2] + gap <= b[0]
        or b[2] + gap <= a[0]
        or a[3] + gap <= b[1]
        or b[3] + gap <= a[1]
    )


def bbox_intersection(a: BBox, b: BBox) -> Tuple[float, float]:
    ix = min(a[2], b[2]) - max(a[0], b[0])
    iy = min(a[3], b[3]) - max(a[1], b[1])
    return ix, iy


def bbox_contains(outer: BBox, inner: BBox, margin: float = 0.0) -> bool:
    return (
        inner[0] >= outer[0] + margin
        and inner[1] >= outer[1] + margin
        and inner[2] <= outer[2] - margin
        and inner[3] <= outer[3] - margin
    )


def point_in_bbox(x: float, y: float, bbox: BBox) -> bool:
    return bbox[0] <= x <= bbox[2] and bbox[1] <= y <= bbox[3]


def seg_distance(a: Tuple[float, float], b: Tuple[float, float], c: Tuple[float, float], d: Tuple[float, float]) -> float:
    """Minimum distance between two segments (approximate, via sampling)."""
    if _segments_intersect(a, b, c, d):
        return 0.0
    return min(
        _point_seg_distance(a, c, d),
        _point_seg_distance(b, c, d),
        _point_seg_distance(c, a, b),
        _point_seg_distance(d, a, b),
    )


def _point_seg_distance(p: Tuple[float, float], a: Tuple[float, float], b: Tuple[float, float]) -> float:
    px, py = p
    ax, ay = a
    bx, by = b
    dx, dy = bx - ax, by - ay
    if dx == 0 and dy == 0:
        return math.hypot(px - ax, py - ay)
    t = max(0.0, min(1.0, ((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)))
    return math.hypot(px - (ax + t * dx), py - (ay + t * dy))


def _segments_intersect(a, b, c, d) -> bool:
    def orientation(p, q, r):
        val = (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])
        if abs(val) < 1e-9:
            return 0
        return 1 if val > 0 else 2

    o1 = orientation(a, b, c)
    o2 = orientation(a, b, d)
    o3 = orientation(c, d, a)
    o4 = orientation(c, d, b)
    if o1 != o2 and o3 != o4:
        return True
    return False


def seg_bbox(a: Tuple[float, float], b: Tuple[float, float]) -> BBox:
    return min(a[0], b[0]), min(a[1], b[1]), max(a[0], b[0]), max(a[1], b[1])


def bbox_area(bbox: BBox) -> float:
    return max(0.0, bbox[2] - bbox[0]) * max(0.0, bbox[3] - bbox[1])


def union_bbox(boxes: Iterable[BBox]) -> BBox | None:
    boxes = list(boxes)
    if not boxes:
        return None
    return (
        min(b[0] for b in boxes),
        min(b[1] for b in boxes),
        max(b[2] for b in boxes),
        max(b[3] for b in boxes),
    )
