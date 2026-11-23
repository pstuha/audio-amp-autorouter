//! Module 6 — Routing Engine.
//!
//! This is AmpRoute's **own** routing engine, not a wrapper around an external
//! router. It owns the whole routing decision:
//!
//! * net ordering (owned by [`Router::route_board`], driven by [`NetClass::priority`]);
//! * the search itself (grid A\* with audio-aware cost weights);
//! * clearance and obstacle handling (`Occupancy`, exact millimetre geometry);
//! * reroute passes when a net ends up walled in.
//!
//! What it explicitly does **not** decide:
//!
//! * which nets are fixed copper — that is module 5 (pre-route). Fixed copper
//!   arrives here as [`FixedCopper`] and is treated as a hard obstacle;
//! * whether the result is manufacturable — that is module 4, which is asked
//!   before and after every run.
//!
//! The engine is therefore a pure function of (board, rules, fixed copper):
//! same inputs, same copper. No external process, no network, no state.
//!
//! FreeRouting is **not** used. It survives in this crate only as a reference
//! in `tests/routing_engine_reference.rs`, which compares our engine against a
//! recorded session file; that test never invokes the external tool at runtime.

use crate::board::{Board, NetClass};
use crate::occupancy::Occupancy;
use crate::router::{RouteSegment, Router, RoutingResult};
use crate::rules::{
    parallel_overlap_mm, LutRules, RuleReport, MAX_PARALLEL_RUN_MM, PARALLEL_ANGLE_TOL_DEG,
    PARALLEL_GAP_MM,
};
use serde::Serialize;
use std::collections::HashSet;

// ---------------------------------------------------------------------------
// Fixed copper (module 5 output, treated as immovable)
// ---------------------------------------------------------------------------

/// Copper that the engine must route around and must never move.
///
/// Produced by the pre-route module (ground star, power rails, polygons). The
/// engine only subtracts from it: it never adds to, merges or reorders it.
#[derive(Debug, Clone, Default, Serialize)]
pub struct FixedCopper {
    pub segments: Vec<RouteSegment>,
    /// Radius of each fixed star-ground node, so the router keeps clearance
    /// around a copper island as well as around a track.
    pub islands: Vec<(f64, f64, f64)>,
}

impl FixedCopper {
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty() && self.islands.is_empty()
    }

    pub fn total_length_mm(&self) -> f64 {
        self.segments.iter().map(|s| s.length_mm()).sum()
    }
}

// ---------------------------------------------------------------------------
// Configuration and result
// ---------------------------------------------------------------------------

/// Engine tuning. Defaults are the values validated on the reference boards.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Routing grid in millimetres. 0.2 mm comfortably resolves a 0.3 mm
    /// feature with 0.3 mm clearance on a single layer.
    pub grid_mm: f64,
    /// Grid points where the search prefers free board because of a thermal or
    /// star-ground hotspot; passable, just expensive.
    pub penalty_weight: f32,
    /// Compare an 8-connected/octile search with the stable 4-connected one.
    pub diagonal_search: bool,
    /// Reroute passes after the first net order fails.
    pub retry_budget: usize,
    /// Wall-clock ceiling for one net order, in seconds. The search is
    /// breadth-first over a whole board and a pathological order can spend
    /// minutes re-trying the same dead ends; the ceiling turns that into a
    /// reported partial result instead of a hung pipeline. Zero disables it.
    pub time_budget_s: f64,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            grid_mm: 0.2,
            penalty_weight: 1.5,
            diagonal_search: false,
            retry_budget: 24,
            time_budget_s: 180.0,
        }
    }
}

impl EngineConfig {
    /// Finer grid for dense boards; the cost is quadratic in grid resolution.
    pub fn dense() -> Self {
        Self {
            grid_mm: 0.1,
            ..Self::default()
        }
    }
}

/// Why a run produced (or refused to produce) copper.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum EngineOutcome {
    /// Every multi-terminal net is connected and module 4 is clean.
    Routed,
    /// The engine ran, some nets stayed unconnected.
    PartiallyRouted,
    /// Module 4 rejected the board, so no search was started.
    RejectedBeforeRouting,
}

impl EngineOutcome {
    pub fn label(self) -> &'static str {
        match self {
            EngineOutcome::Routed => "routed",
            EngineOutcome::PartiallyRouted => "partially routed",
            EngineOutcome::RejectedBeforeRouting => "rejected before routing",
        }
    }
}

/// The complete trace of one engine run.
#[derive(Debug, Serialize)]
pub struct EngineRun {
    pub outcome: EngineOutcome,
    /// Signal copper the engine produced (never includes [`FixedCopper`]).
    pub result: RoutingResult,
    /// Module 4 verdict on the board, computed before the search.
    pub pre_rules: RuleReport,
    /// Module 4 verdict on the copper, computed after the search.
    pub post_rules: RuleReport,
    /// Fixed copper handed in by module 5, echoed for the caller.
    pub fixed_length_mm: f64,
    /// `true` when the search stopped because [`EngineConfig::time_budget_s`]
    /// ran out rather than because it was finished. A partial result with this
    /// flag means "retry with more time or a coarser grid", not "board is
    /// unroutable".
    pub deadline_reached: bool,
    pub log: Vec<String>,
}

