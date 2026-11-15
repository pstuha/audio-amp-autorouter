"""Report generation (JSON / Markdown)."""

from __future__ import annotations

import json
from dataclasses import asdict
from typing import Any, Dict

from .pipeline import PipelineResult


def to_dict(result: PipelineResult) -> Dict[str, Any]:
    return {
        "ok": result.ok,
        "placement": {
            "moves": {k: list(v) for k, v in result.placement.moves.items()},
            "score": result.placement.score,
        },
        "routing": {
            "routed_nets": result.routing.routed_nets,
            "failed_nets": result.routing.failed_nets,
            "total_length_mm": round(result.routing.total_length_mm, 2),
            "segments": [asdict(s) for s in result.routing.segments],
        },
        "jumpers": {
            "count": result.jumpers.count,
            "total_length_mm": round(result.jumpers.total_length_mm, 2),
            "items": [asdict(j) for j in result.jumpers.jumpers],
        },
        "geometry": {
            "ok": result.geometry_report.ok,
            "violations": [asdict(v) for v in result.geometry_report.violations],
        },
        "routing_validation": {
            "ok": result.routing_report.ok,
            "violations": [asdict(v) for v in result.routing_report.violations],
        },
        "log": result.log,
    }


def to_json(result: PipelineResult) -> str:
    return json.dumps(to_dict(result), ensure_ascii=False, indent=2)


def to_markdown(result: PipelineResult) -> str:
    lines = [
        "# AmpRoute Studio report",
        "",
        f"- Result: **{'OK' if result.ok else 'FAILED'}**",
        f"- Routed nets: {len(result.routing.routed_nets)}",
        f"- Failed nets: {len(result.routing.failed_nets)}",
        f"- Total length: {result.routing.total_length_mm:.1f} mm",
        f"- Jumpers: {result.jumpers.count}",
        "",
    ]
    if result.routing.failed_nets:
        lines.append("## Failed nets")
        lines.extend(f"- {n}" for n in result.routing.failed_nets)
        lines.append("")
    for title, report in (("Geometry", result.geometry_report), ("Routing", result.routing_report)):
        if report.violations:
            lines.append(f"## {title} violations")
            lines.extend(f"- [{v.severity}] {v.code}: {v.message}" for v in report.violations)
            lines.append("")
    return "\n".join(lines)
