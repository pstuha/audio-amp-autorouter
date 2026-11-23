//! Standards exporters and real jumper footprint insertion.

use amproute_studio::board::{Board, Footprint, LutPreset, Net, NetClass, Pad, PadRef};
use amproute_studio::export::{write_ipc2581_profile, write_odb_profile};
use amproute_studio::jumper::{apply_jumpers, Jumper};
use amproute_studio::router::{RouteSegment, RoutingResult};

fn board() -> Board {
    let mut b = Board::new("standards", 40.0, 30.0, LutPreset::reliable());
    b.add_footprint(
        Footprint::new("R1", "10k", NetClass::Signal, 5.0, 10.0, 10.0, 4.0).with_pads(vec![
            Pad::new(1.0, 2.0, 1.8, 0.9),
            Pad::new(9.0, 2.0, 1.8, 0.9),
        ]),
    );
    b.add_net(Net::new(
        "SIG",
        NetClass::Signal,
        vec![PadRef::new("R1", 0), PadRef::new("R1", 1)],
    ));
    b
}

fn routed() -> RoutingResult {
    RoutingResult {
        segments: vec![RouteSegment::new(
            6.0,
            12.0,
            14.0,
            12.0,
            "SIG",
            NetClass::Signal,
        )],
        routed_nets: vec!["SIG".into()],
        ..Default::default()
    }
}

#[test]
fn odb_profile_is_a_real_directory_with_matrix_netlist_and_features() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("board.odb");
    write_odb_profile(&board(), &routed(), dir.to_str().unwrap()).unwrap();
    assert!(dir.join("matrix").join("matrix").exists());
    assert!(dir.join("steps").join("pcb").join("stephdr").exists());
    assert!(dir.join("steps").join("pcb").join("netlists").join("netlist").exists());
    assert!(dir.join("steps").join("pcb").join("layers").join("top").join("features").exists());
    let netlist = std::fs::read_to_string(dir.join("steps").join("pcb").join("netlists").join("netlist")).unwrap();
    assert!(netlist.contains("NET=SIG CLASS=Signal"));
    let features = std::fs::read_to_string(dir.join("steps").join("pcb").join("layers").join("top").join("features")).unwrap();
    assert!(features.contains("L 6.0000 12.0000 14.0000 12.0000"));
}

#[test]
fn ipc2581_has_content_dictionary_netlist_and_tracks() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("board.xml");
    write_ipc2581_profile(&board(), &routed(), path.to_str().unwrap()).unwrap();
    let xml = std::fs::read_to_string(path).unwrap();
    for tag in ["<Content", "<Dictionary", "<Netlist>", "<Track", "</CadData>"] {
        assert!(xml.contains(tag), "missing {tag}");
    }
}

#[test]
fn jumpers_are_inserted_as_real_footprints() {
    let b = board();
    let j = Jumper {
        refdes: "JP_SIG".into(),
        net: "SIG".into(),
        class: NetClass::Signal,
        pad_a: (3.0, 5.0),
        pad_b: (10.0, 5.0),
        pad_size_mm: 1.6,
        wire_length_mm: 7.0,
        wire_diameter_mm: 0.6,
        replaces_via: false,
    };
    let with = apply_jumpers(&b, &[j]);
    assert_eq!(with.footprints.len(), b.footprints.len() + 1);
    let jp = with.footprint("JP_SIG").expect("JP footprint");
    assert_eq!(jp.pads.len(), 2);
    let (x, y) = jp.pad_world(0).unwrap();
    assert!((x - 3.0).abs() < 1e-6 && (y - 5.0).abs() < 1e-6);
    let (x, y) = jp.pad_world(1).unwrap();
    assert!((x - 10.0).abs() < 1e-6 && (y - 5.0).abs() < 1e-6);
    // The jumper is a wire, not copper: its net must not stay in the netlist.
    assert!(with.nets.iter().all(|n| n.name != "SIG"));
}
