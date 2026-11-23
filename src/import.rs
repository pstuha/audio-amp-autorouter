//! Import of existing boards from KiCad `.kicad_pcb` files.
//!
//! The goal is practical: open a board you drew in KiCad, see its pads and nets,
//! and autoroute the missing copper. We read through-hole pads, footprint
//! placement, net assignments, the board outline and any existing copper.
//!
//! Deliberately not a full KiCad importer. Zones, vias, arcs on inner layers,
//! teardrops and custom pad shapes are reduced or skipped; every skipped feature
//! is reported so nobody assumes the model matches the file exactly.

use crate::board::{Board, Footprint, LutPreset, MountingHole, Net, NetClass, Pad, PadRef};
use crate::sexpr::{parse, Sexpr};
use std::collections::HashMap;

/// What an import could not represent faithfully.
#[derive(Debug, Default, Clone)]
pub struct ImportReport {
    pub warnings: Vec<String>,
    pub footprints: usize,
    pub pads: usize,
    pub nets: usize,
    pub existing_tracks: usize,
    pub outline_from_edges: bool,
}

impl ImportReport {
    pub fn summary(&self) -> String {
        format!(
            "{} footprints, {} pads, {} nets, {} existing tracks{}",
            self.footprints,
            self.pads,
            self.nets,
            self.existing_tracks,
            if self.outline_from_edges {
                ", outline from Edge.Cuts"
            } else {
                ", default outline"
            }
        )
    }
}

/// A track already present in the imported file.
#[derive(Debug, Clone)]
pub struct ImportedTrack {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
    pub width: f64,
    pub layer: String,
    pub net: String,
}

/// Result of reading a KiCad board.
pub struct ImportedBoard {
    pub board: Board,
    pub report: ImportReport,
    pub tracks: Vec<ImportedTrack>,
}

/// Read a `.kicad_pcb` file into the board model.
///
/// The board takes the file's name, so an export of *your* board is called
/// after it instead of the generic default.
pub fn import_kicad_pcb(path: &str) -> anyhow::Result<ImportedBoard> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("cannot read {path}: {e}"))?;
    let name = std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "Imported board".into());
    import_kicad_pcb_str_named(&text, &name)
}

/// Read KiCad board text into the board model.
pub fn import_kicad_pcb_str(text: &str) -> anyhow::Result<ImportedBoard> {
    import_kicad_pcb_str_named(text, "Imported board")
}

