//! I/O conversion layer of the pipeline (module 2).
//!
//! [`import`] (the `.kicad_pcb` reader) belongs to this module as well; this
//! file is the format side of module 2: Specctra DSN (the session format
//! FreeRouting reads), the Specctra session format written back by
//! FreeRouting (`.ses`) and the KiCad netlist. Every function converts
//! between the board model and those text formats; nothing routes, decides
//! or edits geometry.
//!
//! DSN and SES follow the dialect of FreeRouting' `io.specctra` readers and
//! writers (its `DsnWriter`, `SesReader`, `PolylinePath` and `SesPathGeometry`).

use crate::board::Board;
use crate::sexpr::parse;
use anyhow::Result;
use std::fmt::Write as _;

// ---------------------------------------------------------------------------
// DSN export (Board -> Specctra session)
// ---------------------------------------------------------------------------

/// Export `board` to Specctra DSN: the session file FreeRouting reads.
///
/// Emits the board outline as a `boundary` rectangle and every net with the
/// component-pin atoms of its own nodes. The `placement` scope is written
/// empty — AmpRoute's [`Board`] model carries footprints without DSN
/// padstacks, so component placement only exists in the board's own files.
pub fn export_dsn(board: &Board) -> String {
    let mut s = String::with_capacity(4 * 1024);
    let _ = writeln!(s, "(pcb \"{}\")", escape(&board.name));
    let _ = writeln!(s, "  (resolution mm 0.001)");
    let _ = writeln!(s, "  (unit mm)");
    let _ = writeln!(s, "  (structure");
    let _ = writeln!(
        s,
        "    (boundary (rect \"L1\" 0 0 {:.4} {:.4}))",
        board.width_mm, board.height_mm
    );
    let _ = writeln!(s, "  )");
    let _ = writeln!(s, "  (placement)");
    let _ = writeln!(s, "  (network");
    for net in &board.nets {
        let mut pins = String::new();
        for node in &net.nodes {
            let _ = write!(pins, "{}-{} ", node.refdes, node.pad + 1);
        }
        let _ = writeln!(s, "    (net \"{}\" 0 (pins {}))", escape(&net.name), pins.trim_end());
    }
    let _ = writeln!(s, "  )");
    let _ = writeln!(s, "  (wiring)");
    let _ = writeln!(s, ")");
    s
}

