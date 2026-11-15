"""AmpRoute Studio GUI — PySide6 desktop shell."""

from __future__ import annotations

import sys
from pathlib import Path

try:
    from PySide6.QtCore import Qt
except ImportError as exc:  # pragma: no cover
    raise SystemExit(
        "AmpRoute Studio GUI requires PySide6. Install it with: pip install -e \".[gui]\""
    ) from exc
from PySide6.QtGui import QAction, QBrush, QColor, QPainter, QPen
from PySide6.QtWidgets import (
    QApplication,
    QFileDialog,
    QGraphicsScene,
    QGraphicsView,
    QMainWindow,
    QMessageBox,
    QSplitter,
    QTextEdit,
    QToolBar,
    QVBoxLayout,
    QWidget,
)

from ..adapters.kicad_pcb import load_board
from ..core.pipeline import PipelineOptions, run_pipeline
from ..core.report import to_markdown


class BoardView(QGraphicsView):
    """Minimal board preview: components, routes and jumpers."""

    def __init__(self) -> None:
        super().__init__()
        self.scene_ = QGraphicsScene(self)
        self.setScene(self.scene_)
        self.setRenderHint(QPainter.Antialiasing)
        self.setDragMode(QGraphicsView.ScrollHandDrag)

    def show_board(self, result) -> None:
        self.scene_.clear()
        board = result.board
        if board.bounds_mm:
            minx, miny, maxx, maxy = board.bounds_mm
            self.scene_.addRect(minx, miny, maxx - minx, maxy - miny, QPen(QColor("#888"), 0.4))

        for comp in board.components.values():
            w = comp.courtyard_width_mm
            h = comp.courtyard_height_mm
            rect = self.scene_.addRect(
                comp.x_mm - w / 2, comp.y_mm - h / 2, w, h,
                QPen(QColor("#2d6cdf"), 0.2), QBrush(QColor(45, 108, 223, 40)),
            )
            rect.setToolTip(comp.ref)

        for seg in result.routing.segments:
            self.scene_.addLine(seg.x1_mm, seg.y1_mm, seg.x2_mm, seg.y2_mm, QPen(QColor("#c0392b"), max(0.2, seg.width_mm)))

        for jumper in result.jumpers.jumpers:
            self.scene_.addLine(jumper.x1_mm, jumper.y1_mm, jumper.x2_mm, jumper.y2_mm, QPen(QColor("#27ae60"), 0.5, Qt.DashLine))

        self.fitInView(self.scene_.itemsBoundingRect(), Qt.KeepAspectRatio)

    def wheelEvent(self, event) -> None:
        factor = 1.15 if event.angleDelta().y() > 0 else 1 / 1.15
        self.scale(factor, factor)


class MainWindow(QMainWindow):
    def __init__(self) -> None:
        super().__init__()
        self.setWindowTitle("AmpRoute Studio")
        self.resize(1400, 900)
        self._board_path: Path | None = None
        self._result = None

        self.view = BoardView()
        self.report = QTextEdit()
        self.report.setReadOnly(True)

        splitter = QSplitter(Qt.Horizontal)
        splitter.addWidget(self.view)
        splitter.addWidget(self.report)
        splitter.setSizes([900, 500])

        central = QWidget()
        layout = QVBoxLayout(central)
        layout.addWidget(splitter)
        self.setCentralWidget(central)

        self._build_toolbar()

    def _build_toolbar(self) -> None:
        toolbar = QToolBar("Main")
        self.addToolBar(toolbar)

        open_action = QAction("Open…", self)
        open_action.triggered.connect(self._open)
        toolbar.addAction(open_action)

        run_action = QAction("Run pipeline", self)
        run_action.triggered.connect(self._run)
        toolbar.addAction(run_action)

        save_action = QAction("Save…", self)
        save_action.triggered.connect(self._save)
        toolbar.addAction(save_action)

    def _open(self) -> None:
        path, _ = QFileDialog.getOpenFileName(self, "Open board", "", "KiCad PCB (*.kicad_pcb)")
        if not path:
            return
        self._board_path = Path(path)
        self.report.setPlainText(f"Loaded {path}")

    def _run(self) -> None:
        if not self._board_path:
            QMessageBox.warning(self, "AmpRoute Studio", "Open a board first")
            return
        try:
            board = load_board(self._board_path)
            self._result = run_pipeline(board, PipelineOptions())
            self.view.show_board(self._result)
            self.report.setPlainText(to_markdown(self._result))
        except Exception as exc:  # pragma: no cover - GUI feedback
            QMessageBox.critical(self, "AmpRoute Studio", str(exc))

    def _save(self) -> None:
        if self._result is None:
            QMessageBox.warning(self, "AmpRoute Studio", "Run the pipeline first")
            return
        path, _ = QFileDialog.getSaveFileName(self, "Save routed board", "", "KiCad PCB (*.kicad_pcb)")
        if not path:
            return
        from ..adapters.kicad_pcb import save_board

        save_board(self._result.board, path, self._result.routing.segments, self._result.jumpers.jumpers)
        self.report.append(f"\nSaved {path}")


def run(argv=None) -> int:
    args = list(argv) if argv is not None else sys.argv[1:]
    if "-h" in args or "--help" in args:
        print("AmpRoute Studio — desktop autorouter for single-layer audio amplifier PCBs (LUT).")
        print("Usage: amprote-studio [--help] [--version]")
        print("Launch with no arguments to open the GUI.")
        return 0
    if "--version" in args:
        from .. import __version__

        print(f"amproute-studio {__version__}")
        return 0
    app = QApplication.instance() or QApplication(sys.argv)
    window = MainWindow()
    window.show()
    return app.exec()


if __name__ == "__main__":
    raise SystemExit(run())