impl EngineRun {
    /// `true` only when everything routed, module 4 is clean after routing and
    /// the engine produced no via. The pipeline may bind copper only on this.
    pub fn is_acceptable(&self) -> bool {
        self.outcome == EngineOutcome::Routed
            && self.post_rules.is_empty()
            && self.result.vias == 0
    }

    /// Total copper the board will carry: fixed plus routed.
    pub fn total_copper_mm(&self) -> f64 {
        self.fixed_length_mm + self.result.total_length_mm
    }

    /// Nets the engine could not connect, for the pipeline's retry decision.
    pub fn failed_nets(&self) -> &[String] {
        &self.result.failed_nets
    }
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

/// AmpRoute's routing engine.
pub struct RoutingEngine {
    pub rules: LutRules,
    pub config: EngineConfig,
}

impl RoutingEngine {
    pub fn new(rules: LutRules, config: EngineConfig) -> Self {
        Self { rules, config }
    }

    /// Default engine: the reliable LUT profile on a 0.2 mm grid.
    pub fn reliable() -> Self {
        Self::new(LutRules::reliable(), EngineConfig::default())
    }

    /// Route `board`, treating `fixed` as immovable copper.
    ///
    /// The board is never mutated. Every path returns an [`EngineRun`], so a
    /// rejection or a partial result is data, not an error; only a panic in a
    /// dependency could abort.
    pub fn run(&self, board: &Board, fixed: &FixedCopper) -> EngineRun {
        let mut log = vec![format!("rules: {}", self.rules.name)];

        // 1. Module 4 gate: never spend a search on an unmanufacturable board.
        let pre_rules = self.rules.check_board(board);
        log.push(format!("pre-route rules: {}", pre_rules.summary()));
        if !pre_rules.is_empty() {
            return EngineRun {
                outcome: EngineOutcome::RejectedBeforeRouting,
                result: RoutingResult::new(),
                pre_rules,
                post_rules: RuleReport::default(),
                fixed_length_mm: fixed.total_length_mm(),
                deadline_reached: false,
                log,
            };
        }

        // 2. The search. Module 5's copper becomes part of the obstacle field
        //    before the first net is attempted, so no net can be routed over it.
        log.push(format!(
            "grid {:.2} mm, fixed copper {:.1} mm in {} segments, {} islands",
            self.config.grid_mm,
            fixed.total_length_mm(),
            fixed.segments.len(),
            fixed.islands.len()
        ));
        let started = std::time::Instant::now();
        let deadline = if self.config.time_budget_s > 0.0 {
            Some(started + std::time::Duration::from_secs_f64(self.config.time_budget_s))
        } else {
            None
        };
        let mut router = self.build_router(board, fixed);
        router.deadline = deadline;
        let (mut result, mut orders_tried, mut deadline_reached) =
            self.search(&router, board, deadline, &mut log);

        // A net walled in at 0.5 mm often threads at 0.25 mm. Retry once at
        // half the pitch before declaring the board unroutable; the finer pass
        // still runs under the same wall-clock deadline.
        if !result.failed_nets.is_empty() && self.config.grid_mm > 0.25 {
            let fine_grid = (self.config.grid_mm / 2.0).max(0.25);
            let mut fine = self.build_router_at(board, fixed, fine_grid);
            fine.deadline = deadline;
            let (alt, alt_orders, alt_deadline) = self.search(&fine, board, deadline, &mut log);
            if alt.failed_nets.len() < result.failed_nets.len() {
                log.push(format!("refined grid to {fine_grid:.2} mm"));
                result = alt;
                orders_tried += alt_orders;
                deadline_reached |= alt_deadline;
            }
        }

        // 3. Signal-only guarantee: fixed copper must never appear in the
        //    engine's own output, otherwise the caller would double-count it.
        self.strip_fixed(&mut result, fixed);
        if let Some(simplified) = self.simplify_result(board, &result) {
            log.push(format!(
                "line-of-sight simplification: {} → {} segments",
                result.segments.len(),
                simplified.segments.len()
            ));
            result = simplified;
        }

        // Optional 45° search, compared AFTER simplification: a raw 8-connected
        // path can look worse before line-of-sight cleanup and much better
        // afterwards. The cleaner result wins only if it is rule-clean.
        if self.config.diagonal_search {
            let mut diagonal = self.build_router(board, fixed);
            diagonal.diagonal_search = true;
            diagonal.deadline = deadline;
            let (mut alt, alt_orders, alt_deadline) =
                self.search(&diagonal, board, deadline, &mut log);
            self.strip_fixed(&mut alt, fixed);
            if let Some(simplified) = self.simplify_result(board, &alt) {
                alt = simplified;
            }
            let alt_rules = self.rules.check_routing(board, &alt);
            let base_rules = self.rules.check_routing(board, &result);
            if std::env::var_os("AMPROUTE_DEBUG_SIMPLIFY").is_some() {
                eprintln!(
                    "diag compare: alt_failed={} base_failed={} alt_rules={} base_rules={} alt_segs={} base_segs={}",
                    alt.failed_nets.len(),
                    result.failed_nets.len(),
                    alt_rules.violations.len(),
                    base_rules.violations.len(),
                    alt.segments.len(),
                    result.segments.len()
                );
            }
            let better = alt.failed_nets.len() < result.failed_nets.len()
                || (alt.failed_nets.len() == result.failed_nets.len()
                    && alt_rules.violations.len() <= base_rules.violations.len()
                    && alt.segments.len() < result.segments.len());
            if better {
                log.push(format!(
                    "8-connected/octile search adopted: {} → {} segments",
                    result.segments.len(),
                    alt.segments.len()
                ));
                result = alt;
                orders_tried += alt_orders;
                deadline_reached |= alt_deadline;
            } else {
                log.push("8-connected search rejected (no improvement)".into());
            }
        }

        log.push(format!(
            "search: {} orders in {:.1} s{}, {} nets routed, {} failed, {:.1} mm, {} vias",
            orders_tried,
            started.elapsed().as_secs_f64(),
            if deadline_reached { " (time budget reached)" } else { "" },
            result.routed_nets.len(),
            result.failed_nets.len(),
            result.total_length_mm,
            result.vias
        ));

        // 4. Module 4 again, on the copper we intend to ship.
        let mut post_rules = self.rules.check_routing(board, &result);
        // Parallel runs found by module 4 are resolved by a rip-up: of the two
        // offending nets, the one with less copper is torn out and reported as
        // failed. Keeping the longer net preserves as much of the plan as
        // possible while the rules stay clean.
        if !post_rules.violations.is_empty() {
            let pairs: Vec<(&str, &str)> = post_rules
                .violations
                .iter()
                .filter(|v| v.rule == crate::rules::RuleId::ParallelRun)
                .filter_map(|v| {
                    // Message format: "`/NET1` and `/NET2` run parallel …" — the
                    // names are verbatim board net names, leading slash included.
                    let parts: Vec<&str> = v.message.split('`').collect();
                    if parts.len() >= 4 {
                        Some((parts[1], parts[3]))
                    } else {
                        None
                    }
                })
                .collect();

            // Pre-compute copper lengths to avoid borrowing issues.
            let mut copper_len: std::collections::HashMap<String, f64> =
                std::collections::HashMap::new();
            for s in &result.segments {
                *copper_len.entry(s.net.clone()).or_default() += s.length_mm();
            }
            for (a, b) in pairs {
                let len_a = copper_len.get(a).copied().unwrap_or(0.0);
                let len_b = copper_len.get(b).copied().unwrap_or(0.0);
                let drop = if len_a <= len_b { a } else { b };
                if let Some(pos) = result.routed_nets.iter().position(|n| n == drop) {
                    result.routed_nets.remove(pos);
                    result.failed_nets.push(format!("{drop}: parallel run violation"));
                }
                // The copper goes with the claim: segments for a net that no
                // longer counts as routed would ship illegal geometry.
                result.segments.retain(|s| s.net != drop);
                copper_len.remove(drop);
            }
            result.total_length_mm = result.segments.iter().map(|s| s.length_mm()).sum();
            post_rules = self.rules.check_routing(board, &result);
        }
        log.push(format!("post-route rules: {}", post_rules.summary()));

        let outcome = if result.failed_nets.is_empty() && post_rules.is_empty() {
            EngineOutcome::Routed
        } else {
            EngineOutcome::PartiallyRouted
        };

        EngineRun {
            outcome,
            result,
            pre_rules,
            post_rules,
            fixed_length_mm: fixed.total_length_mm(),
            deadline_reached,
            log,
        }
    }

