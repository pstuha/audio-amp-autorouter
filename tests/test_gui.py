import os

import pytest

PySide6 = pytest.importorskip("PySide6")


@pytest.fixture(scope="module")
def qapp():
    os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
    from PySide6.QtWidgets import QApplication

    app = QApplication.instance() or QApplication([])
    yield app


def test_main_window_builds(qapp):
    from amproute_studio.gui.app import MainWindow

    window = MainWindow()
    assert window.windowTitle() == "AmpRoute Studio"
    assert window.report.isReadOnly()


def test_board_view_accepts_pipeline_result(qapp, tmp_path):
    from pathlib import Path

    from amproute_studio.adapters.kicad_pcb import load_board
    from amproute_studio.core.pipeline import PipelineOptions, run_pipeline
    from amproute_studio.gui.app import BoardView

    example = tmp_path / "board.kicad_pcb"
    example.write_bytes(Path("examples/amplifier.kicad_pcb").read_bytes())
    board = load_board(example)
    result = run_pipeline(board, PipelineOptions(place=False, route=False))

    view = BoardView()
    view.show_board(result)  # must not raise