/// Write `board` to `path` as Specctra DSN.
pub fn write_dsn(board: &Board, path: &str) -> Result<()> {
    std::fs::write(path, export_dsn(board))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// SES import (Specctra session -> routing data)
// ---------------------------------------------------------------------------

/// One wire (trace segment) parsed from a `.ses` session file.
#[derive(Debug, Clone, PartialEq)]
pub struct SesTrack {
    pub net: String,
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
    pub width: f64,
    pub layer: String,
}

/// One via parsed from a `.ses` session file.
#[derive(Debug, Clone, PartialEq)]
pub struct SesVia {
    pub net: String,
    pub x: f64,
    pub y: f64,
    /// Padstack name from the file (the diameter lives in the padstack, not
    /// in the session itself).
    pub padstack: String,
}

/// Routing data parsed out of a Specctra session file.
#[derive(Debug, Default)]
pub struct SesImport {
    pub session: String,
    pub tracks: Vec<SesTrack>,
    pub vias: Vec<SesVia>,
}

/// Parse the Specctra session (`.ses`) file produced by FreeRouting:
/// `(session "name" (base_design "file"))` with `(routes ... (network_out
/// (net "GND" (wire (path "F.Cu" 0.4 x1 y1 x2 y2)) (via "Via_2.5" x y))))`.
pub fn import_ses(text: &str) -> Result<SesImport> {
    let root = parse(text).map_err(|e| anyhow::anyhow!("not a valid Specctra session: {e}"))?;
    if !root.is("session") {
        anyhow::bail!("not a Specctra session file (root is not `session`)");
    }
    let mut out = SesImport {
        session: root.atom(1).unwrap_or_default().to_string(),
        ..Default::default()
    };
    for routes in root.children("routes") {
        // `(resolution UNIT DIVISOR)` — a coordinate is `UNIT / DIVISOR`.
        // FreeRouting writes `(resolution um 10)`, i.e. tenths of microns.
        let scale = routes
            .children("resolution")
            .next()
            .map(|r| resolution_mm(r.atoms()))
            .unwrap_or(1.0);

        for net_node in routes.children("network_out") {
            for net in net_node.children("net") {
                let name = net.atom(1).unwrap_or_default().to_string();
                for wire in net.children("wire") {
                    for path in wire.children("path") {
                        // `(path LAYER WIDTH x1 y1 x2 y2 ...)` — the numbers are
                        // a list of POINTS, two coordinates each; consecutive
                        // points form segments. Reading them four at a time
                        // chains the end of one segment to the start of the next
                        // and inflates total length by orders of magnitude.
                        let atoms = path.atoms();
                        let layer = atoms.get(1).copied().unwrap_or("F.Cu");
                        let width = atoms
                            .get(2)
                            .and_then(|a| a.parse::<f64>().ok())
                            .map(|w| w * scale)
                            .unwrap_or(0.0);
                        let coords: Vec<f64> = atoms
                            .iter()
                            .skip(3)
                            .filter_map(|a| a.parse::<f64>().ok())
                            .map(|v| v * scale)
                            .collect();
                        for pair in coords.as_chunks::<2>().0.windows(2) {
                            let (x1, y1) = (pair[0][0], pair[0][1]);
                            let (x2, y2) = (pair[1][0], pair[1][1]);
                            if x1 == x2 && y1 == y2 {
                                continue;
                            }
                            out.tracks.push(SesTrack {
                                net: name.clone(),
                                x1,
                                y1,
                                x2,
                                y2,
                                width,
                                layer: layer.to_string(),
                            });
                        }
                    }
                }
                for via in net.children("via") {
                    // `(via "PADSTACK" x y)` — `atoms()` includes the head atom.
                    let a = via.atoms();
                    if a.len() >= 4 {
                        out.vias.push(SesVia {
                            net: name.clone(),
                            padstack: a[1].to_string(),
                            x: a[2].parse::<f64>().unwrap_or_default() * scale,
                            y: a[3].parse::<f64>().unwrap_or_default() * scale,
                        });
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Millimetres per session coordinate unit, from `(resolution UNIT DIVISOR)`.
///
/// `(resolution um 10)` means one coordinate unit is a tenth of a micrometre,
/// i.e. 1e-4 mm: the divisor splits the unit. Verified against the shipped
/// reference session, where a `Via[0-1]_600:300_um` padstack is written as
/// `(circle F.Cu 6000 0 0)` — 6000 * 1e-4 mm = 0.6 mm, the 600 µm via.
/// An unrecognised or absent unit is treated as millimetres, so a hand-written
/// session without a resolution block still parses sensibly.
fn resolution_mm(atoms: Vec<&str>) -> f64 {
    let unit_mm = match atoms.get(1).copied().unwrap_or("mm") {
        "mm" => 1.0,
        "um" => 1e-3,
        "mm100" => 1e-2,
        "mil" => 0.0254,
        "inch" | "in" => 25.4,
        "cm" => 10.0,
        _ => 1.0,
    };
    let divisor = atoms
        .get(2)
        .and_then(|d| d.parse::<f64>().ok())
        .filter(|d| *d > 0.0)
        .unwrap_or(1.0);
    unit_mm / divisor
}

// ---------------------------------------------------------------------------
// Netlist export (Board -> KiCad netlist)
// ---------------------------------------------------------------------------

/// Export `board` to the KiCad netlist: `(netlist ... (net (code N) (name
/// "...") (nodes (node (ref "R1") (pin 1)) ...)))`.
pub fn export_netlist(board: &Board) -> String {
    let mut s = String::with_capacity(2 * 1024);
    let _ = writeln!(s, "(netlist (version 20240127))");
    for (i, net) in board.nets.iter().enumerate() {
        let _ = writeln!(s, "  (net");
        let _ = writeln!(s, "    (code {})", i + 1);
        let _ = writeln!(s, "    (name \"{}\")", escape(&net.name));
        let _ = writeln!(s, "    (nodes");
        for node in &net.nodes {
            let _ = writeln!(
                s,
                "      (node (ref \"{}\") (pin {}))",
                escape(&node.refdes),
                node.pad + 1
            );
        }
        let _ = writeln!(s, "    )");
        let _ = writeln!(s, "  )");
    }
    let _ = writeln!(s, ")");
    s
}

/// Write the KiCad netlist for `board` to `path`.
pub fn write_netlist(board: &Board, path: &str) -> Result<()> {
    std::fs::write(path, export_netlist(board))?;
    Ok(())
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boards;

    #[test]
    fn dsn_session_covers_the_board() {
        let board = boards::default_board();
        let dsn = export_dsn(&board);
        assert!(dsn.contains(&format!("(pcb \"{}\")", board.name)));
        assert!(dsn.contains("(resolution mm 0.001)"));
        assert!(dsn.contains("(unit mm)"));
        assert!(dsn.contains("(boundary"));
        assert!(dsn.contains("(network"));
        assert!(dsn.contains("(wiring)"));
        assert_eq!(dsn.matches("(net \"").count(), board.nets.len());
    }

    #[test]
    fn dsn_nets_name_their_component_pins() {
        let board = boards::default_board();
        let dsn = export_dsn(&board);
        for net in &board.nets {
            assert!(
                dsn.contains(&format!("(net \"{}\"", net.name)),
                "net `{}` missing from DSN",
                net.name
            );
            for node in &net.nodes {
                let pin = format!("{}-{}", node.refdes, node.pad + 1);
                assert!(dsn.contains(&pin), "pin {pin} of net `{}` missing", net.name);
            }
        }
    }

    #[test]
    fn parses_a_freecrouting_session() {
        let ses = r#"(session "lm386" (base_design "lm386.dsn")
  (routes
    (network_out
      (net "GND"
        (wire (path "F.Cu" 0.4 0 0 10 0))
        (wire (path "F.Cu" 0.4 10 0 10 10))
        (via "Via_2.5" 20 20)
      )
    )
  )
)"#;
        let import = import_ses(ses).unwrap();
        assert_eq!(import.session, "lm386");
        assert_eq!(import.tracks.len(), 2);
        assert_eq!(
            (import.tracks[0].x1, import.tracks[0].y1, import.tracks[0].x2, import.tracks[0].y2),
            (0.0, 0.0, 10.0, 0.0)
        );
        assert_eq!((import.tracks[0].net.as_str(), import.tracks[0].layer.as_str()), ("GND", "F.Cu"));
        assert_eq!(import.tracks[1].width, 0.4);
        assert_eq!((import.tracks[1].x1, import.tracks[1].y1), (10.0, 0.0));
        assert_eq!(import.vias.len(), 1);
        assert_eq!((import.vias[0].x, import.vias[0].y), (20.0, 20.0));
        assert_eq!(import.vias[0].padstack, "Via_2.5");
        assert_eq!(import.vias[0].net, "GND");
    }

    #[test]
    fn session_resolution_scales_coordinates_to_millimetres() {
        // `(resolution um 10)`: one coordinate unit is 0.1 µm, i.e. 1e-4 mm.
        // Anchored on the shipped reference file, where the 600 µm via is
        // written as `(circle F.Cu 6000 0 0)`: 6000 * 1e-4 mm = 0.6 mm.
        let ses = r#"(session "x" (base_design "y")
  (routes
    (resolution um 10)
    (network_out
      (net "SIG" (wire (path F.Cu 2000 100000 -50000 200000 -50000)))
    )
  )
)"#;
        let import = import_ses(ses).unwrap();
        assert_eq!(import.tracks.len(), 1);
        let t = &import.tracks[0];
        assert!((t.x1 - 10.0).abs() < 1e-9, "x1 = {}", t.x1);
        assert!((t.y1 + 5.0).abs() < 1e-9, "y1 = {}", t.y1);
        assert!((t.x2 - 20.0).abs() < 1e-9, "x2 = {}", t.x2);
        assert!((t.width - 0.2).abs() < 1e-12, "width = {}", t.width);
    }

    #[test]
    fn multi_point_path_becomes_consecutive_segments() {
        // A polyline of three points is two segments. Reading the coordinate
        // list four at a time chains the end of one segment to the start of the
        // next, which once produced a 15 000 000 mm "reference length".
        let ses = r#"(session "x" (base_design "y")
  (routes
    (resolution um 10)
    (network_out
      (net "SIG"
        (wire (path F.Cu 2000
          0 0
          100000 0
          100000 100000
        ))
      )
    )
  )
)"#;
        let import = import_ses(ses).unwrap();
        assert_eq!(import.tracks.len(), 2, "three points make two segments");
        let len: f64 = import
            .tracks
            .iter()
            .map(|t| (t.x2 - t.x1).hypot(t.y2 - t.y1))
            .sum();
        assert!((len - 20.0).abs() < 1e-9, "polyline length = {len}");
        assert!((import.tracks[1].x1 - 10.0).abs() < 1e-9);
        assert!((import.tracks[1].y1 - 0.0).abs() < 1e-9);
    }

    #[test]
    fn netlist_exports_net_nodes() {
        let board = boards::default_board();
        let netlist = export_netlist(&board);
        assert!(netlist.contains("(netlist (version"));
        for net in &board.nets {
            assert!(netlist.contains(&format!("(name \"{}\")", net.name)));
            for node in &net.nodes {
                assert!(
                    netlist.contains(&format!("(ref \"{}\") (pin {})", node.refdes, node.pad + 1))
                );
            }
        }
    }

    #[test]
    fn rejects_non_session_input() {
        assert!(import_ses("(kicad_pcb)").is_err());
        assert!(import_ses("not a session at all").is_err());
    }
}