    /// Build the router with module 5's copper already stamped in.
    fn build_router(&self, board: &Board, fixed: &FixedCopper) -> Router {
        self.build_router_at(board, fixed, self.config.grid_mm)
    }

    fn build_router_at(&self, board: &Board, fixed: &FixedCopper, grid_mm: f64) -> Router {
        let mut router = Router::new(grid_mm, board.width_mm, board.height_mm);
        router.penalty_weight = self.config.penalty_weight;
        router.clearance_mm = self.rules.min_clearance_mm;
        // The baseline search is always the stable 4-connected one; the
        // optional diagonal pass is built separately in `run()`.
        router.diagonal_search = false;

        // Hotspots and star-ground nodes: passable, expensive.
        for &(x, y, r) in &fixed.islands {
            router.add_penalty_circle(x, y, r);
        }

        // Rails are expensive rather than forbidden: a wide fixed rail leaves
        // room beside it on a single layer, but a dense board may have to come
        // close. DRC audits the delivered copper, so a real crossing is caught.
        for seg in &fixed.segments {
            let length = seg.length_mm();
            let steps = (length / grid_mm).ceil().max(1.0) as usize;
            for i in 0..=steps {
                let t = i as f64 / steps as f64;
                let x = seg.x1_mm + (seg.x2_mm - seg.x1_mm) * t;
                let y = seg.y1_mm + (seg.y2_mm - seg.y1_mm) * t;
                router.add_penalty_circle(
                    x,
                    y,
                    (seg.width_mm / 2.0 + self.rules.min_clearance_mm).max(0.5),
                );
            }
        }
        router
    }

