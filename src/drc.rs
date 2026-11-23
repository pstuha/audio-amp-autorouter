//! Module 9: DRC/Audit — `kicad-cli` DRC + own geometry audit.
//!
//! - [`kicad_cli_drc`] — runs KiCad's own DRC on a `.kicad_pcb`
//!   (`kicad-cli pcb drc --format json`) and parses the report it writes.
//!   Returns `Ok(None)` when the `kicad-cli` binary is not installed — the
//!   external stage is then skipped.
//! - [`own_audit`] — audits the geometry we can check without KiCad:
//!   no vias (the LUT process is single-layer, so D01) and clearance
//!   between pads and tracks (D02, against `LutRules::from_preset`).
//!   Single-layer is architectural — this model stores only F.Cu copper —
//!   so the auditable proxy for "однослойность" is the via check.
//! - [`audit`] — merged entry point: KiCad's DRC when available + own audit.
//!
//! The final verdict is [`DrcReport::is_ok`]: accept or reject.

use crate::board::Board;
use crate::router::{RouteSegment, RoutingResult};
use crate::rules::{segment_distance_mm, LutRules};
use anyhow::Result;
use serde::Serialize;
use serde_json::Value;

const EPS: f64 = 1e-9;

/// Rule identifiers of the own audit.
pub const RULE_VIAS: &str = "D01_VIAS";
pub const RULE_CLEARANCE: &str = "D02_CLEARANCE";
/// Mounting-hole to copper clearance.
pub const RULE_HOLE_CLEARANCE: &str = "D03_HOLE_CLEARANCE";

/// One DRC violation.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DrcViolation {
    /// Rule identifier, e.g. `"D02_CLEARANCE"`.
    pub rule: String,
    /// Human-readable description of the violation.
    pub message: String,
    /// Closest copper position of the offending element (board mm), when known.
    pub at: Option<(f64, f64)>,
    /// Severity as reported by the source tool (KiCad: `error`, `warning`);
    /// `None` for the own audit and when the source doesn't specify.
    pub severity: Option<String>,
}

/// Result of a DRC/audit run — the final accept/reject verdict.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DrcReport {
    pub violations: Vec<DrcViolation>,
    /// `true` when the external `kicad-cli` DRC actually ran and produced this report.
    pub kicad_drc_ran: bool,
}

impl DrcReport {
    /// Final verdict: no violations.
    pub fn is_ok(&self) -> bool {
        self.violations.is_empty()
    }

    pub fn passed(&self) -> bool {
        self.is_ok()
    }

    pub fn count(&self, rule: &str) -> usize {
        self.violations.iter().filter(|v| v.rule == rule).count()
    }

    /// Merge two reports: violations are concatenated, duplicates (same rule
    /// and message) dropped — the own audit and `kicad-cli` may flag the same
    /// defect twice.
    pub fn merge(a: DrcReport, b: DrcReport) -> DrcReport {
        let mut violations = a.violations;
        for v in b.violations {
            if !violations.iter().any(|w| w.rule == v.rule && w.message == v.message) {
                violations.push(v);
            }
        }
        DrcReport {
            violations,
            kicad_drc_ran: a.kicad_drc_ran || b.kicad_drc_ran,
        }
    }

    pub fn summary(&self) -> String {
        format!(
            "{} violation(s){} — {}",
            self.violations.len(),
            if self.kicad_drc_ran { " [kicad-cli]" } else { "" },
            if self.is_ok() { "PASS" } else { "FAIL" }
        )
    }
}

