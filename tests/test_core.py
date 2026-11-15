from amproute_studio.core.geometry import bbox_overlap, component_bbox
from amproute_studio.core.models import Board, Component, DesignRules
from amproute_studio.core.rules import LutProfile, classify_net
from amproute_studio.core.validation import validate_geometry


def test_classify_net():
    assert classify_net("GND") == "ground"
    assert classify_net("VCC") == "power"
    assert classify_net("OUT") == "high_current"
    assert classify_net("FB") == "feedback"
    assert classify_net("IN") == "signal"


def test_bbox_overlap():
    assert bbox_overlap((0, 0, 10, 10), (5, 5, 15, 15))
    assert not bbox_overlap((0, 0, 10, 10), (20, 20, 30, 30))


def test_component_bbox_rotation():
    bbox = component_bbox(0, 0, 90, 10, 4)
    assert abs(bbox[2] - bbox[0] - 4) < 1e-6
    assert abs(bbox[3] - bbox[1] - 10) < 1e-6


def test_validate_geometry_detects_overlap():
    board = Board(bounds_mm=(0, 0, 100, 100))
    board.components["R1"] = Component("R1", "1k", "R_0805", 20, 20, 0, "F.Cu", courtyard_width_mm=10, courtyard_height_mm=10)
    board.components["R2"] = Component("R2", "1k", "R_0805", 22, 22, 0, "F.Cu", courtyard_width_mm=10, courtyard_height_mm=10)
    report = validate_geometry(board, DesignRules())
    assert not report.ok
    assert any(v.code == "courtyard_overlap" for v in report.violations)


def test_lut_profile_defaults():
    lut = LutProfile()
    assert lut.min_clearance_mm > 0
    assert lut.min_track_mm > 0