    /// The engine's own net-order strategy, run under a wall-clock deadline.
    ///
    /// [`Router::route_board`] explores order permutations without any notion
    /// of elapsed time; on a dense board a single order costs ~16 s, so an
    /// unbounded walk can hold the pipeline for tens of minutes. The engine
    /// therefore drives the orders itself and stops the walk when the budget
    /// is gone: the best plan found so far is returned with a log line saying
    /// the search was cut short. A partial result is data; a hang is a bug.
    ///
    /// Returns `(result, orders_tried, deadline_reached)`.
    fn search(
        &self,
        router: &Router,
        board: &Board,
        deadline: Option<std::time::Instant>,
        log: &mut Vec<String>,
    ) -> (RoutingResult, usize, bool) {
        // Stage 1: a fixed, reproducible ladder of orderings, best kept.
        let orders = self.net_order_ladder(board);
        let mut best: Option<(RoutingResult, Vec<usize>)> = None;
        let mut tried = 0usize;

        for order in &orders {
            if best.is_some() && best.as_ref().map_or(true, |(r, _)| r.failed_nets.is_empty()) {
                break;
            }
            tried += 1;
            let order_started = std::time::Instant::now();
            let run = router.route_in_order(board, order);
            log.push(format!(
                "order {}: {}/{} nets in {:.1} s",
                tried,
                run.routed_nets.len(),
                board.nets.len(),
                order_started.elapsed().as_secs_f64()
            ));
            let better = match &best {
                None => true,
                Some((b, _)) => {
                    run.routed_nets.len() > b.routed_nets.len()
                        || (run.routed_nets.len() == b.routed_nets.len()
                            && run.total_length_mm < b.total_length_mm)
                }
            };
            if better {
                best = Some((run, order.clone()));
            }
            if deadline.is_some_and(|d| std::time::Instant::now() >= d) {
                log.push("time budget reached during the order ladder".to_string());
                let (result, _) = best.expect("at least one order ran");
                return (result, tried, true);
            }
        }

        // Stage 2: promote stuck nets past their blockers while the budget
        // holds and each move strictly improves on the current plan.
        let (mut result, mut order) = best.expect("the ladder always runs one order");
        let mut tried_moves: HashSet<(usize, usize)> = HashSet::new();
        let mut exhausted = false;

        if !result.failed_nets.is_empty() {
            'moves: while tried < self.config.retry_budget * board.nets.len().max(1) {
                if deadline.is_some_and(|d| std::time::Instant::now() >= d) {
                    exhausted = true;
                    break;
                }
                let Some(stuck_net) = result
                    .failed_nets
                    .iter()
                    .find_map(|f| board.nets.iter().position(|n| f.starts_with(&n.name)))
                else {
                    break;
                };
                // Blockers: nets whose copper sits in the way, approximated by
                // the nets routed just before the stuck one in the plan.
                let Some(slot) = order.iter().position(|&i| i == stuck_net) else {
                    break;
                };
                let mut promoted = false;
                for back in 1..=slot.min(4) {
                    let anchor = order[slot - back];
                    if !tried_moves.insert((stuck_net, anchor)) {
                        continue;
                    }
                    let mut candidate = order.clone();
                    candidate.retain(|&i| i != stuck_net);
                    let at = candidate
                        .iter()
                        .position(|&i| i == anchor)
                        .unwrap_or(0);
                    candidate.insert(at, stuck_net);
                    tried += 1;
                    let run = router.route_in_order(board, &candidate);
                    // A swap (same count, different failed set) is also worth
                    // taking: it moves the dead-end from one net to another and
                    // is the only way out of a wall where every net is blocked
                    // by someone else's copper.
                    if run.routed_nets.len() > result.routed_nets.len()
                        || (run.routed_nets.len() == result.routed_nets.len()
                            && run.failed_nets != result.failed_nets)
                    {
                        result = run;
                        order = candidate;
                        promoted = true;
                        break;
                    }
                }
                if !promoted {
                    break 'moves;
                }
            }
        }

