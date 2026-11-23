//! End-to-end checks for the quality stages: AC, EMI and transient thermal
//! reports must reach the pipeline result and the machine-readable contract.

use amproute_studio::ac::AcConfig;
use amproute_studio::emi::EmiConfig;
use amproute_studio::pipeline::{run_pipeline, PipelineConfig};

#[test]
fn ac_emi_and_transient_reports_reach_pipeline_result() {
    let tmp = tempfile::tempdir().unwrap();
    let config = PipelineConfig {
        grid_mm: 0.5,
        time_budget_s: 60.0,
        ac: Some(AcConfig::default()),
        emi: Some(EmiConfig::default()),
        include_transient: true,
        ..Default::default()
    };
    let result = run_pipeline(
        amproute_studio::build_any("lm386-mini").unwrap(),
        config,
        tmp.path().to_str().unwrap(),
    )
    .unwrap();

    let ac = result.ac.as_ref().expect("AC report");
    assert_eq!(ac.model, "proxy");
    assert!(ac.points.len() >= 2);
    assert!(ac.dc_gain_db.is_finite());

    let emi = result.emi.as_ref().expect("EMI report");
    assert_eq!(emi.model, "proxy");
    assert!(!emi.assumptions.is_empty());

    let thermal = result.transient.as_ref().expect("transient report");
    assert!(thermal.time_constant_s > 0.0);
    assert!(thermal.peak_junction_c > 25.0);
    assert_eq!(thermal.model, "proxy");

    let json = serde_json::to_string(&result).unwrap();
    for field in ["\"ac\":", "\"emi\":", "\"transient\":", "\"quality\":"] {
        assert!(json.contains(field), "JSON missing {field}");
    }
    assert!(result.quality.assumptions.iter().any(|a| a.contains("proxy")));
}
