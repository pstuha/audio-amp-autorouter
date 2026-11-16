//! Geometry primitives for autorouting.

use pyo3::prelude::*;
use serde::{Deserialize, Serialize};

/// A 2D point in grid coordinates
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub fn new(x: i32, y: i32) -> Self {
        Point { x, y }
    }
    
    pub fn manhattan_dist(&self, other: &Point) -> i32 {
        (self.x - other.x).abs() + (self.y - other.y).abs()
    }
}

/// Rectangle in mm coordinates
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rect {
    pub x_min: f64,
    pub y_min: f64,
    pub x_max: f64,
    pub y_max: f64,
}

impl Rect {
    pub fn new(x_min: f64, y_min: f64, x_max: f64, y_max: f64) -> Self {
        Rect { x_min, y_min, x_max, y_max }
    }
    
    pub fn contains_point(&self, x: f64, y: f64) -> bool {
        x >= self.x_min && x <= self.x_max && y >= self.y_min && y <= self.y_max
    }
    
    pub fn grid_points(&self, grid_mm: f64) -> Vec<Point> {
        let mut points = Vec::new();
        let x_start = (self.x_min / grid_mm).ceil() as i32;
        let x_end = (self.x_max / grid_mm).floor() as i32;
        let y_start = (self.y_min / grid_mm).ceil() as i32;
        let y_end = (self.y_max / grid_mm).floor() as i32;
        
        for x in x_start..=x_end {
            for y in y_start..=y_end {
                points.push(Point::new(x, y));
            }
        }
        points
    }
}

/// Python-accessible geometry functions
#[pyfunction]
pub fn point_in_rect(x: f64, y: f64, rect: &PyAny) -> bool {
    // Accept rect as (x_min, y_min, x_max, y_max) tuple
    if let (Ok(x_min), Ok(y_min), Ok(x_max), Ok(y_max)) = (
        rect.get_item(0),
        rect.get_item(1),
        rect.get_item(2),
        rect.get_item(3),
    ) {
        x >= x_min && x <= x_max && y >= y_min && y <= y_max
    } else {
        false
    }
}

#[pyfunction]
pub fn grid_points_in_rect(x_min: f64, y_min: f64, x_max: f64, y_max: f64, grid_mm: f64) -> Vec<(i32, i32)> {
    let rect = Rect::new(x_min, y_min, x_max, y_max);
    rect.grid_points(grid_mm)
        .into_iter()
        .map(|p| (p.x, p.y))
        .collect()
}

/// Board geometry for routing
#[pyclass]
#[derive(Debug, Clone)]
pub struct BoardGeometry {
    pub bounds_mm: (f64, f64, f64, f64),
    pub grid_mm: f64,
}

#[pymethods]
impl BoardGeometry {
    #[new]
    fn new(bounds: (f64, f64, f64, f64), grid_mm: f64) -> Self {
        BoardGeometry {
            bounds_mm: bounds,
            grid_mm,
        }
    }
    
    fn bounds_grid(&self) -> (i32, i32, i32, i32) {
        let (minx, miny, maxx, maxy) = self.bounds_mm;
        let grid = self.grid_mm;
        (
            (minx / grid).ceil() as i32,
            (miny / grid).ceil() as i32,
            (maxx / grid).floor() as i32,
            (maxy / grid).floor() as i32,
        )
    }
    
    fn mm_to_grid(&self, x: f64, y: f64) -> (i32, i32) {
        (
            (x / self.grid_mm).round() as i32,
            (y / self.grid_mm).round() as i32,
        )
    }
    
    fn grid_to_mm(&self, x: i32, y: i32) -> (f64, f64) {
        (
            x as f64 * self.grid_mm,
            y as i32 as f64 * self.grid_mm,
        )
    }
}