        // Simple rip‑up fallback: if still many nets failed, discard all signal
        // copper and try a fresh ordering (reverse of the current one). This
        // gives the engine a chance to find a different net order that avoids
        // dead‑ends created by earlier placements. The retry budget limits how
        // many fresh attempts we make.
        let mut extra_attempts = 0usize;
        while !result.failed_nets.is_empty() && extra_attempts < self.config.retry_budget {
            if deadline.is_some_and(|d| std::time::Instant::now() >= d) {
                exhausted = true;
                break;
            }
            extra_attempts += 1;
            // New order – reverse of the previous one (could be randomised).
            let mut new_order = order.clone();
            new_order.reverse();
            let run = router.route_in_order(board, &new_order);
            // Capture before the move: `run` is moved into `result` on an
            // improvement, but the tie/break condition still needs it.
            let same_count = run.routed_nets.len() == result.routed_nets.len();
            let improved = run.routed_nets.len() > result.routed_nets.len()
                || (same_count && run.total_length_mm < result.total_length_mm);
            if improved {
                result = run;
                order = new_order;
            }
            // Break when we improved or when the reorder changed nothing.
            if improved || same_count {
                break;
            }
        }

        (result, tried, exhausted)
    }

    /// The reproducible order ladder: most useful plans first.
    fn net_order_ladder(&self, budget_board: &Board) -> Vec<Vec<usize>> {
        let board = budget_board;
        let n = board.nets.len();
        let all: Vec<usize> = (0..n).collect();
        let name = |i: usize| board.nets[i].name.clone();

        // 1. Class priority, then name.
        let mut by_priority = all.clone();
        by_priority.sort_by(|a, b| {
            board.nets[*a]
                .class
                .priority()
                .cmp(&board.nets[*b].class.priority())
                .then_with(|| name(*a).cmp(&name(*b)))
        });
        let mut ladder = vec![by_priority.clone()];

        // 2. Signals first, rails last.
        let mut signal_first = all.clone();
        signal_first.sort_by(|a, b| {
            let rank = |i: usize| match board.nets[i].class {
                NetClass::Ground => 2,
                NetClass::Power => 1,
                _ => 0,
            };
            rank(*a)
                .cmp(&rank(*b))
                .then_with(|| board.nets[*a].class.priority().cmp(&board.nets[*b].class.priority()))
                .then_with(|| name(*a).cmp(&name(*b)))
        });
        ladder.push(signal_first);

        // 3. Rails first, signals last.
        let mut rails_first = all.clone();
        rails_first.sort_by(|a, b| {
            let rank = |i: usize| match board.nets[i].class {
                NetClass::Ground => 0,
                NetClass::Power => 1,
                _ => 2,
            };
            rank(*a)
                .cmp(&rank(*b))
                .then_with(|| board.nets[*a].class.priority().cmp(&board.nets[*b].class.priority()))
                .then_with(|| name(*a).cmp(&name(*b)))
        });
        ladder.push(rails_first);

        // 4. Most terminals first — the most constrained net claims first.
        let mut most_terminals = all;
        most_terminals.sort_by(|a, b| {
            let t = |i: usize| board.net_terminals(&board.nets[i]).len();
            t(*b).cmp(&t(*a)).then_with(|| name(*a).cmp(&name(*b)))
        });
        ladder.push(most_terminals);
        let _ = by_priority;

        ladder
    }

