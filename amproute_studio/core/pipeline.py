"""End-to-end pipeline: parse -> validate -> place -> route -> validate -> export."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Dict, List, Optional, Set, Tuple

from .jumpers import JumperPlan, plan_jumpers
from .models import Board, DesignRules, RouteSegment
from .placement import PlacementEngine, PlacementSolution
from .routing import Router, RoutingSolution
from .rules import LutProfile
from .simulations import (
    ThermalResult, CrosstalkResult, EMIPrediction,
    analyze_thermal, analyze_crosstalk, estimate_emi, analyze_power_bus,
    analyze_current_loops, run_full_simulation, SimulationReport,
)
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
    simulation_report: Optional[SimulationReport] = None

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
    sim_aware: bool = False  # Enable simulation-aware routing
    sim_attempts: int = 3   # Max simulation-aware improvement iterations


# --- Penalty Zones for Simulation-Aware Routing ---

@dataclass
class PenaltyZone:
    """A zone where routing cost is increased."""
    x_min: float
    y_min: float
    x_max: float
    y_max: float
    penalty: float  # Cost multiplier for tracks in this zone
    reason: str      # Human-readable reason


def create_penalty_zones_from_sim(
    board: Board,
    segments: List[RouteSegment],
    thermal_results: List[ThermalResult],
    crosstalk_results: List[CrosstalkResult],
    emi: Optional[EMIPrediction] = None,
    grid_mm: float = 0.25,
) -> List[PenaltyZone]:
    """Create penalty zones based on simulation results.
    
    Penalty zones increase routing cost in problematic areas:
    - Thermal hotspots: penalize routing high-current tracks near hot components
    - Crosstalk: penalize close parallel routing of affected pairs
    - EMI: penalize long loops (handled by preference for shorter routes)
    """
    zones: List[PenaltyZone] = []
    
    # --- Thermal penalty zones ---
    if thermal_results:
        for t in thermal_results:
            if t.junction_temp_c > 80:  # Hot component
                # Get component position
                if t.component_ref in board.components:
                    comp = board.components[t.component_ref]
                    # Create penalty zone around hot component
                    zone_extent = 5.0  # mm
                    zones.append(PenaltyZone(
                        x_min=comp.x_mm - zone_extent,
                        y_min=comp.y_mm - zone_extent,
                        x_max=comp.x_mm + zone_extent,
                        y_max=comp.y_mm + zone_extent,
                        penalty=2.0 if t.junction_temp_c > 100 else 1.5,
                        reason=f"thermal_hot_{t.component_ref}_{t.junction_temp_c:.0f}C",
                    ))
    
    # --- Crosstalk penalty zones ---
    if crosstalk_results:
        for ct in crosstalk_results:
            if ct.critical:
                # Find segments of both nets and create penalty zones between them
                # to encourage wider spacing
                net_a_segs = [s for s in segments if getattr(s, 'net', None) == ct.net_a]
                net_b_segs = [s for s in segments if getattr(s, 'net', None) == ct.net_b]
                
                if net_a_segs and net_b_segs:
                    # Find closest approach between the two nets
                    min_gap = float('inf')
                    for sa in net_a_segs:
                        for sb in net_b_segs:
                            # Simple distance check between segment centers
                            mid_a = ((sa.x1 + sa.x2) / 2, (sa.y1 + sa.y2) / 2)
                            mid_b = ((sb.x1 + sb.x2) / 2, (sb.y1 + sb.y2) / 2)
                            gap = ((mid_a[0] - mid_b[0])**2 + (mid_a[1] - mid_b[1])**2)**0.5
                            min_gap = min(min_gap, gap)
                    
                    if min_gap < 5.0:  # Close routing - needs more separation
                        # Create zone between them to push routing apart
                        center_x = sum(s.x1 + s.x2 for s in net_a_segs[:1]) / 2
                        center_y = sum(s.y1 + s.y2 for s in net_a_segs[:1]) / 2
                        zones.append(PenaltyZone(
                            x_min=center_x - min_gap,
                            y_min=center_y - min_gap,
                            x_max=center_x + min_gap,
                            y_max=center_y + min_gap,
                            penalty=3.0,
                            reason=f"crosstalk_{ct.net_a}_{ct.net_b}",
                        ))
    
    return zones


def points_in_penalty_zones(
    zones: List[PenaltyZone],
    grid_mm: float = 0.25,
) -> Set[Tuple[int, int]]:
    """Convert penalty zones to grid points for A* avoid set."""
    avoid: Set[Tuple[int, int]] = set()
    
    for zone in zones:
        # Convert mm to grid cells
        x_start = int(round(zone.x_min / grid_mm))
        x_end = int(round(zone.x_max / grid_mm))
        y_start = int(round(zone.y_min / grid_mm))
        y_end = int(round(zone.y_max / grid_mm))
        
        for x in range(x_start, x_end + 1):
            for y in range(y_start, y_end + 1):
                # Only add points that are in high-penalty zones
                if zone.penalty >= 2.0:
                    avoid.add((x, y))
    
    return avoid


def apply_penalty_zones_to_astar(avoid: Set[Tuple[int, int]], segments: List[RouteSegment]) -> None:
    """Inject penalty zones into A* avoid set.
    
    This is a simplified approach - we mark specific grid points as avoided.
    The actual A* implementation reads from module-level _penalty_avoid.
    """
    global _current_penalty_avoid
    _current_penalty_avoid = avoid


# Module-level storage for penalty zones (accessed by A*)
_current_penalty_avoid: Set[Tuple[int, int]] = set()


def get_penalty_multiplier(point: Tuple[int, int], zones: List[PenaltyZone], grid_mm: float = 0.25) -> float:
    """Get cost multiplier for a point based on penalty zones."""
    x_mm = point[0] * grid_mm
    y_mm = point[1] * grid_mm
    
    max_penalty = 1.0
    for zone in zones:
        if zone.x_min <= x_mm <= zone.x_max and zone.y_min <= y_mm <= zone.y_max:
            max_penalty = max(max_penalty, zone.penalty)
    
    return max_penalty


def run_sim_aware_pipeline(
    board: Board,
    options: Optional[PipelineOptions] = None,
) -> PipelineResult:
    """Run pipeline with simulation-aware routing feedback.
    
    This function:
    1. Does initial routing
    2. Runs simulations on the routed board
    3. If simulation results are poor, creates penalty zones
    4. Re-routes with penalty zones to improve results
    5. Repeats until good results or max iterations
    """
    from .routing import Router, _IMPROVE_PASSES
    
    opts = options or PipelineOptions()
    log: List[str] = []
    sim_report: Optional[SimulationReport] = None
    
    # Initial routing (no simulation feedback)
    geometry_report = validate_geometry(board, opts.rules)
    log.append(f"geometry: {len(geometry_report.errors)} errors, {len(geometry_report.warnings)} warnings")
    
    placement = PlacementSolution()
    if opts.place:
        placement = PlacementEngine(opts.rules).run(board)
        log.extend(placement.log)
        geometry_report = validate_geometry(board, opts.rules)
        log.append(f"geometry after placement: {len(geometry_report.errors)} errors")
    
    # Initial routing
    router = Router(opts.rules, opts.lut)
    
    if opts.route:
        # Apply initial routing
        routing = router.route(board, seed=opts.seed, attempts=opts.attempts)
        log.extend(routing.notes)
        
        # If not simulation-aware, just return normal result
        if not opts.sim_aware:
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
        
        # Simulation-aware iterative improvement
        log.append("Starting simulation-aware routing improvement...")
        
        for sim_iter in range(opts.sim_attempts):
            log.append(f"=== Simulation-aware iteration {sim_iter + 1} ===")
            
            # Attach current segments to board for simulation
            board = _attach_segments(board, routing.segments)
            
            # Run simulations
            sim_report = run_full_simulation(board)
            log.append(f"Simulation: DF={sim_report.audio.damping_factor:.2f}, "
                      f"THD={sim_report.audio.thd_percent:.4f}%")
            
            # Check if results are acceptable
            thermal_ok = all(t.junction_temp_c < 100 for t in sim_report.thermal_results) if sim_report.thermal_results else True
            crosstalk_ok = not any(cr.critical for cr in sim_report.crosstalk) if sim_report.crosstalk else True
            
            if thermal_ok and crosstalk_ok:
                log.append("Simulation results acceptable - keeping current routing")
                break
            
            # Analyze results and create penalty zones
            thermal_bad = [t for t in sim_report.thermal_results if t.junction_temp_c > 80] if sim_report.thermal_results else []
            crosstalk_bad = [c for c in sim_report.crosstalk if c.critical] if sim_report.crosstalk else []
            
            if thermal_bad:
                log.append(f"Thermal issues: {[t.component_ref for t in thermal_bad]}")
            if crosstalk_bad:
                log.append(f"Crosstalk issues: {[(c.net_a, c.net_b) for c in crosstalk_bad]}")
            
            # Create penalty zones
            zones = create_penalty_zones_from_sim(
                board, routing.segments,
                thermal_bad, crosstalk_bad,
                sim_report.emi,
                opts.rules.grid_mm,
            )
            
            if zones:
                log.append(f"Created {len(zones)} penalty zones for next iteration")
            
            # Convert penalty zones to avoid points
            avoid = points_in_penalty_zones(zones, opts.rules.grid_mm)
            
            # Clear and re-route with penalty zones
            # This requires re-initializing the router with penalty avoidance
            router = Router(opts.rules, opts.lut)
            
            # We need to inject penalty zones into the routing
            # For now, we'll do a fresh route with modified constraints
            new_routing = _route_with_penalty(board, router, opts, zones)
            
            if new_routing and new_routing.score < routing.score:
                log.append(f"Improved routing: score {routing.score:.0f} -> {new_routing.score:.0f}")
                routing = new_routing
            else:
                log.append("No improvement in this iteration")
                break
        
        # Final routing result
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
        simulation_report=sim_report,
    )


def _attach_segments(board: Board, segments: List[RouteSegment]) -> Board:
    """Attach routing segments to board for simulation."""
    board.segments = segments
    return board


def _route_with_penalty(
    board: Board,
    router: Router,
    opts: PipelineOptions,
    zones: List[PenaltyZone],
) -> Optional[RoutingSolution]:
    """Route with penalty zones applied.
    
    penalty_zones are grid points where routing cost is 4x higher,
    encouraging the router to find alternate paths around problematic areas.
    """
    avoid = points_in_penalty_zones(zones, opts.rules.grid_mm)
    if not avoid:
        return router.route(board, seed=opts.seed, attempts=opts.attempts)
    return router.route_with_penalty_zones(board, avoid, seed=opts.seed, attempts=opts.attempts)


def run_pipeline(
    board: Board,
    options: Optional[PipelineOptions] = None,
) -> PipelineResult:
    """Standard pipeline without simulation feedback.
    
    For simulation-aware routing, use run_sim_aware_pipeline().
    """
    opts = options or PipelineOptions()
    
    # If simulation-aware was requested, use the sim-aware version
    if opts.sim_aware:
        return run_sim_aware_pipeline(board, options)
    
    return _run_pipeline_basic(board, options)


def _run_pipeline_basic(
    board: Board,
    options: Optional[PipelineOptions] = None,
) -> PipelineResult:
    """Basic pipeline without simulation awareness."""
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
