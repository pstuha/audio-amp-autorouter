//! Simulation functions - thermal, crosstalk, EMI analysis.

use pyo3::prelude::*;
use std::collections::HashMap;

#[pyfunction]
pub fn run_thermal(
    _py: Python<'_>,
    components: Vec<(String, f64, f64)>,  // (ref, x, y)
    tracks: Vec<(f64, f64, f64, f64, String)>,  // (x1, y1, x2, y2, net)
    power_dissipation: HashMap<String, f64>,
) -> PyResult<Vec<(String, f64, f64, f64)>> {
    // Thermal analysis: calculate junction temperatures
    let mut results = Vec::new();
    
    for (ref_des, x, y) in &components {
        let power = power_dissipation.get(ref_des).copied().unwrap_or(0.1);
        
        // Simple thermal model: Tj = Ta + P * Rth
        // Assume Rth = 40°C/W for typical TO-220, ambient 25°C
        let rth = 40.0;
        let ta = 25.0;
        let tj = ta + power * rth;
        
        results.push((ref_des.clone(), tj, power, rth));
    }
    
    Ok(results)
}

#[pyfunction]
pub fn run_crosstalk(
    _py: Python<'_>,
    tracks: Vec<(f64, f64, f64, f64, String)>,  // (x1, y1, x2, y2, net)
    min_gap_mm: f64,
) -> PyResult<Vec<(String, String, f64, bool)>> {
    // Crosstalk analysis: find parallel track pairs with insufficient spacing
    let mut results = Vec::new();
    
    for i in 0..tracks.len() {
        for j in (i + 1)..tracks.len() {
            let (x1a, y1a, x2a, y2a, neta) = &tracks[i];
            let (x1b, y1b, x2b, y2b, netb) = &tracks[j];
            
            // Skip if different nets
            if neta != netb {
                continue;
            }
            
            // Calculate gap between parallel segments
            let (gap, parallel) = calculate_track_gap(*x1a, *y1a, *x2a, *y2a, *x1b, *y1b, *x2b, *y2b);
            
            if parallel && gap < min_gap_mm {
                let crosstalk_pct = 100.0 * (1.0 - gap / min_gap_mm);
                let critical = crosstalk_pct > 20.0;
                results.push((neta.clone(), netb.clone(), crosstalk_pct, critical));
            }
        }
    }
    
    Ok(results)
}

fn calculate_track_gap(
    x1a: f64, y1a: f64, x2a: f64, y2a: f64,
    x1b: f64, y1b: f64, x2b: f64, y2b: f64,
) -> (f64, bool) {
    // Check if segments are roughly parallel and calculate gap
    let dxa = (x2a - x1a).abs();
    let dya = (y2a - y1a).abs();
    let dxb = (x2b - x1b).abs();
    let dyb = (y2b - y1b).abs();
    
    let is_horizontal_a = dya < dxa;
    let is_horizontal_b = dyb < dxb;
    
    if is_horizontal_a != is_horizontal_b {
        return (f64::MAX, false);  // Not parallel
    }
    
    // Calculate minimum distance between line segments
    let gap = if is_horizontal_a {
        // Both horizontal - use y distance
        (y1a - y1b).abs().min((y1a - y2b).abs())
    } else {
        // Both vertical - use x distance  
        (x1a - x1b).abs().min((x1a - x2b).abs())
    };
    
    (gap, true)
}

#[pyfunction]
pub fn run_full_sim(
    _py: Python<'_>,
    components: Vec<(String, f64, f64)>,
    tracks: Vec<(f64, f64, f64, f64, String)>,
    power_dissipation: HashMap<String, f64>,
) -> PyResult<HashMap<String, String>> {
    // Run all simulations and return summary
    let thermal = run_thermal(_py, components.clone(), tracks.clone(), power_dissipation.clone())?;
    let crosstalk = run_crosstalk(_py, tracks.clone(), 0.3)?;
    
    let mut summary = HashMap::new();
    
    // Thermal summary
    let max_tj = thermal.iter().map(|(_, tj, _, _)| *tj).fold(0.0, f64::max);
    summary.insert("max_junction_temp".to_string(), format!("{:.1f}C", max_tj));
    
    // Crosstalk summary
    let critical_count = crosstalk.iter().filter(|(_, _, _, c)| *c).count();
    summary.insert("crosstalk_pairs".to_string(), format!("{}", crosstalk.len()));
    summary.insert("critical_crosstalk".to_string(), format!("{}", critical_count));
    
    // Overall score
    let score = if max_tj > 100.0 || critical_count > 0 { "FAIL" } else { "PASS" };
    summary.insert("overall".to_string(), score.to_string());
    
    Ok(summary)
}