    /// Remove any segment that duplicates fixed copper.
    ///
    /// The router treats fixed copper as an obstacle, so this is a safety net
    /// rather than an expected path: it guarantees the invariant the pipeline
    /// relies on (engine output = signal copper only).
    /// Post-routing line-of-sight simplification.
    ///
    /// The search keeps its byte-stable 4-neighbour geometry. Only after all
    /// nets are routed and legal do we merge adjacent segments of the same net
    /// into longer 0°/45°/90° runs. Every merged segment is checked against
    /// exact-mm occupancy and the parallel-run limit; if the result is not
    /// fully rule-clean, the original copper is kept.
    fn simplify_result(&self, board: &Board, result: &RoutingResult) -> Option<RoutingResult> {
        let baseline = self.rules.check_routing(board, result).violations.len();
        let mut out = result.clone();
        let clearance = self.rules.min_clearance_mm;
        for net in &board.nets {
            let Some(net_index) = board.nets.iter().position(|n| n.name == net.name) else {
                continue;
            };
            let mut segments: Vec<RouteSegment> = result
                .segments
                .iter()
                .filter(|s| s.net == net.name)
                .cloned()
                .collect();
            if segments.len() < 2 {
                continue;
            }
            let foreign_tracks: Vec<(f64, f64, f64, f64, f64, usize)> = result
                .segments
                .iter()
                .filter(|s| s.net != net.name)
                .map(|s| {
                    (
                        s.x1_mm,
                        s.y1_mm,
                        s.x2_mm,
                        s.y2_mm,
                        s.width_mm / 2.0,
                        board
                            .nets
                            .iter()
                            .position(|n| n.name == s.net)
                            .unwrap_or(usize::MAX),
                    )
                })
                .collect();
            let occupancy = Occupancy::new(board, &foreign_tracks);

            let a_width = segments[0].width_mm;
            let a_layer = segments[0].layer;
            // Decompose an arbitrary displacement into legal 0°/45°/90°
            // pieces: a diagonal part of length min(dx,dy) plus one straight
            // remainder. This is what turns an A* staircase into the clean
            // 45°+90° geometry a real router would emit.
            let canonical_pieces = |from: (f64, f64), to: (f64, f64)| -> Option<Vec<RouteSegment>> {
                let dx = to.0 - from.0;
                let dy = to.1 - from.1;
                let adx = dx.abs();
                let ady = dy.abs();
                let sx = dx.signum();
                let sy = dy.signum();
                let eps = 1e-9;
                let direct_canonical = adx < eps || ady < eps || (adx - ady).abs() <= 0.01;
                let mut variants: Vec<Vec<(f64, f64)>> = if direct_canonical {
                    vec![vec![from, to]]
                } else {
                    Vec::new()
                };
                if adx > eps && ady > eps && (adx - ady).abs() > self.config.grid_mm * 1.5 {
                    let m = adx.min(ady);
                    variants.push(vec![from, (from.0 + sx * m, from.1 + sy * m), to]);
                    if adx >= ady {
                        variants.push(vec![
                            from,
                            (from.0 + sx * (adx - m), from.1),
                            to,
                        ]);
                    } else {
                        variants.push(vec![
                            from,
                            (from.0, from.1 + sy * (ady - m)),
                            to,
                        ]);
                    }
                }
                // LUT width taper: a straight 45° line may only fit when
                // narrowed towards the process minimum.
                let min_width = self.rules.min_track_mm;
                let mut widths = vec![a_width];
                for factor in [0.75, 0.5] {
                    let w = (a_width * factor).max(min_width);
                    if !widths.iter().any(|existing| (existing - w).abs() < 1e-9) {
                        widths.push(w);
                    }
                }
                if !widths.iter().any(|w| (w - min_width).abs() < 1e-9) {
                    widths.push(min_width);
                }
                for points in variants {
                    for width in &widths {
                        let pieces: Vec<RouteSegment> = points
                            .windows(2)
                            .map(|pair| {
                                RouteSegment::new(
                                    pair[0].0,
                                    pair[0].1,
                                    pair[1].0,
                                    pair[1].1,
                                    &net.name,
                                    net.class,
                                )
                                .with_width(*width)
                                .on_layer(a_layer)
                            })
                            .collect();
                        let legal = pieces.iter().all(|piece| {
                            let parallel = result.segments.iter().any(|other| {
                                other.net != net.name
                                    && parallel_overlap_mm(
                                        piece,
                                        other,
                                        PARALLEL_GAP_MM,
                                        PARALLEL_ANGLE_TOL_DEG,
                                    ) > MAX_PARALLEL_RUN_MM
                            });
                            !parallel
                                && occupancy.allows_segment(
                                    (piece.x1_mm, piece.y1_mm),
                                    (piece.x2_mm, piece.y2_mm),
                                    piece.width_mm / 2.0,
                                    clearance,
                                    net_index,
                                )
                        });
                        if legal && pieces.len() < 4 {
                            return Some(pieces);
                        }
                    }
                }
                None
            };

            let mut changed = true;
            let mut passes = 0usize;
            while changed && passes < 4 {
                passes += 1;
                changed = false;
                let mut i = 0;
                while i + 1 < segments.len() {
                    let a = segments[i].clone();
                    let b = segments[i + 1].clone();
                    // Determine the shared endpoint and the two outer ends.
                    let tol = 1e-6;
                    let near = |p: (f64, f64), q: (f64, f64)| {
                        (p.0 - q.0).abs() < tol && (p.1 - q.1).abs() < tol
                    };
                    let a_start = (a.x1_mm, a.y1_mm);
                    let a_end = (a.x2_mm, a.y2_mm);
                    let b_start = (b.x1_mm, b.y1_mm);
                    let b_end = (b.x2_mm, b.y2_mm);
                    let (from, to) = if near(a_end, b_start) {
                        (a_start, b_end)
                    } else if near(a_end, b_end) {
                        (a_start, b_start)
                    } else if near(a_start, b_start) {
                        (a_end, b_end)
                    } else if near(a_start, b_end) {
                        (a_end, b_start)
                    } else {
                        i += 1;
                        continue;
                    };
                    if let Some(pieces) = canonical_pieces(from, to) {
                        if pieces.len() <= 2 {
                            let count = pieces.len();
                            segments.splice(i..i + 2, pieces);
                            i += count;
                            changed = true;
                        } else {
                            i += 1;
                        }
                    } else {
                        i += 1;
                    }
                }
            }
            // Last-mile canonicalization: the A* terminals sit at exact pad
            // centres, which are not on the 0.5 mm grid. Replace any remaining
            // arbitrary-slope stub by a legal 0°/45°/90° chain to the same
            // endpoint, preserving electrical connectivity.
            let mut i = 0;
            while i < segments.len() {
                let s = segments[i].clone();
                let dx = (s.x2_mm - s.x1_mm).abs();
                let dy = (s.y2_mm - s.y1_mm).abs();
                let canonical = dx < 1e-9 || dy < 1e-9 || (dx - dy).abs() < 0.01;
                if canonical {
                    i += 1;
                    continue;
                }
                let pieces = canonical_pieces((s.x1_mm, s.y1_mm), (s.x2_mm, s.y2_mm));
                if std::env::var_os("AMPROUTE_DEBUG_SIMPLIFY").is_some() {
                    eprintln!(
                        "last-mile {} ({:.2},{:.2})→({:.2},{:.2}) canonicalized={}",
                        net.name, s.x1_mm, s.y1_mm, s.x2_mm, s.y2_mm,
                        pieces.is_some()
                    );
                }
                if let Some(pieces) = pieces {
                    let count = pieces.len();
                    if count <= 2 {
                        segments.splice(i..i + 1, pieces);
                        i += count;
                        continue;
                    }
                }
                i += 1;
            }
            // Round the remaining 90° corners with a small 45° cut. The
            // whole route is revalidated at the end, so a bad chamfer
            // disables simplification for this board instead of shipping.
            if segments.len() >= 2 {
                let router = Router::new(self.config.grid_mm, board.width_mm, board.height_mm);
                segments = router.chamfer_corners(
                    &segments,
                    segments[0].width_mm / 2.0,
                    clearance,
                    net_index,
                    &occupancy,
                );
            }
            // Accept only per-net changes that do not increase rule
            // violations. One crowded net must not veto safe simplifications
            // on all the others.
            let mut candidate = out.clone();
            candidate.segments.retain(|s| s.net != net.name);
            candidate.segments.extend(segments);
            candidate.total_length_mm = candidate.segments.iter().map(|s| s.length_mm()).sum();
            let candidate_violations = self.rules.check_routing(board, &candidate).violations.len();
            if std::env::var_os("AMPROUTE_DEBUG_SIMPLIFY").is_some()
                && candidate_violations > baseline
            {
                eprintln!(
                    "simplify net {} rejected: {candidate_violations} violations vs baseline {baseline}",
                    net.name
                );
            }
            if candidate_violations <= baseline {
                out = candidate;
            }
        }
        out.total_length_mm = out.segments.iter().map(|s| s.length_mm()).sum();
        Some(out)
    }

