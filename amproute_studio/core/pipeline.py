"""End-to-end pipeline: parse -> validate -> place -> route -> validate -> export."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import List, Optional

from .jumpers import JumperPlan, plan_jumpers
from .models import Board, DesignRules
from .placement import PlacementEngine, PlacementSolution
from .routing import Router, RoutingSolution
from .rules import LutProfile
from .validation import ValidationReport, validate_geometry, validate_routing


@dataclass
class PipelineResult:
    board: Board
    placement: PlacementSolution
    routing: RoutingSolution
    jumpers: JumperPlan
    geometry_report: ValidationReport
    routing_report: ValidationReport
    log: List[str] = field(default_factory=list)

    @property
    def ok(self) -> bool:
        return self.geometry_report.ok and self.routing_report.ok


@dataclass
class PipelineOptions:
    rules: DesignRules = field(default_factory=DesignRules)
    lut: LutProfile = field(default_factory=LutProfile)
    seed: int = 17
    attempts: int = 6
    place: bool = True
    route: bool = True


def run_pipeline(
    board: Board,
    options: Optional[PipelineOptions] = None,
) -> PipelineResult:
    opts = options or PipelineOptions()
    log: List[str] = []

    geometry_report = validate_geometry(board, opts.rules)
    log.append(f"geometry: {len(geometry_report.errors)} errors, {len(geometry_report.warnings)} warnings")

    placement = PlacementSolution()
    if opts.place:
        placement = PlacementEngine(opts.rules).run(board)
        log.extend(placement.log)
        geometry_report = validate_geometry(board, opts.rules)
        log.append(f"geometry after placement: {len(geometry_report.errors)} errors")

    routing = RoutingSolution()
    jumpers = JumperPlan()
    routing_report = ValidationReport()
    if opts.route:
        routing = Router(opts.rules, opts.lut).route(board, seed=opts.seed, attempts=opts.attempts)
        log.extend(routing.notes)
        jumpers = plan_jumpers(board, routing.segments, opts.lut, routing.failed_nets)
        log.extend(jumpers.notes)
        jumpered_nets = {j.net for j in jumpers.jumpers}
        routing_report = validate_routing(board, routing.segments, opts.rules, opts.lut, jumpered_nets)
        log.append(f"routing: {len(routing_report.errors)} errors")

    return PipelineResult(
        board=board,
        placement=placement,
        routing=routing,
        jumpers=jumpers,
        geometry_report=geometry_report,
        routing_report=routing_report,
        log=log,
    )
