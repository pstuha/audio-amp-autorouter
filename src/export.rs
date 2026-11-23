//! Export writers: Gerber (RS-274X), SVG and PDF.

use crate::board::{Board, NetClass};
use crate::router::{CopperLayer, RouteSegment, RoutingResult};
use anyhow::Result;
use printpdf::path::{PaintMode, WindingOrder};
use printpdf::*;
use std::fs::File;
use std::io::{BufWriter, Write};

fn gerber_units(mm: f64) -> i64 {
    (mm * 1_000_000.0).round() as i64
}

/// Write one copper layer as Gerber RS-274X.
fn write_gerber_layer(board: &Board, result: &RoutingResult, path: &str, layer: CopperLayer) -> Result<()> {
    let mut f = BufWriter::new(File::create(path)?);
    writeln!(f, "G04 AmpRoute Studio copper*")?;
    writeln!(f, "%FSLAX46Y46*%")?;
    writeln!(f, "%MOMM*%")?;
    writeln!(f, "%LPD*%")?;

    // Apertures: one per distinct track width, plus one per distinct pad diameter.
    let mut track_apertures: Vec<(f64, u32)> = Vec::new();
    for class in NetClass::ALL {
        let w = class.track_width_mm();
        if !track_apertures.iter().any(|(aw, _)| (aw - w).abs() < 1e-9) {
            track_apertures.push((w, (track_apertures.len() + 10) as u32));
        }
    }

    let mut pad_apertures: Vec<(f64, u32)> = Vec::new();
    for (_pos, size, _net) in board.all_pads() {
        if !pad_apertures
            .iter()
            .any(|(asize, _)| (asize - size).abs() < 1e-9)
        {
            pad_apertures.push((
                size,
                (track_apertures.len() + pad_apertures.len() + 10) as u32,
            ));
        }
    }
    // Vias are flashed on both copper layers with their own aperture.
    for via in &result.via_objects {
        if !pad_apertures
            .iter()
            .any(|(asize, _)| (asize - via.diameter_mm).abs() < 1e-9)
        {
            pad_apertures.push((
                via.diameter_mm,
                (track_apertures.len() + pad_apertures.len() + 10) as u32,
            ));
        }
    }

    for (w, d) in &track_apertures {
        writeln!(f, "%ADD{}C,{:.3}*%", d, w)?;
    }
    for (size, d) in &pad_apertures {
        writeln!(f, "%ADD{}C,{:.3}*%", d, size)?;
    }

    writeln!(f, "G04 Pads*")?;
    for ((x, y), size, _net) in board.all_pads() {
        let code = pad_apertures
            .iter()
            .find(|(asize, _)| (asize - size).abs() < 1e-9)
            .map(|(_, d)| *d)
            .unwrap_or(track_apertures[0].1);
        writeln!(f, "D{}*", code)?;
        writeln!(f, "X{}Y{}D03*", gerber_units(x), gerber_units(y))?;
    }

    writeln!(f, "G04 Tracks ({})*", layer.kicad_name())?;
    for seg in result.segments.iter().filter(|s| s.layer == layer) {
        let w = seg.width_mm;
        let d = track_apertures
            .iter()
            .find(|(aw, _)| (aw - w).abs() < 1e-9)
            .map(|(_, d)| *d)
            .unwrap_or(track_apertures[0].1);
        writeln!(f, "D{}*", d)?;
        writeln!(
            f,
            "X{}Y{}D02*",
            gerber_units(seg.x1_mm),
            gerber_units(seg.y1_mm)
        )?;
        writeln!(
            f,
            "X{}Y{}D01*",
            gerber_units(seg.x2_mm),
            gerber_units(seg.y2_mm)
        )?;
    }

    if !result.via_objects.is_empty() {
        writeln!(f, "G04 Vias ({})*", layer.kicad_name())?;
        for via in &result.via_objects {
            let code = pad_apertures
                .iter()
                .find(|(asize, _)| (asize - via.diameter_mm).abs() < 1e-9)
                .map(|(_, d)| *d)
                .unwrap_or(track_apertures[0].1);
            writeln!(f, "D{}*", code)?;
            writeln!(
                f,
                "X{}Y{}D03*",
                gerber_units(via.x_mm),
                gerber_units(via.y_mm)
            )?;
        }
    }

    writeln!(f, "M02*")?;
    f.flush()?;
    Ok(())
}