/// Own geometry audit: no vias and clearance between distinct nets' copper.
pub fn own_audit(board: &Board, routing: Option<&RoutingResult>) -> DrcReport {
    let mut report = DrcReport::default();
    let rules = LutRules::from_preset_for(&board.preset, board.two_layer);
    let min_clearance = rules.min_clearance_mm;

    // D01: vias are forbidden on single-layer LUT boards; a two-layer board
    // gets a bounded budget and the same clearance audit as any other copper.
    if let Some(routing) = routing {
        if !board.two_layer && routing.vias > 0 {
            report.violations.push(DrcViolation {
                rule: RULE_VIAS.into(),
                message: format!(
                    "{} vias — the LUT process is single-layer, vias are forbidden",
                    routing.vias
                ),
                at: None,
                severity: None,
            });
        }
    }

    // D02: clearance between distinct nets' copper — pad-to-pad and
    // pad-to-track and track-to-track via `rules::segment_distance_mm`.
    let pads = board.all_pads();
    for i in 0..pads.len() {
        let ((ax, ay), asize, anet) = pads[i].clone();
        for pad in pads.iter().skip(i + 1) {
            let ((bx, by), bsize, bnet) = pad.clone();
            // Same conductor: nothing to check between its pads.
            if anet.is_some() && anet.as_deref() == bnet.as_deref() {
                continue;
            }
            // Edge-to-edge clearance between the two pad circles.
            let dist = (ax - bx).hypot(ay - by) - (asize + bsize) / 2.0;
            if dist + EPS < min_clearance {
                report.violations.push(DrcViolation {
                    rule: RULE_CLEARANCE.into(),
                    message: format!(
                        "pads at ({ax:.1}, {ay:.1}) and ({bx:.1}, {by:.1}): {dist:.2} mm clearance < {min_clearance:.2} mm"
                    ),
                    at: Some(((ax + bx) / 2.0, (ay + by) / 2.0)),
                    severity: None,
                });
            }
        }
    }

    if let Some(routing) = routing {
        // Pad-to-track and track-to-track.
        for ((px, py), size, net) in &pads {
            for seg in &routing.segments {
                if Some(seg.net.as_str()) == net.as_deref() {
                    continue;
                }
                let (d, pa, pb) = segment_distance_mm(
                    seg,
                    &RouteSegment::new(*px, *py, *px, *py, &seg.net, crate::board::NetClass::Signal),
                );
                let clearance = d - (seg.width_mm / 2.0 + size / 2.0);
                if clearance + EPS < min_clearance {
                    report.violations.push(DrcViolation {
                        rule: RULE_CLEARANCE.into(),
                        message: format!(
                            "pad at ({px:.1}, {py:.1}) and track: {clearance:.2} mm clearance < {min_clearance:.2} mm"
                        ),
                        at: Some(((pa.0 + pb.0) / 2.0, (pa.1 + pb.1) / 2.0)),
                        severity: None,
                    });
                }
            }
        }

        let segs = &routing.segments;
        for i in 0..segs.len() {
            for j in (i + 1)..segs.len() {
                if segs[i].net == segs[j].net || segs[i].layer != segs[j].layer {
                    continue;
                }
                let (d, pa, pb) = segment_distance_mm(&segs[i], &segs[j]);
                let clearance = d - (segs[i].width_mm / 2.0 + segs[j].width_mm / 2.0);
                if clearance + EPS < min_clearance {
                    report.violations.push(DrcViolation {
                        rule: RULE_CLEARANCE.into(),
                        message: format!(
                            "tracks: {clearance:.2} mm clearance < {min_clearance:.2} mm"
                        ),
                        at: Some(((pa.0 + pb.0) / 2.0, (pa.1 + pb.1) / 2.0)),
                        severity: None,
                    });
                }
            }
        }

        if board.two_layer {
            if routing.vias > rules.max_vias {
                report.violations.push(DrcViolation {
                    rule: RULE_VIAS.into(),
                    message: format!("{} vias exceed the {} via budget", routing.vias, rules.max_vias),
                    at: None,
                    severity: None,
                });
            }
            for via in &routing.via_objects {
                for ((px, py), size, net) in &pads {
                    if net.as_deref() == Some(via.net.as_str()) {
                        continue;
                    }
                    let clearance =
                        (via.x_mm - px).hypot(via.y_mm - py) - (via.diameter_mm + *size) / 2.0;
                    if clearance + EPS < min_clearance {
                        report.violations.push(DrcViolation {
                            rule: RULE_CLEARANCE.into(),
                            message: format!("via at ({:.1}, {:.1}) and pad at ({px:.1}, {py:.1}): {clearance:.2} mm clearance", via.x_mm, via.y_mm),
                            at: Some((via.x_mm, via.y_mm)),
                            severity: None,
                        });
                    }
                }
                for seg in &routing.segments {
                    if seg.net == via.net {
                        continue;
                    }
                    let (d, pa, pb) = segment_distance_mm(
                        seg,
                        &RouteSegment::new(via.x_mm, via.y_mm, via.x_mm, via.y_mm, &via.net, seg.class),
                    );
                    let clearance = d - (seg.width_mm + via.diameter_mm) / 2.0;
                    if clearance + EPS < min_clearance {
                        report.violations.push(DrcViolation {
                            rule: RULE_CLEARANCE.into(),
                            message: format!("via at ({:.1}, {:.1}) and track: {clearance:.2} mm clearance", via.x_mm, via.y_mm),
                            at: Some(((pa.0 + pb.0) / 2.0, (pa.1 + pb.1) / 2.0)),
                            severity: None,
                        });
                    }
                }
            }
        }
    }

    // D03: clearance between mounting holes and copper (pads and tracks).
    for hole in &board.mounting_holes {
        let r_hole = hole.drill_mm / 2.0;
        for ((px, py), size, _net) in &pads {
            let clearance = (hole.x - px).hypot(hole.y - py) - (size / 2.0 + r_hole);
            if clearance + EPS < min_clearance {
                report.violations.push(DrcViolation {
                    rule: RULE_HOLE_CLEARANCE.into(),
                    message: format!(
                        "hole at ({:.1}, {:.1}) and pad at ({px:.1}, {py:.1}): {clearance:.2} mm clearance < {min_clearance:.2} mm",
                        hole.x, hole.y
                    ),
                    at: Some(((hole.x + px) / 2.0, (hole.y + py) / 2.0)),
                    severity: None,
                });
            }
        }
        if let Some(routing) = routing {
            for seg in &routing.segments {
                let (d, pa, pb) = segment_distance_mm(
                    seg,
                    &RouteSegment::new(
                        hole.x,
                        hole.y,
                        hole.x,
                        hole.y,
                        &seg.net,
                        crate::board::NetClass::Signal,
                    ),
                );
                let clearance = d - (seg.width_mm / 2.0 + r_hole);
                if clearance + EPS < min_clearance {
                    report.violations.push(DrcViolation {
                        rule: RULE_HOLE_CLEARANCE.into(),
                        message: format!(
                            "hole at ({:.1}, {:.1}) and track: {clearance:.2} mm clearance < {min_clearance:.2} mm",
                            hole.x, hole.y
                        ),
                        at: Some(((pa.0 + pb.0) / 2.0, (pa.1 + pb.1) / 2.0)),
                        severity: None,
                    });
                }
            }
        }
    }

    report
}