/// Read KiCad board text, giving the board a name of the caller's choosing.
pub fn import_kicad_pcb_str_named(text: &str, name: &str) -> anyhow::Result<ImportedBoard> {
    let root = parse(text).map_err(|e| anyhow::anyhow!("not a valid KiCad S-expression: {e}"))?;
    if !root.is("kicad_pcb") {
        anyhow::bail!("file is not a KiCad PCB (root is not `kicad_pcb`)");
    }

    let mut report = ImportReport::default();

    // Net table. KiCad v5..9 stores `(net 3 "GND")` at the top level and pads
    // refer to nets by number. KiCad 10+ drops the table: net names live on the
    // pads themselves as `(net "GND")`. We read whatever table exists and treat
    // an unknown token as a net name, which covers both dialects.
    let mut net_codes: HashMap<String, String> = HashMap::new();
    for net in root.children("net") {
        let atoms = net.atoms();
        let (code, name) = if atoms.len() >= 3 {
            (atoms[1], atoms[2])
        } else if atoms.len() == 2 {
            (atoms[1], atoms[1])
        } else {
            continue;
        };
        if name.is_empty() || code == "0" {
            continue;
        }
        net_codes.insert(code.to_string(), name.to_string());
    }

    /// Turn the token inside a pad's `(net …)` node into a net name.
    fn resolve_net(token: &str, codes: &HashMap<String, String>) -> Option<String> {
        if token.is_empty() || token == "0" {
            return None;
        }
        Some(codes.get(token).cloned().unwrap_or_else(|| token.to_string()))
    }

    /// The token inside a `(net …)` node — a code in old files, a name in new ones.
    fn net_token(node: &Sexpr) -> Option<String> {
        node.atom(1).map(|s| s.to_string())
    }

    // Outline from Edge.Cuts. Fall back to the bounding box of all pads.
    let outline = outline_from_edge_cuts(&root);
    let mut warnings = Vec::new();

    let mut pads_world: Vec<(f64, f64, f64, f64, String)> = Vec::new();
    let mut mounting_holes: Vec<MountingHole> = Vec::new();
    let mut footprints: Vec<Footprint> = Vec::new();
    // net name -> pad references, collected while walking footprints.
    let mut net_nodes: HashMap<String, Vec<PadRef>> = HashMap::new();
    // Remember which net class each net should get, guessed from its name.
    let mut net_guess: HashMap<String, NetClass> = HashMap::new();

    for fp_node in root.children("footprint").chain(root.children("module")) {
        let lib_id = fp_node.atom(1).unwrap_or("unknown").to_string();

        // Position: `(at X Y [rot])`. KiCad's positive angle is
        // counter-clockwise on screen while its Y axis points down.
        let at = match fp_node.child("at") {
            Some(a) => a,
            None => continue,
        };
        let fx = at.number(1).unwrap_or(0.0);
        let fy = at.number(2).unwrap_or(0.0);
        let angle = at.number(3).unwrap_or(0.0);
        let rotate = |x: f64, y: f64| {
            let r = angle.to_radians();
            (x * r.cos() + y * r.sin(), -x * r.sin() + y * r.cos())
        };

        // A non-plated mounting hole is a board feature, not a component. Both
        // `MountingHole:MountingHole_3.2mm` (KiCad library) and the bare
        // `MountingHole_3.2mm` this program writes name themselves the same way.
        if lib_id.starts_with("MountingHole") {
            for pad_node in fp_node.children("pad") {
                let pad_type = pad_node.atom(2).unwrap_or("");
                // Accept both non-plated and plated mounting-hole pads.
                if pad_type != "np_thru_hole" && pad_type != "thru_hole" {
                    continue;
                }
                if let Some(drill) = pad_node.child("drill").and_then(|d| d.value()) {
                    if let Some(pad_at) = pad_node.child("at") {
                        mounting_holes.push(MountingHole::new(
                            fx + pad_at.number(1).unwrap_or(0.0),
                            fy + pad_at.number(2).unwrap_or(0.0),
                            drill,
                        ));
                    }
                }
            }
            continue;
        }

        let refdes = property_of(fp_node, "Reference").unwrap_or_else(|| "?".into());
        let value = property_of(fp_node, "Value").unwrap_or_default();

        let mut pads: Vec<Pad> = Vec::new();
        let mut pad_nets: Vec<Option<String>> = Vec::new();

        for pad_node in fp_node.children("pad") {
            // Only through-hole pads are routable on a single-layer board.
            let pad_type = pad_node.atom(2).unwrap_or("");
            if pad_type != "thru_hole" && pad_type != "np_thru_hole" {
                continue;
            }

            let pad_at = match pad_node.child("at") {
                Some(a) => a,
                None => continue,
            };
            let px = pad_at.number(1).unwrap_or(0.0);
            let py = pad_at.number(2).unwrap_or(0.0);

            // Size can be `(size 2 2)` or `(size 2 2.5)`; use the larger axis.
            let size = pad_node
                .child("size")
                .map(|s| {
                    s.number(1)
                        .unwrap_or(1.6)
                        .max(s.number(2).unwrap_or(1.6))
                })
                .unwrap_or(1.6);
            let drill = pad_node
                .child("drill")
                .and_then(|d| d.value())
                .unwrap_or(0.8);

            // A pad with no net is either mechanical or unassigned.
            let net = pad_node
                .child("net")
                .and_then(net_token)
                .and_then(|token| resolve_net(&token, &net_codes));

            let index = pads.len();
            pads.push(Pad::new(px, py, size, drill));
            let (wx, wy) = rotate(px, py);
            pads_world.push((fx + wx, fy + wy, size, drill, refdes.clone()));

            if let Some(net_name) = &net {
                net_nodes
                    .entry(net_name.clone())
                    .or_default()
                    .push(PadRef::new(&refdes, index));
                net_guess
                    .entry(net_name.clone())
                    .or_insert_with(|| guess_class(net_name));
            }
            pad_nets.push(net);
        }

        if pads.is_empty() {
            continue;
        }

        // Bounding box of the pads plus a margin stands in for the courtyard,
        // which KiCad stores separately and may not exist.
        let min_x = pads.iter().map(|p| p.x).fold(f64::MAX, f64::min);
        let max_x = pads.iter().map(|p| p.x).fold(f64::MIN, f64::max);
        let min_y = pads.iter().map(|p| p.y).fold(f64::MAX, f64::min);
        let max_y = pads.iter().map(|p| p.y).fold(f64::MIN, f64::max);
        let margin = 1.2;
        let width = (max_x - min_x) + 2.0 * margin;
        let height = (max_y - min_y) + 2.0 * margin;
        // The model measures pad offsets from the footprint's bounding-box
        // top-left; KiCad measures them from the footprint anchor.
        let (origin_x, origin_y) = (min_x - margin, min_y - margin);
        for pad in &mut pads {
            pad.x -= origin_x;
            pad.y -= origin_y;
        }

        let class = net_guess
            .values()
            .next()
            .copied()
            .unwrap_or(NetClass::Signal);

        // Rotating the pad-bounding-box centre around KiCad's anchor gives the
        // model footprint centre; local offsets are measured from the top-left.
        let (bcx, bcy) = rotate((min_x + max_x) / 2.0, (min_y + max_y) / 2.0);
        footprints.push(Footprint {
            refdes: refdes.clone(),
            value,
            class,
            x: fx + bcx - width / 2.0,
            y: fy + bcy - height / 2.0,
            width_mm: width,
            height_mm: height,
            pads,
            rotation_deg: angle,
        });
        report.pads += footprints.last().map(|f| f.pads.len()).unwrap_or(0);
    }

    // Board size: outline if present, otherwise the extent of the copper.
    let (width, height, offset_x, offset_y) = match outline {
        Some((x0, y0, x1, y1)) => {
            report.outline_from_edges = true;
            // Shift everything so the outline starts at (0, 0).
            (x1 - x0, y1 - y0, -x0, -y0)
        }
        None => {
            if pads_world.is_empty() {
                anyhow::bail!("no through-hole pads and no board outline — nothing to route");
            }
            let x0 = pads_world.iter().map(|p| p.0).fold(f64::MAX, f64::min) - 5.0;
            let y0 = pads_world.iter().map(|p| p.1).fold(f64::MAX, f64::min) - 5.0;
            let x1 = pads_world.iter().map(|p| p.0).fold(f64::MIN, f64::max) + 5.0;
            let y1 = pads_world.iter().map(|p| p.1).fold(f64::MIN, f64::max) + 5.0;
            warnings.push(
                "no Edge.Cuts outline found; board area is the pad bounding box plus 5 mm".into(),
            );
            (x1 - x0, y1 - y0, -x0, -y0)
        }
    };

    if !(width > 1.0 && height > 1.0) {
        anyhow::bail!("board outline is too small to route ({width:.1} x {height:.1} mm)");
    }

    // Apply the offset so the board origin is the outline's lower-left corner.
    for fp in &mut footprints {
        fp.x += offset_x;
        fp.y += offset_y;
    }
    for hole in &mut mounting_holes {
        hole.x += offset_x;
        hole.y += offset_y;
    }

    let mut board = Board::new(name, width, height, LutPreset::reliable());
    board.footprints = footprints;
    board.mounting_holes = mounting_holes;

    // Build nets, skipping ones with fewer than two pads (nothing to route).
    for (net_name, mut nodes) in net_nodes {
        nodes.sort_by(|a, b| a.refdes.cmp(&b.refdes).then(a.pad.cmp(&b.pad)));
        if nodes.len() < 2 {
            continue;
        }
        let class = net_guess.get(&net_name).copied().unwrap_or(NetClass::Signal);
        board.add_net(Net::new(&net_name, class, nodes));
    }
    report.nets = board.nets.len();
    report.footprints = board.footprints.len();

    // Existing copper, kept for display so the user sees what was already there.
    let mut tracks = Vec::new();
    for seg in root.children("segment") {
        let start = match seg.child("start") {
            Some(s) => s,
            None => continue,
        };
        let end = match seg.child("end") {
            Some(e) => e,
            None => continue,
        };
        let net_code = seg
            .child("net")
            .and_then(|n| n.atom(1))
            .unwrap_or("0");
        let net_name = resolve_net(net_code, &net_codes)
            .unwrap_or_else(|| format!("net{}", net_code));
        tracks.push(ImportedTrack {
            x1: start.number(1).unwrap_or(0.0) + offset_x,
            y1: start.number(2).unwrap_or(0.0) + offset_y,
            x2: end.number(1).unwrap_or(0.0) + offset_x,
            y2: end.number(2).unwrap_or(0.0) + offset_y,
            width: seg.child("width").and_then(|w| w.value()).unwrap_or(0.25),
            layer: seg.atom(2).unwrap_or("F.Cu").to_string(),
            net: net_name,
        });
    }
    report.existing_tracks = tracks.len();
    if !tracks.is_empty() {
        warnings.push(format!(
            "{} existing tracks are shown but not reused: routing starts from a blank board",
            tracks.len()
        ));
    }

    // Anything we could not model must be said out loud.
    let vias = root.children("via").count();
    if vias > 0 {
        warnings.push(format!(
            "{vias} vias ignored — this tool routes a single layer"
        ));
    }
    let zones = root.children("zone").count();
    if zones > 0 {
        warnings.push(format!(
            "{zones} copper zones ignored — no polygon pours are imported"
        ));
    }

    report.warnings = warnings;
    Ok(ImportedBoard {
        board,
        report,
        tracks,
    })
}