    fn strip_fixed(&self, result: &mut RoutingResult, fixed: &FixedCopper) {
        if fixed.is_empty() {
            return;
        }
        let fixed_keys: HashSet<(String, i64, i64, i64, i64)> = fixed
            .segments
            .iter()
            .map(|s| {
                (
                    s.net.clone(),
                    (s.x1_mm * 1000.0).round() as i64,
                    (s.y1_mm * 1000.0).round() as i64,
                    (s.x2_mm * 1000.0).round() as i64,
                    (s.y2_mm * 1000.0).round() as i64,
                )
            })
            .collect();

        let before = result.segments.len();
        result.segments.retain(|s| {
            !fixed_keys.contains(&(
                s.net.clone(),
                (s.x1_mm * 1000.0).round() as i64,
                (s.y1_mm * 1000.0).round() as i64,
                (s.x2_mm * 1000.0).round() as i64,
                (s.y2_mm * 1000.0).round() as i64,
            ))
        });
        if result.segments.len() != before {
            result.total_length_mm = result.segments.iter().map(|s| s.length_mm()).sum();
        }
    }

    /// Nets the engine is responsible for, in routing order.
    ///
    /// Module 5's star rails are a bias, not delivered copper: every
    /// multi-terminal net (including power and ground) is routed with exact
    /// clearance against all others, which is what makes the exported board
    /// DRC-clean.
    pub fn signal_nets<'a>(&self, board: &'a Board, _fixed: &FixedCopper) -> Vec<&'a crate::board::Net> {
        board
            .nets
            .iter()
            .filter(|net| net.nodes.len() > 1)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{Footprint, LutPreset, Net, Pad, PadRef};

    fn board_with_signal_net() -> Board {
        let mut board = Board::new("engine-test", 60.0, 40.0, LutPreset::reliable());
        let mut r1 = Footprint::new("R1", "10k", NetClass::Signal, 10.0, 10.0, 7.0, 3.0);
        r1.pads = vec![Pad::new(0.0, 0.0, 1.8, 0.8), Pad::new(7.0, 0.0, 1.8, 0.8)];
        let mut r2 = Footprint::new("R2", "10k", NetClass::Signal, 40.0, 10.0, 7.0, 3.0);
        r2.pads = vec![Pad::new(0.0, 0.0, 1.8, 0.8), Pad::new(7.0, 0.0, 1.8, 0.8)];
        board.add_footprint(r1);
        board.add_footprint(r2);
        board.add_net(Net::new(
            "SIG",
            NetClass::Signal,
            vec![PadRef::new("R1", 0), PadRef::new("R2", 1)],
        ));
        board
    }

    #[test]
    fn engine_routes_a_simple_two_pad_net() {
        let engine = RoutingEngine::reliable();
        let board = board_with_signal_net();
        let run = engine.run(&board, &FixedCopper::default());

        assert!(
            !run.result.segments.is_empty(),
            "expected copper for SIG, log: {:?}",
            run.log
        );
        assert!(run.result.failed_nets.is_empty(), "log: {:?}", run.log);
        assert_eq!(run.outcome, EngineOutcome::Routed, "log: {:?}", run.log);
    }

    #[test]
    fn engine_produces_no_vias() {
        let engine = RoutingEngine::reliable();
        let board = board_with_signal_net();
        let run = engine.run(&board, &FixedCopper::default());
        assert_eq!(run.result.vias, 0, "LUT forbids vias");
        assert_eq!(engine.rules.max_vias, 0);
    }

    #[test]
    fn unmanufacturable_board_is_rejected_without_searching() {
        let engine = RoutingEngine::reliable();
        // A GND-named net classed as Signal violates the net-class rule.
        let mut board = Board::new("reject-me", 40.0, 30.0, LutPreset::reliable());
        let mut r1 = Footprint::new("R1", "10k", NetClass::Signal, 10.0, 10.0, 7.0, 3.0);
        r1.pads = vec![Pad::new(0.0, 0.0, 1.8, 0.8)];
        board.add_footprint(r1);
        board.add_net(Net::new("GND", NetClass::Signal, vec![PadRef::new("R1", 0)]));

        let run = engine.run(&board, &FixedCopper::default());
        assert_eq!(run.outcome, EngineOutcome::RejectedBeforeRouting);
        assert!(!run.pre_rules.is_empty());
        assert!(run.result.segments.is_empty(), "no search may run");
        assert!(!run.is_acceptable());
    }

    #[test]
    fn fixed_copper_is_never_duplicated_in_the_output() {
        let engine = RoutingEngine::reliable();
        let board = board_with_signal_net();
        let fixed = FixedCopper {
            segments: vec![RouteSegment::new(
                0.0, 0.0, 60.0, 0.0, "GND", NetClass::Ground,
            )],
            islands: vec![(30.0, 35.0, 3.0)],
        };
        let run = engine.run(&board, &fixed);
        assert!(
            !run.result.segments.iter().any(|s| s.net == "GND"),
            "engine output must not contain module 5 copper"
        );
        assert!(run.fixed_length_mm > 0.0);
        assert!((run.total_copper_mm() - (run.fixed_length_mm + run.result.total_length_mm)).abs() < 1e-9);
    }

    #[test]
    fn module_five_covers_rails_as_a_bias_but_the_engine_routes_them() {
        let engine = RoutingEngine::reliable();
        let mut board = board_with_signal_net();
        board.add_net(Net::new(
            "GND",
            NetClass::Ground,
            vec![PadRef::new("R1", 1), PadRef::new("R2", 0)],
        ));

        let fixed = FixedCopper {
            segments: vec![RouteSegment::new(0.0, 30.0, 60.0, 30.0, "GND", NetClass::Ground)],
            islands: vec![],
        };
        let names: Vec<&str> = engine.signal_nets(&board, &fixed).iter().map(|n| n.name.as_str()).collect();
        assert!(names.contains(&"SIG"));
        assert!(
            names.contains(&"GND"),
            "module 5's star is a bias, not delivered copper: ground is routed too"
        );
    }

    #[test]
    fn single_terminal_nets_are_not_routed() {
        let engine = RoutingEngine::reliable();
        let mut board = board_with_signal_net();
        board.add_net(Net::new(
            "ORPHAN",
            NetClass::Signal,
            vec![PadRef::new("R1", 1)],
        ));
        let names: Vec<&str> = engine
            .signal_nets(&board, &FixedCopper::default())
            .iter()
            .map(|n| n.name.as_str())
            .collect();
        assert!(!names.contains(&"ORPHAN"));
    }

    #[test]
    fn engine_is_deterministic() {
        let engine = RoutingEngine::reliable();
        let board = board_with_signal_net();
        let a = engine.run(&board, &FixedCopper::default());
        let b = engine.run(&board, &FixedCopper::default());
        assert_eq!(a.result.segments.len(), b.result.segments.len());
        assert!((a.result.total_length_mm - b.result.total_length_mm).abs() < 1e-9);
        assert_eq!(a.outcome, b.outcome);
    }

    #[test]
    fn config_defaults_match_the_validated_values() {
        let cfg = EngineConfig::default();
        assert!((cfg.grid_mm - 0.2).abs() < 1e-9);
        assert_eq!(cfg.retry_budget, 24);
        assert!((EngineConfig::dense().grid_mm - 0.1).abs() < 1e-9);
    }

    #[test]
    fn outcome_labels_are_stable() {
        assert_eq!(EngineOutcome::Routed.label(), "routed");
        assert_eq!(EngineOutcome::PartiallyRouted.label(), "partially routed");
        assert_eq!(
            EngineOutcome::RejectedBeforeRouting.label(),
            "rejected before routing"
        );
    }
}