/// Locate `kicad-cli`. KiCad's installer does not add it to PATH on Windows,
/// so check the standard per-user and system install locations as well.
pub fn find_kicad_cli() -> std::path::PathBuf {
    let probe = |p: &std::path::Path| p.is_file();
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        for version in ["10.0", "9.0", "8.0"] {
            candidates.push(
                std::path::Path::new(&local)
                    .join("Programs")
                    .join("KiCad")
                    .join(version)
                    .join("bin")
                    .join("kicad-cli.exe"),
            );
        }
    }
    for version in ["10.0", "9.0", "8.0"] {
        candidates.push(
            std::path::Path::new("C:/Program Files/KiCad")
                .join(version)
                .join("bin")
                .join("kicad-cli.exe"),
        );
    }
    candidates.push("/usr/bin/kicad-cli".into());
    candidates.push("/usr/local/bin/kicad-cli".into());
    candidates
        .into_iter()
        .find(|p| probe(p))
        .unwrap_or_else(|| "kicad-cli".into())
}

/// Run `kicad-cli pcb drc --format json` on `board_path` and parse the report.
///
/// Returns `Ok(None)` when the `kicad-cli` binary is not installed, or when
/// the tool exits with an error status. The report is written to a temp file
/// (KiCad's default is the board directory, which we avoid polluting) and
/// is deleted after parsing.
pub fn kicad_cli_drc(board_path: &str) -> Result<Option<DrcReport>> {
    let out = std::env::temp_dir().join(format!("amproute_drc_{}.json", std::process::id()));
    let out_str = out.to_string_lossy().to_string();
    let cli = find_kicad_cli();
    let status = match std::process::Command::new(&cli)
        .args([
            "pcb",
            "drc",
            "--severity-all",
            "--format",
            "json",
            "--exit-code-violations",
            "--output",
            &out_str,
            board_path,
        ])
        .output()
    {
        Ok(output) => output,
        // Not installed, or cannot be started: the external stage is skipped.
        Err(_not_installed) => return Ok(None),
    };
    // Exit 5 with `--exit-code-violations` means violations were found;
    // 0 means none. Other codes are tool errors.
    let ran = status.status.success() || status.status.code() == Some(5);
    if !ran {
        return Ok(None);
    }
    let violations = std::fs::read_to_string(&out)
        .ok()
        .map(|text| parse_drc_json(&text))
        .unwrap_or_default();
    std::fs::remove_file(&out).ok();
    Ok(Some(DrcReport {
        violations,
        kicad_drc_ran: true,
    }))
}

