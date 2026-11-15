from amproute_studio.core.models import Board, Component, Net, Pad
from amproute_studio.core.pipeline import PipelineOptions, run_pipeline
from amproute_studio.core.report import to_dict, to_markdown


def _board() -> Board:
    board = Board(bounds_mm=(0, 0, 80, 60))
    board.components["R1"] = Component(
        "R1", "1k", "R_0805", 10, 10, 0, "F.Cu",
        pads=[Pad("1", "IN", 0, 0), Pad("2", "MID", 0, 0)],
        courtyard_width_mm=4, courtyard_height_mm=2,
    )
    board.components["R2"] = Component(
        "R2", "1k", "R_0805", 40, 10, 0, "F.Cu",
        pads=[Pad("1", "MID", 0, 0), Pad("2", "OUT", 0, 0)],
        courtyard_width_mm=4, courtyard_height_mm=2,
    )
    board.nets["IN"] = Net("IN", 1, [("R1", "1")])
    board.nets["MID"] = Net("MID", 2, [("R1", "2"), ("R2", "1")])
    board.nets["OUT"] = Net("OUT", 3, [("R2", "2")])
    return board


def test_pipeline_runs_and_reports():
    result = run_pipeline(_board(), PipelineOptions(place=True, route=True, attempts=1))
    assert result.board is not None
    data = to_dict(result)
    assert "routing" in data
    assert "jumpers" in data
    assert "# AmpRoute Studio report" in to_markdown(result)
