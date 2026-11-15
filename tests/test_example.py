from pathlib import Path

from amproute_studio.adapters.kicad_pcb import load_board
from amproute_studio.core.pipeline import PipelineOptions, run_pipeline

EXAMPLE = Path(__file__).parent.parent / "examples" / "amplifier.kicad_pcb"


def test_example_board_parses():
    board = load_board(EXAMPLE)
    assert "J1" in board.components
    assert board.bounds_mm == (0.0, 0.0, 80.0, 60.0)
    assert board.nets["IN"].pins == [("J1", "1"), ("R1", "1")]


def test_example_board_pipeline_runs():
    board = load_board(EXAMPLE)
    result = run_pipeline(board, PipelineOptions(place=True, route=True, attempts=2))
    assert result.board is not None
    assert result.routing.routed_nets or result.routing.failed_nets


def test_example_board_pipeline_ok():
    board = load_board(EXAMPLE)
    result = run_pipeline(board, PipelineOptions(place=True, route=True, attempts=4))
    assert result.ok, [v for v in result.routing_report.violations] + [v for v in result.geometry_report.violations]