/// Final DRC pipeline entry: KiCad's DRC (when available) plus the own audit.
pub fn audit(board: &Board, routing: Option<&RoutingResult>, board_path: Option<&str>) -> DrcReport {
    let mut report = own_audit(board, routing);
    if let Some(path) = board_path {
        if let Ok(Some(kicad)) = kicad_cli_drc(path) {
            report = DrcReport::merge(report, kicad);
        }
    }
    report
}

/// Tolerant extraction of violations from the JSON report: any object that
/// carries a rule identifier and a message becomes a violation; everything
/// else is recursed into.
fn parse_drc_json(text: &str) -> Vec<DrcViolation> {
    let Ok(root) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    collect_violations(&root, &mut out);
    out
}

fn collect_violations(value: &Value, out: &mut Vec<DrcViolation>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_violations(item, out);
            }
        }
        Value::Object(map) => {
            let rule = ["rule", "code", "id", "name"]
                .iter()
                .filter_map(|k| map.get(*k))
                .find_map(|v| v.as_str());
            if let Some(rule) = rule {
                let message = ["message", "description", "text", "detail"]
                    .iter()
                    .filter_map(|k| map.get(*k))
                    .find_map(|v| v.as_str())
                    .unwrap_or_default();
                out.push(DrcViolation {
                    rule: rule.to_string(),
                    message: message.to_string(),
                    at: point_from(map),
                    severity: ["severity", "level", "type"]
                        .iter()
                        .filter_map(|k| map.get(*k))
                        .find_map(|v| v.as_str())
                        .map(str::to_string),
                });
                // Terminal: the violation object's own values are not violations.
            } else {
                for v in map.values() {
                    collect_violations(v, out);
                }
            }
        }
        _ => {}
    }
}

