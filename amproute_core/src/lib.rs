//! AmpRoute Core - High-performance autorouter core in Rust
//! 
//! Exposes A* routing and simulation functions to Python via PyO3.

use pyo3::prelude::*;

pub mod geometry;
pub mod routing;
pub mod simulations;

use routing::{PyRoutedBoard, PyRoutingOptions, route_board, route_with_simulation_feedback};
use simulations::{run_thermal, run_crosstalk, run_full_sim};

#[pymodule]
fn amproute_core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(route_board, m))?;
    m.add_function(wrap_pyfunction!(route_with_simulation_feedback, m))?;
    m.add_function(wrap_pyfunction!(run_thermal, m))?;
    m.add_function(wrap_pyfunction!(run_crosstalk, m))?;
    m.add_function(wrap_pyfunction!(run_full_sim, m))?;
    m.add_class::<PyRoutedBoard>()?;
    m.add_class::<PyRoutingOptions>()?;
    Ok(())
}