/// Front copper — `board.gbr`, the LUT stencil for a single-layer board.
pub fn write_gerber(board: &Board, result: &RoutingResult, path: &str) -> Result<()> {
    write_gerber_layer(board, result, path, CopperLayer::FCu)
}

/// Back copper, emitted only when a board actually uses two layers.
pub fn write_gerber_back(board: &Board, result: &RoutingResult, path: &str) -> Result<()> {
    write_gerber_layer(board, result, path, CopperLayer::BCu)
}

pub fn write_svg(board: &Board, result: &RoutingResult, path: &str) -> Result<()> {
    const S: f64 = 2.0;
    let w = board.width_mm * S;
    let h = board.height_mm * S;
    let mut f = BufWriter::new(File::create(path)?);

    writeln!(f, r#"<?xml version="1.0" encoding="UTF-8"?>"#)?;
    writeln!(
        f,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">"#
    )?;
    writeln!(f, r##"<rect width="{w}" height="{h}" fill="#0d2818"/>"##)?;

    writeln!(f, r##"<g stroke="#1d4a2e" stroke-width="0.5">"##)?;
    let mut x = 0.0;
    while x <= board.width_mm {
        writeln!(
            f,
            r#"<line x1="{}" y1="0" x2="{}" y2="{}"/>"#,
            x * S,
            x * S,
            h
        )?;
        x += 10.0;
    }
    let mut y = 0.0;
    while y <= board.height_mm {
        writeln!(
            f,
            r#"<line x1="0" y1="{}" x2="{}" y2="{}"/>"#,
            y * S,
            w,
            y * S
        )?;
        y += 10.0;
    }
    writeln!(f, "</g>")?;

    writeln!(
        f,
        r##"<rect x="0" y="0" width="{}" height="{}" fill="none" stroke="#5fe08a" stroke-width="2"/>"##,
        w, h
    )?;

    writeln!(f, r##"<g fill="none" stroke="#7fbf9a" stroke-width="1">"##)?;
    for fp in &board.footprints {
        writeln!(
            f,
            r#"<rect x="{}" y="{}" width="{}" height="{}" rx="2"/>"#,
            fp.x * S,
            fp.y * S,
            fp.width_mm * S,
            fp.height_mm * S
        )?;
    }
    writeln!(f, "</g>")?;

    writeln!(f, r#"<g stroke-linecap="round" stroke-linejoin="round" fill="none">"#)?;
    for seg in &result.segments {
        let (r, g, b) = seg.class.rgb();
        writeln!(
            f,
            r##"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="#{:02x}{:02x}{:02x}" stroke-width="{}"/>"##,
            seg.x1_mm * S,
            seg.y1_mm * S,
            seg.x2_mm * S,
            seg.y2_mm * S,
            r,
            g,
            b,
            (seg.width_mm * S).max(1.0)
        )?;
    }
    writeln!(f, "</g>")?;

    writeln!(
        f,
        r##"<g fill="#c8a13a" stroke="#f0d47a" stroke-width="0.8">"##
    )?;
    for (pos, size, _net) in board.all_pads() {
        writeln!(
            f,
            r#"<circle cx="{}" cy="{}" r="{}"/>"#,
            pos.0 * S,
            pos.1 * S,
            (size / 2.0 * S).max(1.5)
        )?;
    }
    writeln!(f, "</g>")?;

    if !result.via_objects.is_empty() {
        writeln!(f, r##"<g stroke="#e8eef4" stroke-width="0.8">"##)?;
        for via in &result.via_objects {
            let (r, g, b) = via.class.rgb();
            writeln!(
                f,
                r##"<circle cx="{}" cy="{}" r="{}" fill="#{:02x}{:02x}{:02x}"/>"##,
                via.x_mm * S,
                via.y_mm * S,
                via.diameter_mm / 2.0 * S,
                r,
                g,
                b
            )?;
            writeln!(
                f,
                r##"<circle cx="{}" cy="{}" r="{}" fill="#0a1224"/>"##,
                via.x_mm * S,
                via.y_mm * S,
                via.drill_mm / 2.0 * S
            )?;
        }
        writeln!(f, "</g>")?;
    }

    writeln!(
        f,
        r##"<g font-family="monospace" font-size="3" fill="#d8f5c8">"##
    )?;
    writeln!(
        f,
        r#"<text x="4" y="5">AmpRoute Studio — {} — {}x{}mm</text>"#,
        board.name, board.width_mm, board.height_mm
    )?;
    writeln!(
        f,
        r#"<text x="4" y="10">routed {}/{} nets · {:.1}mm track</text>"#,
        result.routed_nets.len(),
        result.routed_nets.len() + result.failed_nets.len(),
        result.total_length_mm
    )?;
    writeln!(f, "</g>")?;
    writeln!(f, "</svg>")?;
    f.flush()?;
    Ok(())
}

pub fn write_pdf(board: &Board, result: &RoutingResult, path: &str) -> Result<()> {
    let (doc, page1, layer1) = PdfDocument::new(
        "AmpRoute Studio",
        Mm(board.width_mm as f32 + 20.0),
        Mm(board.height_mm as f32 + 32.0),
        "Layer 1",
    );
    let font = doc.add_builtin_font(BuiltinFont::Helvetica)?;
    let bold = doc.add_builtin_font(BuiltinFont::HelveticaBold)?;
    let layer = doc.get_page(page1).get_layer(layer1);

    layer.use_text(
        format!("AmpRoute Studio - {}", board.name),
        16.0,
        Mm(10.0),
        Mm(board.height_mm as f32 + 22.0),
        &bold,
    );
    layer.use_text(
        format!(
            "Board {}x{}mm - LUT preset {} - routed {}/{} nets - {:.1}mm total track",
            board.width_mm,
            board.height_mm,
            board.preset.name,
            result.routed_nets.len(),
            result.routed_nets.len() + result.failed_nets.len(),
            result.total_length_mm
        ),
        9.0,
        Mm(10.0),
        Mm(board.height_mm as f32 + 14.0),
        &font,
    );

    // Board outline.
    layer.set_outline_color(Color::Rgb(Rgb::new(0.35, 0.85, 0.55, None)));
    layer.set_outline_thickness(1.5);
    let outline = Line::from_iter(vec![
        (Point::new(Mm(0.0), Mm(0.0)), false),
        (Point::new(Mm(board.width_mm as f32), Mm(0.0)), false),
        (
            Point::new(Mm(board.width_mm as f32), Mm(board.height_mm as f32)),
            false,
        ),
        (Point::new(Mm(0.0), Mm(board.height_mm as f32)), false),
        (Point::new(Mm(0.0), Mm(0.0)), false),
    ]);
    layer.add_line(outline);

    // Tracks (coloured per net class).
    for seg in &result.segments {
        let (r, g, b) = seg.class.rgb();
        layer.set_outline_color(Color::Rgb(Rgb::new(
            r as f32 / 255.0,
            g as f32 / 255.0,
            b as f32 / 255.0,
            None,
        )));
        layer.set_outline_thickness(seg.width_mm as f32 * 2.83);
        let line = Line::from_iter(vec![
            (
                Point::new(Mm(seg.x1_mm as f32), Mm(seg.y1_mm as f32)),
                false,
            ),
            (
                Point::new(Mm(seg.x2_mm as f32), Mm(seg.y2_mm as f32)),
                false,
            ),
        ]);
        layer.add_line(line);
    }

    // Pads (filled gold).
    layer.set_fill_color(Color::Rgb(Rgb::new(0.78, 0.63, 0.23, None)));
    layer.set_outline_color(Color::Rgb(Rgb::new(0.95, 0.86, 0.48, None)));
    layer.set_outline_thickness(0.6);
    for ((x, y), size, _net) in board.all_pads() {
        let r = size as f32 / 2.0;
        let mut pts = Vec::with_capacity(13);
        for i in 0..12 {
            let a = std::f32::consts::TAU * i as f32 / 12.0;
            pts.push((
                Point::new(Mm(x as f32 + r * a.cos()), Mm(y as f32 + r * a.sin())),
                false,
            ));
        }
        pts.push((Point::new(Mm(x as f32 + r), Mm(y as f32)), false));
        let poly = Polygon {
            rings: vec![pts],
            mode: PaintMode::Fill,
            winding_order: WindingOrder::NonZero,
        };
        layer.add_polygon(poly);
    }

    let mut buf = BufWriter::new(File::create(path)?);
    doc.save(&mut buf)?;
    buf.flush()?;
    Ok(())
}

/// Export all formats at once.
///
/// Refuses to write copper if the board fails structural validation — this is
/// the safety gate that prevents shipping a short-circuit layout to the LUT
/// transfer.
pub fn write_all(board: &Board, result: &RoutingResult, dir: &str) -> Result<Vec<String>> {
    let issues = board.validate();
    if !issues.is_empty() {
        anyhow::bail!("board failed validation:\n  {}", issues.join("\n  "));
    }
    if !result.failed_nets.is_empty() {
        anyhow::bail!(
            "routing failed for {} net(s):\n  {}",
            result.failed_nets.len(),
            result.failed_nets.join("\n  ")
        );
    }
    std::fs::create_dir_all(dir)?;
    let mut written = Vec::new();
    write_gerber(board, result, &format!("{}/board.gbr", dir))?;
    written.push(format!("{}/board.gbr", dir));
    if result.segments.iter().any(|s| s.layer == CopperLayer::BCu) {
        write_gerber_back(board, result, &format!("{}/board_b.gbr", dir))?;
        written.push(format!("{}/board_b.gbr", dir));
    }
    if !result.via_objects.is_empty() {
        write_excellon(board, result, &format!("{}/vias.drl", dir))?;
        written.push(format!("{}/vias.drl", dir));
    }
    write_svg(board, result, &format!("{}/board.svg", dir))?;
    written.push(format!("{}/board.svg", dir));
    write_pdf(board, result, &format!("{}/board.pdf", dir))?;
    written.push(format!("{}/board.pdf", dir));
    write_lut_template(board, result, &format!("{}/lut_print.pdf", dir))?;
    written.push(format!("{}/lut_print.pdf", dir));
    if result.segments.iter().any(|s| s.layer == CopperLayer::BCu) {
        write_lut_template_back(board, result, &format!("{}/lut_print_b.pdf", dir))?;
        written.push(format!("{}/lut_print_b.pdf", dir));
    }
    Ok(written)
}

/// A 1:1 scale print template for laser-iron / toner-transfer (LUT) production.
///
/// The page is sized to the board plus a 10 mm border; copper and pads are
/// drawn at true scale so the sheet can be printed 100% (no "fit to page") and
/// ironed directly. Mirrors nothing — use the SVG preview to flip if needed.
/// Minimal ODB++-style net/topology profile for downstream CAM tools.
pub fn write_odb_profile(board: &Board, result: &RoutingResult, path: &str) -> Result<()> {
    use std::path::Path;
    let root = Path::new(path);
    let step = root.join("steps").join("pcb");
    for dir in [
        root.join("matrix"),
        step.join("netlists"),
        step.join("layers").join("top"),
        step.join("layers").join("bot"),
    ] {
        std::fs::create_dir_all(dir)?;
    }

    let mut matrix = BufWriter::new(File::create(root.join("matrix").join("matrix"))?);
    writeln!(matrix, "ODB++ 7.0")?;
    writeln!(matrix, "UNITS=MM")?;
    writeln!(matrix, "STEP=pcb")?;
    writeln!(matrix, "LAYER=top TYPE=SIGNAL NAME=F.Cu")?;
    writeln!(matrix, "LAYER=bot TYPE=SIGNAL NAME=B.Cu")?;
    writeln!(matrix, "LAYER=profile TYPE=PROFILE NAME=Edge.Cuts")?;
    matrix.flush()?;

    let mut hdr = BufWriter::new(File::create(step.join("stephdr"))?);
    writeln!(hdr, "STEP=pcb")?;
    writeln!(hdr, "UNITS=MM")?;
    writeln!(hdr, "SIZE={:.4} {:.4}", board.width_mm, board.height_mm)?;
    hdr.flush()?;

    let mut netlist = BufWriter::new(File::create(step.join("netlists").join("netlist"))?);
    for net in &board.nets {
        writeln!(netlist, "NET={} CLASS={}", net.name, net.class.label())?;
        for node in &net.nodes {
            writeln!(
                netlist,
                "NODE={} PAD={}",
                node.refdes,
                node.pad + 1
            )?;
        }
    }
    netlist.flush()?;

    for (layer, name) in [(CopperLayer::FCu, "top"), (CopperLayer::BCu, "bot")] {
        let mut features =
            BufWriter::new(File::create(step.join("layers").join(name).join("features"))?);
        for seg in result.segments.iter().filter(|s| s.layer == layer) {
            writeln!(
                features,
                "L {:.4} {:.4} {:.4} {:.4} {:.4} 0",
                seg.x1_mm,
                seg.y1_mm,
                seg.x2_mm,
                seg.y2_mm,
                seg.width_mm
            )?;
        }
        features.flush()?;
    }

    let mut drill = BufWriter::new(File::create(step.join("drill"))?);
    for via in &result.via_objects {
        writeln!(
            drill,
            "VIA {:.4} {:.4} {:.4} 1",
            via.x_mm, via.y_mm, via.drill_mm
        )?;
    }
    drill.flush()?;
    Ok(())
}

/// IPC-2581 Rev B style XML: content, dictionary, layer stack, netlist and
/// routed geometry. Coordinates are millimetres, as in the rest of the tool.
pub fn write_ipc2581_profile(board: &Board, result: &RoutingResult, path: &str) -> Result<()> {
    let mut f = BufWriter::new(File::create(path)?);
    writeln!(f, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>")?;
    writeln!(
        f,
        "<CadData xmlns=\"http://ipc.org/ipc2581/RevB\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:noNamespaceSchemaLocation=\"ipc2581.xsd\">"
    )?;
    writeln!(f, "  <Content units=\"mm\" schemaVersion=\"B\">")?;
    writeln!(f, "    <PartNumber>{}</PartNumber>", board.name)?;
    writeln!(f, "    <Title>AmpRoute Studio routed board</Title>")?;
    writeln!(f, "  </Content>")?;
    writeln!(f, "  <Dictionary>")?;
    writeln!(
        f,
        "    <Units>mm</Units><LayerDef name=\"F.Cu\" layerType=\"CONDUCTOR\" layerNum=\"1\"/>"
    )?;
    writeln!(
        f,
        "    <LayerDef name=\"B.Cu\" layerType=\"CONDUCTOR\" layerNum=\"2\"/>"
    )?;
    writeln!(
        f,
        "    <LayerDef name=\"Profile\" layerType=\"PROFILE\" layerNum=\"0\"/>"
    )?;
    writeln!(f, "  </Dictionary>")?;
    writeln!(
        f,
        "  <DesignRules><Clearance value=\"{:.4}\"/><MinTrack value=\"{:.4}\"/></DesignRules>",
        board.preset.min_clearance_mm, board.preset.min_track_mm
    )?;
    writeln!(
        f,
        "  <Profile><Outline x0=\"0\" y0=\"0\" x1=\"{:.4}\" y1=\"{:.4}\"/></Profile>",
        board.width_mm, board.height_mm
    )?;
    writeln!(f, "  <Netlist>")?;
    for net in &board.nets {
        writeln!(
            f,
            "    <Net name=\"{}\" netClass=\"{}\">",
            net.name,
            net.class.label()
        )?;
        for seg in result.segments.iter().filter(|s| s.net == net.name) {
            writeln!(
                f,
                "      <Track layer=\"{}\" x1=\"{:.4}\" y1=\"{:.4}\" x2=\"{:.4}\" y2=\"{:.4}\" width=\"{:.4}\"/>",
                seg.layer.kicad_name(),
                seg.x1_mm,
                seg.y1_mm,
                seg.x2_mm,
                seg.y2_mm,
                seg.width_mm
            )?;
        }
        writeln!(f, "    </Net>")?;
    }
    writeln!(f, "  </Netlist>")?;
    if !result.via_objects.is_empty() {
        writeln!(f, "  <Drill>")?;
        for via in &result.via_objects {
            writeln!(
                f,
                "    <Hole x=\"{:.4}\" y=\"{:.4}\" drill=\"{:.4}\" plated=\"true\"/>",
                via.x_mm, via.y_mm, via.drill_mm
            )?;
        }
        writeln!(f, "  </Drill>")?;
    }
    writeln!(f, "</CadData>")?;
    f.flush()?;
    Ok(())
}

/// Excellon drill file for plated vias (no mechanical holes — those are in the board outline).
pub fn write_excellon(board: &Board, result: &RoutingResult, path: &str) -> Result<()> {
    let mut f = BufWriter::new(File::create(path)?);
    writeln!(f, "M48")?;
    writeln!(f, "METRIC")?;
    let drills: Vec<f64> = {
        let mut d = Vec::new();
        for via in &result.via_objects {
            if !d.iter().any(|v| (v - via.drill_mm).abs() < 1e-6) {
                d.push(via.drill_mm);
            }
        }
        d.sort_by(|a, b| a.partial_cmp(b).unwrap());
        d
    };
    for (i, drill) in drills.iter().enumerate() {
        writeln!(f, "T{:02}C{:.3}", i + 1, drill)?;
    }
    writeln!(f, "%")?;
    for via in &result.via_objects {
        let tool = drills.iter().position(|d| (d - via.drill_mm).abs() < 1e-6).unwrap() + 1;
        writeln!(f, "T{:02}", tool)?;
        writeln!(f, "X{:06.3}Y{:06.3}", via.x_mm, via.y_mm)?;
    }
    writeln!(f, "M30")?;
    let _ = board;
    f.flush()?;
    Ok(())
}

/// Mirrored 1:1 template for the back copper layer.
pub fn write_lut_template_back(board: &Board, result: &RoutingResult, path: &str) -> Result<()> {
    let mut mirrored_board = board.clone();
    for fp in &mut mirrored_board.footprints {
        fp.x = board.width_mm - fp.x - fp.width_mm;
        for pad in &mut fp.pads {
            pad.x = fp.width_mm - pad.x;
        }
    }
    let mirrored = RoutingResult {
        segments: result
            .segments
            .iter()
            .filter(|s| s.layer == CopperLayer::BCu)
            .map(|s| RouteSegment {
                x1_mm: board.width_mm - s.x1_mm,
                y1_mm: s.y1_mm,
                x2_mm: board.width_mm - s.x2_mm,
                y2_mm: s.y2_mm,
                ..s.clone()
            })
            .collect(),
        ..RoutingResult::new()
    };
    write_lut_template(&mirrored_board, &mirrored, path)
}

pub fn write_lut_template(board: &Board, result: &RoutingResult, path: &str) -> Result<()> {
    let border = 10.0_f64;
    let (doc, page1, layer1) = PdfDocument::new(
        "AmpRoute Studio — LUT print (1:1)",
        Mm((board.width_mm + 2.0 * border) as f32),
        Mm((board.height_mm + 2.0 * border) as f32),
        "copper",
    );
    let font = doc.add_builtin_font(BuiltinFont::HelveticaBold)?;
    let layer = doc.get_page(page1).get_layer(layer1);

    // Shift everything into the border.
    let ox = border;
    let oy = border;

    // Board outline (thick, so the edge of the sheet is unambiguous).
    layer.set_outline_color(Color::Rgb(Rgb::new(0.1, 0.1, 0.1, None)));
    layer.set_outline_thickness(1.2);
    layer.add_line(Line::from_iter(vec![
        (Point::new(Mm(ox as f32), Mm(oy as f32)), false),
        (
            Point::new(Mm((ox + board.width_mm) as f32), Mm(oy as f32)),
            false,
        ),
        (
            Point::new(
                Mm((ox + board.width_mm) as f32),
                Mm((oy + board.height_mm) as f32),
            ),
            false,
        ),
        (
            Point::new(Mm(ox as f32), Mm((oy + board.height_mm) as f32)),
            false,
        ),
        (Point::new(Mm(ox as f32), Mm(oy as f32)), false),
    ]));

    // Tracks, solid black at true width (copper is opaque on the template).
    layer.set_outline_color(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None)));
    for seg in &result.segments {
        layer.set_outline_thickness(seg.width_mm as f32);
        layer.add_line(Line::from_iter(vec![
            (
                Point::new(Mm((ox + seg.x1_mm) as f32), Mm((oy + seg.y1_mm) as f32)),
                false,
            ),
            (
                Point::new(Mm((ox + seg.x2_mm) as f32), Mm((oy + seg.y2_mm) as f32)),
                false,
            ),
        ]));
    }

    // Pads, filled black.
    layer.set_fill_color(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None)));
    layer.set_outline_color(Color::Rgb(Rgb::new(0.0, 0.0, 0.0, None)));
    layer.set_outline_thickness(0.1);
    for ((x, y), size, _net) in board.all_pads() {
        let r = size / 2.0;
        let mut pts = Vec::new();
        for i in 0..24 {
            let a = std::f64::consts::TAU * i as f64 / 24.0;
            pts.push((
                Point::new(
                    Mm((ox + x + r * a.cos()) as f32),
                    Mm((oy + y + r * a.sin()) as f32),
                ),
                false,
            ));
        }
        layer.add_polygon(Polygon {
            rings: vec![pts],
            mode: PaintMode::Fill,
            winding_order: WindingOrder::NonZero,
        });
    }

    // Reference labels (small, outside the copper where possible).
    layer.set_fill_color(Color::Rgb(Rgb::new(0.4, 0.4, 0.4, None)));
    for fp in &board.footprints {
        let (cx, cy) = fp.center();
        layer.use_text(
            fp.refdes.clone(),
            3.0,
            Mm((ox + cx - 2.0) as f32),
            Mm((oy + cy + 3.0) as f32),
            &font,
        );
    }

    let mut buf = BufWriter::new(File::create(path)?);
    doc.save(&mut buf)?;
    buf.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::placement::PlacementResult;
    use crate::router::Router;

    fn routed() -> (Board, RoutingResult) {
        let b = PlacementResult::auto_amplifier().board;
        let r = Router::new(0.5, b.width_mm, b.height_mm).route_board(&b);
        (b, r)
    }

    #[test]
    fn gerber_has_rs274x_header() {
        let (b, r) = routed();
        let dir = std::env::temp_dir().join("amprs_gbr");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.gbr").to_string_lossy().to_string();
        write_gerber(&b, &r, &p).unwrap();
        let s = std::fs::read_to_string(&p).unwrap();
        assert!(s.contains("%FSLAX46Y46*%"));
        assert!(s.contains("%MOMM*%"));
    }

    #[test]
    fn svg_is_well_formed() {
        let (b, r) = routed();
        let dir = std::env::temp_dir().join("amprs_svg");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.svg").to_string_lossy().to_string();
        write_svg(&b, &r, &p).unwrap();
        let s = std::fs::read_to_string(&p).unwrap();
        assert!(s.starts_with("<?xml"));
        assert!(s.trim_end().ends_with("</svg>"));
    }

    /// Regression: format 4.6 implies six fractional digits, so 200 mm must be
    /// written as X200000000 — not X200000 (which would be 0.2 mm).
    #[test]
    fn gerber_scale_matches_declared_format() {
        assert_eq!(gerber_units(1.0), 1_000_000);
        assert_eq!(gerber_units(200.0), 200_000_000);

        let (b, r) = routed();
        let dir = std::env::temp_dir().join("amprs_gbr_scale");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("s.gbr").to_string_lossy().to_string();
        write_gerber(&b, &r, &p).unwrap();
        let s = std::fs::read_to_string(&p).unwrap();

        // Leading zeros are omitted, so a valid 4.6 number has at most 10 digits
        // (4 integer + 6 fractional). Anything longer means the value is out of range.
        for line in s.lines().filter(|l| l.starts_with('X')) {
            let x_part = line.split('Y').next().unwrap().trim_start_matches('X');
            let digits: String = x_part.chars().take_while(|c| c.is_ascii_digit()).collect();
            assert!(
                digits.len() <= 10,
                "coordinate overflows 4.6 format: {line}"
            );
        }
        // The copper file must place at least one pad at true scale. Take a real
        // pad position and require its exact 4.6 encoding to appear.
        let first_pad = b.all_pads()[0].0;
        let expected = format!(
            "X{}Y{}",
            gerber_units(first_pad.0),
            gerber_units(first_pad.1)
        );
        assert!(
            s.contains(&expected),
            "pad not written at 1:1, expected {expected}"
        );

        // And no coordinate may describe a point outside the board envelope.
        for line in s.lines().filter(|l| l.starts_with('X')) {
            let rest = line.trim_start_matches('X');
            let x_part = rest.split('Y').next().unwrap();
            let digits: String = x_part.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(val) = digits.parse::<i64>() {
                let x_mm = val as f64 / 1_000_000.0;
                assert!(
                    x_mm <= b.width_mm + 0.01,
                    "X {x_mm} mm outside board width {}",
                    b.width_mm
                );
            }
        }
    }

    #[test]
    fn gerber_declares_each_distinct_pad_size() {
        let (b, r) = routed();
        let dir = std::env::temp_dir().join("amprs_gbr_pads");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("p.gbr").to_string_lossy().to_string();
        write_gerber(&b, &r, &p).unwrap();
        let s = std::fs::read_to_string(&p).unwrap();

        let mut sizes: Vec<f64> = b.all_pads().iter().map(|(_, size, _)| *size).collect();
        sizes.sort_by(|a, c| a.partial_cmp(c).unwrap());
        sizes.dedup_by(|a, c| (*a - *c).abs() < 1e-9);
        for size in sizes {
            let expected = format!("%ADD{}C,{:.3}*%", 0, size);
            // Match the aperture diameter regardless of the D-code number.
            assert!(
                s.lines()
                    .any(|l| l.starts_with("%ADD") && l.ends_with(&format!("C,{size:.3}*%"))),
                "missing aperture for pad size {size} (checked {expected})"
            );
        }
    }

    #[test]
    fn gerber_comments_use_star_terminator() {
        let (b, r) = routed();
        let dir = std::env::temp_dir().join("amprs_gbr_cmt");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("c.gbr").to_string_lossy().to_string();
        write_gerber(&b, &r, &p).unwrap();
        let s = std::fs::read_to_string(&p).unwrap();
        assert!(
            !s.contains('$'),
            "Gerber extended commands must end with *, not $"
        );
    }

    #[test]
    fn export_is_refused_when_board_is_invalid() {
        let (mut b, r) = routed();
        b.add_net(crate::board::Net::new(
            "SHORT",
            crate::board::NetClass::Power,
            vec![
                crate::board::PadRef::new("JIN", 0),
                crate::board::PadRef::new("JOUT", 0),
            ],
        ));
        let dir = std::env::temp_dir().join("amprs_gate");
        std::fs::create_dir_all(&dir).unwrap();
        let err = write_all(&b, &r, dir.to_str().unwrap())
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("two nets") || err.contains("validation"),
            "got {err}"
        );
    }

    #[test]
    fn export_is_refused_when_routing_failed() {
        let (b, mut r) = routed();
        r.failed_nets.push("BROKEN: no path".to_string());
        let dir = std::env::temp_dir().join("amprs_gate2");
        std::fs::create_dir_all(&dir).unwrap();
        let err = write_all(&b, &r, dir.to_str().unwrap())
            .unwrap_err()
            .to_string();
        assert!(err.contains("BROKEN"), "got {err}");
    }

    #[test]
    fn export_succeeds_for_a_clean_board() {
        let (b, r) = routed();
        assert!(
            b.validate().is_empty(),
            "demo board must validate: {:?}",
            b.validate()
        );
        assert!(
            r.failed_nets.is_empty(),
            "demo must route: {:?}",
            r.failed_nets
        );
        let dir = std::env::temp_dir().join("amprs_ok");
        std::fs::create_dir_all(&dir).unwrap();
        let files = write_all(&b, &r, dir.to_str().unwrap()).unwrap();
        assert_eq!(files.len(), 4);
        for f in &files {
            assert!(std::fs::metadata(f).unwrap().len() > 100, "{f} is empty");
        }
        let lut = std::fs::read(dir.join("lut_print.pdf")).unwrap();
        assert!(lut.starts_with(b"%PDF"));
    }
}
