//! A* routing algorithm in Rust for maximum performance.

use crate::geometry::Point;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::collections::{HashMap, HashSet, BinaryHeap};
use std::cmp::Ordering;

#[derive(Clone)]
struct AStarNode {
    cost: f64,
    total: f64,
    point: Point,
}

impl PartialEq for AStarNode {
    fn eq(&self, other: &Self) -> bool {
        self.total == other.total && self.point == other.point
    }
}

impl Eq for AStarNode {}

impl PartialOrd for AStarNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for AStarNode {
    fn cmp(&self, other: &Self) -> Ordering {
        other.total.partial_cmp(&self.total).unwrap_or(Ordering::Equal)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetClass {
    Power,
    Ground,
    HighCurrent,
    Signal,
    Control,
    Feedback,
    Compensation,
}

impl NetClass {
    pub fn cost_weight(&self) -> f64 {
        match self {
            NetClass::Power => 3.0,
            NetClass::Ground => 3.0,
            NetClass::HighCurrent => 2.5,
            NetClass::Feedback => 2.0,
            NetClass::Compensation => 1.5,
            NetClass::Signal => 1.0,
            NetClass::Control => 1.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RouteSegment {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
    pub layer: String,
    pub net: String,
}

#[pyclass]
#[derive(Debug, Clone)]
pub struct PyRoutedBoard {
    #[pyo3(get, set)]
    pub segments: Vec<(f64, f64, f64, f64, String, String)>,
    #[pyo3(get, set)]
    pub routed_nets: Vec<String>,
    #[pyo3(get, set)]
    pub failed_nets: Vec<String>,
    #[pyo3(get, set)]
    pub total_length_mm: f64,
    #[pyo3(get, set)]
    pub score: f64,
}

#[pymethods]
impl PyRoutedBoard {
    #[new]
    fn new() -> Self {
        PyRoutedBoard {
            segments: Vec::new(),
            routed_nets: Vec::new(),
            failed_nets: Vec::new(),
            total_length_mm: 0.0,
            score: 0.0,
        }
    }
}

#[pyclass]
#[derive(Debug, Clone)]
pub struct PyRoutingOptions {
    #[pyo3(get, set)]
    pub seed: i32,
    #[pyo3(get, set)]
    pub attempts: i32,
    #[pyo3(get, set)]
    pub grid_mm: f64,
    #[pyo3(get, set)]
    pub min_clearance_mm: f64,
    #[pyo3(get, set)]
    pub min_track_mm: f64,
    #[pyo3(get, set)]
    pub power_track_mm: f64,
    #[pyo3(get, set)]
    pub high_current_track_mm: f64,
}

#[pymethods]
impl PyRoutingOptions {
    #[new]
    fn new(
        seed: i32,
        attempts: i32,
        grid_mm: f64,
        min_clearance_mm: f64,
        min_track_mm: f64,
        power_track_mm: f64,
        high_current_track_mm: f64,
    ) -> Self {
        PyRoutingOptions {
            seed,
            attempts,
            grid_mm,
            min_clearance_mm,
            min_track_mm,
            power_track_mm,
            high_current_track_mm,
        }
    }
}

fn classify_net(net_name: &str) -> NetClass {
    let lower = net_name.to_lowercase();
    if lower.contains("power") || lower.contains("vcc") || lower.contains("+") || lower.contains("15v") || lower.contains("-15v") {
        NetClass::Power
    } else if lower.contains("gnd") || lower.contains("ground") || lower == "0" {
        NetClass::Ground
    } else if lower.contains("high") || lower.contains("current") {
        NetClass::HighCurrent
    } else if lower.contains("feedback") || lower.contains("fb") {
        NetClass::Feedback
    } else if lower.contains("comp") || lower.contains("compensation") {
        NetClass::Compensation
    } else if lower.contains("ctrl") || lower.contains("control") {
        NetClass::Control
    } else {
        NetClass::Signal
    }
}

fn compress_runs(path: &[Point], grid_mm: f64) -> Vec<(f64, f64, f64, f64)> {
    if path.len() < 2 {
        return Vec::new();
    }
    let mut runs = Vec::new();
    let mut start = 0;
    let mut prev = 0;
    let mut dir = (path[1].x - path[0].x, path[1].y - path[0].y);
    
    for i in 2..path.len() {
        let new_dir = (path[i].x - path[i-1].x, path[i].y - path[i-1].y);
        if new_dir != dir {
            runs.push((
                path[start].x as f64 * grid_mm,
                path[start].y as f64 * grid_mm,
                path[prev].x as f64 * grid_mm,
                path[prev].y as f64 * grid_mm,
            ));
            start = prev;
            dir = new_dir;
        }
        prev = i - 1;
    }
    runs.push((
        path[start].x as f64 * grid_mm,
        path[start].y as f64 * grid_mm,
        path[prev].x as f64 * grid_mm,
        path[prev].y as f64 * grid_mm,
    ));
    runs
}

fn astar(
    start: Point,
    goals: &HashSet<Point>,
    blocked: &HashSet<Point>,
    bounds: (i32, i32, i32, i32),
    avoid: &HashSet<Point>,
    cost_weight: f64,
    max_nodes: usize,
) -> Option<Vec<Point>> {
    let (minx, miny, maxx, maxy) = bounds;
    let mut openq = BinaryHeap::new();
    let mut came: HashMap<Point, Point> = HashMap::new();
    let mut cost: HashMap<Point, f64> = HashMap::new();
    
    let h = |p: &Point| goals.iter()
        .map(|g| p.manhattan_dist(g) as f64)
        .fold(f64::INFINITY, f64::min);
    
    openq.push(AStarNode { cost: 0.0, total: h(&start), point: start });
    cost.insert(start, 0.0);
    
    let mut seen = 0;
    
    while let Some(node) = openq.pop() {
        seen += 1;
        if seen > max_nodes {
            break;
        }
        
        if goals.contains(&node.point) {
            let mut path = vec![node.point];
            let mut cur = node.point;
            while let Some(&prev) = came.get(&cur) {
                path.push(prev);
                cur = prev;
            }
            path.reverse();
            return Some(path);
        }
        
        if cost.get(&node.point) != Some(&node.cost) {
            continue;
        }
        
        let x = node.point.x;
        let y = node.point.y;
        
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let nx = x + dx;
            let ny = y + dy;
            let next = Point::new(nx, ny);
            
            if nx < minx || ny < miny || nx > maxx || ny > maxy {
                continue;
            }
            if blocked.contains(&next) {
                continue;
            }
            
            let step_cost = if avoid.contains(&next) { cost_weight * 4.0 } else { cost_weight };
            let new_cost = node.cost + step_cost;
            
            if new_cost < *cost.get(&next).unwrap_or(&f64::INFINITY) {
                came.insert(next, node.point);
                cost.insert(next, new_cost);
                let heuristic = h(&next);
                openq.push(AStarNode { cost: new_cost, total: new_cost + heuristic * cost_weight, point: next });
            }
        }
    }
    None
}

#[pyfunction]
pub fn route_board(
    _py: Python<'_>,
    terminals: &PyDict,
    blocked_cells: Vec<(i32, i32)>,
    bounds: (f64, f64, f64, f64),
    options: &PyRoutingOptions,
    penalty_zones: Vec<(i32, i32)>,
) -> PyResult<PyRoutedBoard> {
    let grid = options.grid_mm;
    let (b_minx, b_miny, b_maxx, b_maxy) = bounds;
    let grid_bounds = (
        (b_minx / grid).ceil() as i32 + 1,
        (b_miny / grid).ceil() as i32 + 1,
        (b_maxx / grid).floor() as i32 - 1,
        (b_maxy / grid).floor() as i32 - 1,
    );
    
    let blocked: HashSet<Point> = blocked_cells.iter().map(|(x, y)| Point::new(*x, *y)).collect();
    let avoid: HashSet<Point> = penalty_zones.iter().map(|(x, y)| Point::new(*x, *y)).collect();
    
    let mut result = PyRoutedBoard::new();
    let mut score = 0.0;
    
    let net_names: Vec<String> = terminals.keys()
        .filter_map(|k| k.extract::<String>().ok())
        .collect();
    
    let mut sorted_nets: Vec<_> = net_names.iter().map(|n| {
        let priority = match classify_net(n) {
            NetClass::Power | NetClass::Ground => 0,
            NetClass::HighCurrent => 1,
            NetClass::Feedback | NetClass::Compensation => 2,
            NetClass::Signal | NetClass::Control => 3,
        };
        (priority, n.clone())
    }).collect();
    sorted_nets.sort_by_key(|(p, _)| *p);
    
    let max_nodes = 300_000;
    
    for (_, net_name) in &sorted_nets {
        let terminals_list = if let Some(t) = terminals.get_item(net_name) {
            t.extract::<Vec<(f64, f64)>>().unwrap_or_default()
        } else {
            continue;
        };
        
        if terminals_list.len() < 2 {
            continue;
        }
        
        let net_terminals: Vec<Point> = terminals_list.iter()
            .map(|(x, y)| Point::new((x / grid).round() as i32, (y / grid).round() as i32))
            .collect();
        
        let mut connected: HashSet<Point> = HashSet::new();
        connected.insert(net_terminals[0]);
        let mut remaining: HashSet<Point> = net_terminals.iter().skip(1).cloned().collect();
        let mut net_runs = Vec::new();
        let cost_weight = classify_net(net_name).cost_weight();
        
        while let Some(&target) = remaining.iter().min_by(|a, b| {
            let da = connected.iter().map(|c| a.manhattan_dist(c)).min().unwrap_or(i32::MAX);
            let db = connected.iter().map(|c| b.manhattan_dist(c)).min().unwrap_or(i32::MAX);
            da.cmp(&db)
        }) {
            if let Some(path) = astar(target, &connected, &blocked, grid_bounds, &avoid, cost_weight, max_nodes) {
                for &p in &path { connected.insert(p); }
                remaining.remove(&target);
                net_runs.extend(compress_runs(&path, grid));
            } else {
                remaining.remove(&target);
            }
        }
        
        if remaining.is_empty() {
            result.routed_nets.push(net_name.clone());
            for (x1, y1, x2, y2) in &net_runs {
                result.segments.push((*x1, *y1, *x2, *y2, "F.Cu".to_string(), net_name.clone()));
                result.total_length_mm += ((x2 - x1).powi(2) + (y2 - y1).powi(2)).sqrt();
            }
            score += net_runs.len() as f64 * 50.0;
            score += net_runs.iter().map(|(x1, y1, x2, y2)| ((x2 - x1).powi(2) + (y2 - y1).powi(2)).sqrt()).sum::<f64>();
        } else {
            result.failed_nets.push(net_name.clone());
            score += 1000.0;
        }
    }
    
    result.score = score;
    Ok(result)
}

#[pyfunction]
pub fn route_with_simulation_feedback(
    _py: Python<'_>,
    terminals: &PyDict,
    blocked_cells: Vec<(i32, i32)>,
    bounds: (f64, f64, f64, f64),
    options: &PyRoutingOptions,
    initial_penalty_zones: Vec<(i32, i32)>,
    sim_attempts: i32,
) -> PyResult<PyRoutedBoard> {
    let mut penalty_zones = initial_penalty_zones;
    let mut best_result: Option<PyRoutedBoard> = None;
    let mut best_score = f64::INFINITY;
    
    for _ in 0..sim_attempts {
        let result = route_board(_py, terminals, blocked_cells.clone(), bounds, options, penalty_zones.clone())?;
        
        if result.score < best_score {
            best_score = result.score;
            best_result = Some(result.clone());
        }
        
        if result.routed_nets.len() > 0 && result.failed_nets.is_empty() {
            break;
        }
    }
    
    Ok(best_result.unwrap_or_else(|| {
        route_board(_py, terminals, blocked_cells, bounds, options, vec![]).unwrap_or_else(|_| PyRoutedBoard::new())
    }))
}