/// Net class guessed from a KiCad net name.
fn guess_class(name: &str) -> NetClass {
    let n = name.to_ascii_uppercase();
    if n.contains("GND") || n.contains("GROUND") || n.contains("VSS") || n == "0" {
        NetClass::Ground
    } else if n.contains("VCC") || n.contains("VDD") || n.contains("V+") || n.contains("V-") {
        NetClass::Power
    } else if n.contains("IN") && !n.contains("GND") {
        NetClass::Input
    } else if n.contains("OUT") || n.contains("SPK") {
        NetClass::Output
    } else if n.contains("FB") || n.contains("FEEDBACK") {
        NetClass::Feedback
    } else if n.contains("BIAS") {
        NetClass::Bias
    } else {
        NetClass::Signal
    }
}

/// Footprint property value, e.g. `Reference` → `R1`.
fn property_of(fp: &Sexpr, key: &str) -> Option<String> {
    for prop in fp.children("property") {
        if prop.atom(1) == Some(key) {
            return prop.atom(2).map(|s| s.to_string());
        }
    }
    None
}

/// Board outline from `Edge.Cuts` graphics, as `(x0, y0, x1, y1)`.
fn outline_from_edge_cuts(root: &Sexpr) -> Option<(f64, f64, f64, f64)> {
    let mut min_x = f64::MAX;
    let mut min_y = f64::MAX;
    let mut max_x = f64::MIN;
    let mut max_y = f64::MIN;
    let mut found = false;

    let mut absorb = |x: f64, y: f64| {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    };

    // Graphics can be lines, rectangles, circles, arcs or polygons.
    for node in root.items() {
        let on_edge = node
            .child("layer")
            .and_then(|l| l.atom(1))
            .map(|l| l == "Edge.Cuts")
            .unwrap_or(false);
        if !on_edge {
            continue;
        }
        match node.kind() {
            Some("gr_rect") => {
                if let (Some(s), Some(e)) = (node.child("start"), node.child("end")) {
                    absorb(s.number(1)?, s.number(2)?);
                    absorb(e.number(1)?, e.number(2)?);
                    found = true;
                }
            }
            Some("gr_line") => {
                if let (Some(s), Some(e)) = (node.child("start"), node.child("end")) {
                    absorb(s.number(1)?, s.number(2)?);
                    absorb(e.number(1)?, e.number(2)?);
                    found = true;
                }
            }
            Some("gr_circle") => {
                if let (Some(c), Some(e)) = (node.child("center"), node.child("end")) {
                    let cx = c.number(1)?;
                    let cy = c.number(2)?;
                    let r = ((e.number(1)? - cx).powi(2) + (e.number(2)? - cy).powi(2)).sqrt();
                    absorb(cx - r, cy - r);
                    absorb(cx + r, cy + r);
                    found = true;
                }
            }
            Some("gr_arc") => {
                if let (Some(s), Some(m), Some(e)) = (
                    node.child("start"),
                    node.child("mid"),
                    node.child("end"),
                ) {
                    for p in [s, m, e] {
                        absorb(p.number(1)?, p.number(2)?);
                    }
                    found = true;
                }
            }
            Some("gr_poly") | Some("gr_curve") => {
                for pt in node.children("pts") {
                    for xy in pt.children("xy") {
                        absorb(xy.number(1)?, xy.number(2)?);
                        found = true;
                    }
                }
            }
            _ => {}
        }
    }

    if found && max_x > min_x && max_y > min_y {
        Some((min_x, min_y, max_x, max_y))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal but real KiCad board: two resistors, one net, one track.
    const SAMPLE: &str = r#"(kicad_pcb (version 20250114) (generator "test")
  (layers (0 "F.Cu" signal) (44 "Edge.Cuts" user))
  (net 0 "")
  (net 1 "GND")
  (net 2 "IN")
  (gr_rect (start 0 0) (end 60 40) (stroke (width 0.1) (type default)) (fill none) (layer "Edge.Cuts"))
  (footprint "R"
    (layer "F.Cu") (at 10 20)
    (property "Reference" "R1" (at 0 0 0) (layer "F.SilkS"))
    (property "Value" "10k" (at 0 0 0) (layer "F.Fab"))
    (pad "1" thru_hole circle (at 0 0) (size 2 2) (drill 1) (layers "*.Cu") (net 1 "GND"))
    (pad "2" thru_hole circle (at 7.5 0) (size 2 2) (drill 1) (layers "*.Cu") (net 2 "IN"))
  )
  (footprint "R"
    (layer "F.Cu") (at 40 20)
    (property "Reference" "R2" (at 0 0 0) (layer "F.SilkS"))
    (property "Value" "10k" (at 0 0 0) (layer "F.Fab"))
    (pad "1" thru_hole circle (at 0 0) (size 2 2) (drill 1) (layers "*.Cu") (net 2 "IN"))
    (pad "2" thru_hole circle (at 7.5 0) (size 2 2) (drill 1) (layers "*.Cu") (net 1 "GND"))
  )
  (segment (start 17.5 20) (end 40 20) (width 0.4) (layer "F.Cu") (net 2))
)"#;

    #[test]
    fn imports_footprints_pads_and_nets() {
        let imported = import_kicad_pcb_str(SAMPLE).unwrap();
        assert_eq!(imported.board.footprints.len(), 2);
        assert_eq!(imported.report.pads, 4);
        // Both nets have two pads and are routable.
        assert_eq!(imported.board.nets.len(), 2);
        assert!(imported.report.outline_from_edges);
        assert!((imported.board.width_mm - 60.0).abs() < 1e-6);
        assert!((imported.board.height_mm - 40.0).abs() < 1e-6);
    }

    #[test]
    fn imports_rotated_footprints_with_pads_in_the_right_place() {
        let text = SAMPLE.replace("(at 10 20)", "(at 10 20 90)");
        let imported = import_kicad_pcb_str(&text).unwrap();
        let fp = imported
            .board
            .footprints
            .iter()
            .find(|f| f.refdes == "R1")
            .unwrap();
        assert_eq!(fp.rotation_deg, 90.0);
        // Local pad 2 at (7.5, 0) becomes (10, 12.5) after +90° KiCad rotation.
        let (x, y) = fp.pad_world(1).unwrap();
        assert!((x - 10.0).abs() < 1e-6, "x = {x}");
        assert!((y - 12.5).abs() < 1e-6, "y = {y}");
        assert!(imported
            .report
            .warnings
            .iter()
            .all(|w| !w.contains("rotation is not modelled")));
    }

    #[test]
    fn guesses_net_classes_from_names() {
        let imported = import_kicad_pcb_str(SAMPLE).unwrap();
        let gnd = imported
            .board
            .nets
            .iter()
            .find(|n| n.name == "GND")
            .unwrap();
        assert_eq!(gnd.class, NetClass::Ground);
        let input = imported.board.nets.iter().find(|n| n.name == "IN").unwrap();
        assert_eq!(input.class, NetClass::Input);
    }

    #[test]
    fn reports_existing_tracks_without_reusing_them() {
        let imported = import_kicad_pcb_str(SAMPLE).unwrap();
        assert_eq!(imported.report.existing_tracks, 1);
        assert!(imported
            .report
            .warnings
            .iter()
            .any(|w| w.contains("not reused")));
    }

    #[test]
    fn ignores_pads_belonging_to_a_single_net() {
        // A net with one pad is not routable and must be dropped.
        let single = SAMPLE.replace(r#"(net 2 "IN")"#, r#"(net 2 "IN") (net 3 "SOLO")"#)
            .replace(
                r#"(pad "2" thru_hole circle (at 7.5 0) (size 2 2) (drill 1) (layers "*.Cu") (net 1 "GND"))"#,
                r#"(pad "2" thru_hole circle (at 7.5 0) (size 2 2) (drill 1) (layers "*.Cu") (net 3 "SOLO"))"#,
            );
        let imported = import_kicad_pcb_str(&single).unwrap();
        assert!(!imported.board.nets.iter().any(|n| n.name == "SOLO"));
    }

    #[test]
    fn rejects_non_kicad_input() {
        assert!(import_kicad_pcb_str("(something else)").is_err());
        assert!(import_kicad_pcb_str("not a board at all").is_err());
    }

    #[test]
    fn warns_about_vias_and_zones() {
        let with_extras = SAMPLE.replace(
            "(segment",
            r#"(via (at 30 20) (size 0.8) (drill 0.4) (layers "F.Cu" "B.Cu") (net 1))
  (zone (net 1) (layer "F.Cu"))
  (segment"#,
        );
        let imported = import_kicad_pcb_str(&with_extras).unwrap();
        assert!(imported.report.warnings.iter().any(|w| w.contains("via")));
        assert!(imported.report.warnings.iter().any(|w| w.contains("zone")));
    }

    #[test]
    fn imported_board_routes_end_to_end() {
        let imported = import_kicad_pcb_str(SAMPLE).unwrap();
        let board = imported.board;
        assert!(board.validate().is_empty(), "{:?}", board.validate());
        let router = crate::router::Router::new(0.5, board.width_mm, board.height_mm);
        let result = router.route_board(&board);
        assert_eq!(result.routed_nets.len(), 2, "both nets should route");
    }

    #[test]
    fn a_named_import_keeps_the_name() {
        // Re-exporting your own board must not rename it to a generic default.
        let imported = import_kicad_pcb_str_named(SAMPLE, "my_preamp").unwrap();
        assert_eq!(imported.board.name, "my_preamp");

        let plain = import_kicad_pcb_str(SAMPLE).unwrap();
        assert_eq!(plain.board.name, "Imported board");
    }
}