fn point_from(map: &serde_json::Map<String, Value>) -> Option<(f64, f64)> {
    let at = map.get("at")?;
    let x = at.get("x")?.as_f64()?;
    let y = at.get("y")?.as_f64()?;
    Some((x, y))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{Footprint, LutPreset, MountingHole, Net, NetClass, Pad, PadRef};

    fn routing(segments: Vec<RouteSegment>, vias: usize) -> RoutingResult {
        RoutingResult {
            total_length_mm: segments.iter().map(|s| s.length_mm()).sum(),
            segments,
            routed_nets: vec!["SIG".into()],
            failed_nets: Vec::new(),
            vias,
            via_objects: Vec::new(),
        }
    }

    fn two_pads_board(gap_mm: f64) -> Board {
        let mut board = Board::new("drc-test", 60.0, 40.0, LutPreset::reliable());
        board.add_footprint(
            Footprint::new("R1", "a", NetClass::Signal, 10.0, 20.0, 4.0, 4.0)
                .with_pads(vec![Pad::new(0.0, 0.0, 1.6, 0.8)]),
        );
        board.add_footprint(
            Footprint::new("R2", "b", NetClass::Signal, 10.0 + gap_mm + 2.0, 20.0, 4.0, 4.0)
                .with_pads(vec![Pad::new(0.0, 0.0, 1.6, 0.8)]),
        );
        board
    }

    #[test]
    fn own_audit_passes_a_clean_board() {
        let report = own_audit(&two_pads_board(0.0), None);
        assert!(report.is_ok(), "{}", report.summary());
        assert_eq!(report.count(RULE_CLEARANCE), 0);
        assert!(report.summary().contains("PASS"));
    }

    #[test]
    fn own_audit_flags_vias() {
        let board = two_pads_board(0.0);
        let routing = routing(Vec::new(), 2);
        let report = own_audit(&board, Some(&routing));
        assert!(!report.is_ok());
        assert_eq!(report.count(RULE_VIAS), 1);
        assert_eq!(report.violations[0].rule, RULE_VIAS);
        assert!(report.summary().contains("FAIL"));
    }

    #[test]
    fn own_audit_flags_clearance_between_pads() {
        let report = own_audit(&two_pads_board(-0.5), None);
        assert_eq!(report.count(RULE_CLEARANCE), 1);
        assert!(!report.is_ok());
        assert!(report.violations[0].at.is_some());
    }

    #[test]
    fn own_audit_flags_clearance_between_tracks() {
        let board = two_pads_board(0.0);
        let routing = routing(
            vec![
                RouteSegment::new(10.0, 10.0, 40.0, 10.0, "SIG", NetClass::Signal),
                RouteSegment::new(10.0, 10.4, 40.0, 10.4, "OTHER", NetClass::Signal),
            ],
            0,
        );
        let report = own_audit(&board, Some(&routing));
        assert_eq!(report.count(RULE_CLEARANCE), 1);
        assert!(!report.is_ok());
    }

    #[test]
    fn own_audit_ignores_same_net_pads() {
        let mut board = Board::new("drc-test", 60.0, 40.0, LutPreset::reliable());
        board.add_footprint(
            Footprint::new("R1", "a", NetClass::Signal, 10.0, 20.0, 4.0, 4.0).with_pads(vec![
                Pad::new(0.0, 0.0, 1.6, 0.8),
                Pad::new(0.6, 0.0, 1.6, 0.8),
            ]),
        );
        board.add_net(Net::new(
            "SIG",
            NetClass::Signal,
            vec![PadRef::new("R1", 0), PadRef::new("R1", 1)],
        ));
        let report = own_audit(&board, None);
        assert!(report.is_ok(), "{}", report.summary());
    }

    #[test]
    fn kicad_cli_drc_is_graceful_when_tool_is_missing() {
        // On machines without KiCad this is Ok(None); anywhere else the tool
        // runs on a missing file and reports the failure as a non-zero exit.
        assert!(kicad_cli_drc("nonexistent.kicad_pcb").is_ok());
    }

    #[test]
    fn parses_the_kicad_cli_json_report() {
        let json = r#"{
            "violations": [
                {
                    "rule": "A03_CLEARANCE",
                    "message": "conductor to conductor 0.2 mm < 0.3 mm",
                    "at": {"x": 10.5, "y": 20.25}
                },
                {"rule": "A05_ZONE_AND_VIA", "message": "zone too close to via"}
            ]
        }"#;
        let out = parse_drc_json(json);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].rule, "A03_CLEARANCE");
        assert_eq!(out[0].message, "conductor to conductor 0.2 mm < 0.3 mm");
        assert_eq!(out[0].at, Some((10.5, 20.25)));
        assert_eq!(out[1].at, None);
    }

    #[test]
    fn own_audit_flags_hole_to_pad_clearance() {
        let mut board = Board::new("drc-holes", 60.0, 40.0, LutPreset::reliable());
        board.add_mounting_hole(MountingHole::new(10.0, 20.0, 2.4));
        board.add_footprint(
            Footprint::new("R1", "a", NetClass::Signal, 10.0, 20.0, 4.0, 4.0)
                .with_pads(vec![Pad::new(0.0, 0.0, 1.6, 0.8)]),
        );
        let report = own_audit(&board, None);
        assert_eq!(report.count(RULE_HOLE_CLEARANCE), 1);
        assert!(!report.is_ok());
    }

    #[test]
    fn own_audit_flags_hole_to_track_clearance() {
        let mut board = Board::new("drc-holes", 60.0, 40.0, LutPreset::reliable());
        board.add_mounting_hole(MountingHole::new(10.0, 20.0, 3.0));
        let routing = routing(
            vec![RouteSegment::new(10.0, 20.0, 30.0, 20.0, "SIG", NetClass::Signal)],
            0,
        );
        let report = own_audit(&board, Some(&routing));
        assert_eq!(report.count(RULE_HOLE_CLEARANCE), 1);
    }

    #[test]
    fn own_audit_passes_hole_clear_of_copper() {
        let mut board = two_pads_board(0.0);
        board.add_mounting_hole(MountingHole::new(2.0, 2.0, 2.4));
        let report = own_audit(&board, None);
        assert!(report.is_ok(), "{}", report.summary());
    }

    #[test]
    fn merge_dedups_identical_violations() {
        let a = DrcReport {
            violations: vec![DrcViolation {
                rule: "D01_VIAS".into(),
                message: "m".into(),
                at: None,
                severity: None,
            }],
            kicad_drc_ran: false,
        };
        let b = DrcReport {
            violations: vec![
                DrcViolation {
                    rule: "D01_VIAS".into(),
                    message: "m".into(),
                    at: None,
                    severity: Some("error".into()),
                },
                DrcViolation {
                    rule: "D02_CLEARANCE".into(),
                    message: "other".into(),
                    at: None,
                    severity: None,
                },
            ],
            kicad_drc_ran: true,
        };
        let merged = DrcReport::merge(a, b);
        assert_eq!(merged.violations.len(), 2);
        assert!(merged.kicad_drc_ran);
    }

    #[test]
    fn severity_is_parsed_from_the_kicad_report() {
        let json = r#"{
            "violations": [
                {"rule": "A03_CLEARANCE", "message": "gap 0.2 mm", "severity": "error"}
            ]
        }"#;
        let out = parse_drc_json(json);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].severity.as_deref(), Some("error"));
    }

    #[test]
    fn kicad_cli_accepts_an_exported_builtin_board() {
        let cli = find_kicad_cli();
        if !std::path::Path::new(&cli).is_file() {
            eprintln!("skip: kicad-cli not installed at {cli:?}");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let config = crate::pipeline::PipelineConfig {
            grid_mm: 0.5,
            time_budget_s: 30.0,
            ..Default::default()
        };
        let res = crate::pipeline::run_pipeline(
            (crate::boards::BOARD_SPECS[0].build)(),
            config,
            tmp.path().to_str().unwrap(),
        )
        .unwrap();
        let path = res.kicad_board_path.expect("KiCad board written");
        let report = kicad_cli_drc(&path).unwrap().expect("kicad-cli ran");
        assert!(report.kicad_drc_ran);
        assert!(report.is_ok(), "KiCad DRC: {:?}", report.violations);
    }
}
