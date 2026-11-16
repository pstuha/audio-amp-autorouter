"""AmpRoute Studio GUI — PySide6 professional interface."""

from __future__ import annotations

import sys
from pathlib import Path

try:
    from PySide6.QtCore import Qt, QThread, Signal, Slot
    from PySide6.QtGui import QAction, QColor, QPainter, QPen
    from PySide6.QtWidgets import (
        QApplication, QFileDialog, QGraphicsScene, QGraphicsView,
        QMainWindow, QMessageBox, QVBoxLayout, QHBoxLayout,
        QWidget, QLabel, QPushButton, QProgressBar, QStatusBar,
        QToolBar, QDockWidget, QListWidget, QCheckBox, QSpinBox,
        QGroupBox, QFormLayout, QTabWidget, QTextEdit, QSplitter,
    )
except ImportError as exc:
    raise SystemExit(
        "AmpRoute Studio GUI requires PySide6. Install: pip install -e \".[gui]\""
    ) from exc

from ..adapters.kicad_pcb import load_board
from ..core.pipeline import PipelineOptions, run_pipeline
from ..core.report import to_markdown
from ..core.simulations import (
    analyze_thermal, analyze_crosstalk, estimate_emi, analyze_power_bus,
    analyze_current_loops, predict_damping_factor, estimate_thd,
    predict_psrr, estimate_phase_margin, check_slew_rate,
    generate_thermal_heatmap, run_full_simulation,
)
from ..core.models import RouteSegment


class SimulationWorker(QThread):
    """Worker thread for running simulations without blocking UI."""
    progress = Signal(str)
    finished = Signal(dict)
    error = Signal(str)

    def __init__(self, func, *args, **kwargs):
        super().__init__()
        self.func = func
        self.args = args
        self.kwargs = kwargs

    def run(self):
        try:
            self.progress.emit(f"Running {self.func.__name__}...")
            result = self.func(*self.args, **self.kwargs)
            self.finished.emit(result if result is not None else {})
        except Exception as e:
            self.error.emit(str(e))


class RoutingWorker(QThread):
    """Worker thread for routing operations."""
    progress = Signal(str)
    finished = Signal(object)
    error = Signal(str)

    def __init__(self, board, options):
        super().__init__()
        self.board = board
        self.options = options

    def run(self):
        try:
            self.progress.emit("Routing in progress...")
            result = run_pipeline(self.board, self.options)
            self.finished.emit(result)
        except Exception as e:
            self.error.emit(str(e))


class MainWindow(QMainWindow):
    """Professional AmpRoute Studio interface."""

    def __init__(self):
        super().__init__()
        self.board = None
        self.current_result = None
        self.workers = []
        self.setup_ui()

    def setup_ui(self):
        self.setWindowTitle("AmpRoute Studio")
        self.setGeometry(100, 100, 1400, 900)

        # Central widget with tabs
        self.tabs = QTabWidget()
        self.setCentralWidget(self.tabs)

        # Board view tab
        self.board_tab = QWidget()
        self.tabs.addTab(self.board_tab, "Board")
        self._setup_board_tab()

        # Simulations tab
        self.sim_tab = QWidget()
        self.tabs.addTab(self.sim_tab, "Simulations")
        self._setup_sim_tab()

        # Routing tab
        self.route_tab = QWidget()
        self.tabs.addTab(self.route_tab, "Routing")
        self._setup_route_tab()

        # Status bar
        self.status_bar = QStatusBar()
        self.setStatusBar(self.status_bar)
        self.progress_bar = QProgressBar()
        self.status_bar.addPermanentWidget(self.progress_bar)
        self.progress_bar.setVisible(False)

    def _setup_board_tab(self):
        layout = QVBoxLayout(self.board_tab)

        # Toolbar
        toolbar = QToolBar()
        layout.addWidget(toolbar)

        open_btn = QPushButton("Open PCB")
        open_btn.clicked.connect(self.open_file)
        toolbar.addWidget(open_btn)

        toolbar.addSeparator()

        # Manual edit tools would go here
        self.edit_label = QLabel("Manual editing: Select tool from Routing tab")
        toolbar.addWidget(self.edit_label)

        # Board graphics view
        self.scene = QGraphicsScene(self)
        self.view = QGraphicsView(self.scene)
        self.view.setRenderHint(QPainter.Antialiasing)
        self.view.setDragMode(QGraphicsView.ScrollHandDrag)
        layout.addWidget(self.view)

    def _setup_sim_tab(self):
        """Simulation panel with separate buttons for each analysis."""
        layout = QVBoxLayout(self.sim_tab)

        # Info label
        layout.addWidget(QLabel("Select simulation to run:"))

        # Grid of simulation buttons
        sim_grid = QWidget()
        sim_grid_layout = QVBoxLayout(sim_grid)

        # Row 1: Primary simulations
        row1 = QHBoxLayout()
        self.btn_thermal = QPushButton("Thermal Analysis")
        self.btn_thermal.clicked.connect(lambda: self.run_simulation("thermal"))
        self.btn_current_loops = QPushButton("Current Loops")
        self.btn_current_loops.clicked.connect(lambda: self.run_simulation("current_loops"))
        self.btn_df = QPushButton("Damping Factor")
        self.btn_df.clicked.connect(lambda: self.run_simulation("df"))
        row1.addWidget(self.btn_thermal)
        row1.addWidget(self.btn_current_loops)
        row1.addWidget(self.btn_df)
        sim_grid_layout.addLayout(row1)

        # Row 2: Secondary simulations
        row2 = QHBoxLayout()
        self.btn_thd = QPushButton("THD Estimate")
        self.btn_thd.clicked.connect(lambda: self.run_simulation("thd"))
        self.btn_psrr = QPushButton("PSRR")
        self.btn_psrr.clicked.connect(lambda: self.run_simulation("psrr"))
        self.btn_crosstalk = QPushButton("Crosstalk")
        self.btn_crosstalk.clicked.connect(lambda: self.run_simulation("crosstalk"))
        row2.addWidget(self.btn_thd)
        row2.addWidget(self.btn_psrr)
        row2.addWidget(self.btn_crosstalk)
        sim_grid_layout.addLayout(row2)

        # Row 3: Advanced simulations
        row3 = QHBoxLayout()
        self.btn_emi = QPushButton("EMI Prediction")
        self.btn_emi.clicked.connect(lambda: self.run_simulation("emi"))
        self.btn_power_bus = QPushButton("Power Bus")
        self.btn_power_bus.clicked.connect(lambda: self.run_simulation("power_bus"))
        self.btn_stability = QPushButton("Stability")
        self.btn_stability.clicked.connect(lambda: self.run_simulation("stability"))
        row3.addWidget(self.btn_emi)
        row3.addWidget(self.btn_power_bus)
        row3.addWidget(self.btn_stability)
        sim_grid_layout.addLayout(row3)

        # Row 4: Run All
        row4 = QHBoxLayout()
        self.btn_run_all = QPushButton("Run All Simulations")
        self.btn_run_all.clicked.connect(lambda: self.run_simulation("all"))
        self.btn_run_all.setStyleSheet("background-color: #4CAF50; color: white; padding: 10px;")
        row4.addWidget(self.btn_run_all)
        sim_grid_layout.addLayout(row4)

        layout.addWidget(sim_grid)

        # Results display
        self.results_display = QTextEdit()
        self.results_display.setReadOnly(True)
        self.results_display.setPlaceholderText("Simulation results will appear here...")
        layout.addWidget(QLabel("Results:"))
        layout.addWidget(self.results_display)

    def _setup_route_tab(self):
        """Routing panel with separate operations."""
        layout = QVBoxLayout(self.route_tab)

        # Route options
        options_group = QGroupBox("Routing Options")
        options_layout = QFormLayout()

        self.cb_place = QCheckBox("Placement")
        self.cb_place.setChecked(True)
        self.cb_route = QCheckBox("Routing")
        self.cb_route.setChecked(True)
        self.attempts_spin = QSpinBox()
        self.attempts_spin.setRange(1, 10)
        self.attempts_spin.setValue(3)

        options_layout.addRow("Placement:", self.cb_place)
        options_layout.addRow("Routing:", self.cb_route)
        options_layout.addRow("Attempts:", self.attempts_spin)
        options_group.setLayout(options_layout)
        layout.addWidget(options_group)

        # Routing buttons
        route_buttons = QWidget()
        route_btn_layout = QVBoxLayout(route_buttons)

        self.btn_route = QPushButton("Run Routing")
        self.btn_route.clicked.connect(self.run_routing)
        self.btn_clear = QPushButton("Clear All Routes")
        self.btn_clear.clicked.connect(self.clear_routes)
        self.btn_route.setStyleSheet("background-color: #2196F3; color: white; padding: 15px;")
        route_btn_layout.addWidget(self.btn_route)
        route_btn_layout.addWidget(self.btn_clear)

        layout.addWidget(route_buttons)

        # Manual edit section
        edit_group = QGroupBox("Manual Editing")
        edit_layout = QVBoxLayout(edit_group)

        edit_info = QLabel("Manual track editing not yet implemented.\nUse KiCad for detailed editing.")
        edit_layout.addWidget(edit_info)

        layout.addWidget(edit_group)

        layout.addStretch()

    def open_file(self):
        path, _ = QFileDialog.getOpenFileName(
            self, "Open KiCad PCB", "", "KiCad PCB Files (*.kicad_pcb)"
        )
        if path:
            try:
                self.board = load_board(path)
                self.status_bar.showMessage(f"Loaded: {Path(path).name}")
                self.display_board()
            except Exception as e:
                QMessageBox.critical(self, "Error", f"Failed to load PCB: {e}")

    def display_board(self):
        self.scene.clear()
        if not self.board or not self.board.bounds_mm:
            return

        minx, miny, maxx, maxy = self.board.bounds_mm
        w, h = maxx - minx, maxy - miny
        self.scene.setSceneRect(minx - 10, miny - 10, w + 20, h + 20)

        # Draw board outline
        self.scene.addRect(minx, miny, w, h, QPen(QColor("#333"), 2))

        # Draw components (simplified)
        for ref, comp in self.board.components.items():
            x, y = comp.x_mm, comp.y_mm
            self.scene.addRect(x - 2, y - 2, 4, 4, QPen(QColor("#0066cc")))

        self.view.fitInView()

    def run_simulation(self, sim_type):
        if not self.board:
            QMessageBox.warning(self, "Warning", "Please open a PCB file first")
            return

        self.progress_bar.setVisible(True)
        self.progress_bar.setRange(0, 0)  # indeterminate
        self.status_bar.showMessage(f"Running {sim_type}...")

        # Run in worker thread
        if sim_type == "thermal":
            worker = SimulationWorker(analyze_thermal, self.board)
        elif sim_type == "current_loops":
            worker = SimulationWorker(analyze_current_loops, self.board, self.board.components.get('segments', []))
        elif sim_type == "df":
            worker = SimulationWorker(predict_damping_factor, self.board)
        elif sim_type == "thd":
            worker = SimulationWorker(estimate_thd, self.board, [])
        elif sim_type == "psrr":
            worker = SimulationWorker(predict_psrr, self.board, [])
        elif sim_type == "crosstalk":
            worker = SimulationWorker(analyze_crosstalk, self.board, [])
        elif sim_type == "emi":
            worker = SimulationWorker(estimate_emi, self.board)
        elif sim_type == "power_bus":
            worker = SimulationWorker(analyze_power_bus, self.board, [])
        elif sim_type == "stability":
            worker = SimulationWorker(estimate_phase_margin, self.board)
        elif sim_type == "all":
            worker = SimulationWorker(run_full_simulation, self.board)
        else:
            return

        worker.finished.connect(lambda r: self._on_sim_complete(sim_type, r))
        worker.error.connect(lambda e: self._on_error(e))
        worker.start()
        self.workers.append(worker)

    def _on_sim_complete(self, sim_type, result):
        self.progress_bar.setVisible(False)
        self.status_bar.showMessage(f"{sim_type} complete")

        if not result:
            return

        # Format results
        if sim_type == "all":
            text = self._format_full_report(result)
        elif sim_type == "thermal":
            text = self._format_thermal(result)
        elif sim_type == "current_loops":
            text = self._format_loops(result)
        else:
            text = str(result)

        self.results_display.setPlainText(text)

    def _format_thermal(self, results):
        lines = ["Thermal Analysis Results:", "=" * 40]
        for t in results:
            lines.append(f"{t.component_ref}: Tj={t.junction_temp_c}°C, P={t.power_dissipation_w}W")
        return "\n".join(lines)

    def _format_loops(self, loops):
        lines = ["Current Loop Analysis:", "=" * 40]
        for l in loops[:10]:
            lines.append(f"{l.net_name}: area={l.area_mm2:.1f}mm², L={l.inductance_nh:.2f}nH")
        return "\n".join(lines)

    def _format_full_report(self, report):
        lines = ["Full Simulation Report:", "=" * 40]
        lines.append(f"\nAudio Parameters:")
        lines.append(f"  DF: {report.audio.damping_factor}")
        lines.append(f"  THD: {report.audio.thd_percent:.4f}%")
        lines.append(f"\nEMI: {report.emi.score if report.emi else 'N/A'}")
        lines.append(f"\nThermal: {len(report.thermal_results)} components")
        lines.append(f"\nCrosstalk: {len(report.crosstalk)} pairs")
        if report.power_bus:
            lines.append(f"\nPower Bus Rails: {list(report.power_bus.rail_impedance_mohm.keys())}")
        lines.append(f"\nRecommendations: {len(report.recommendations)}")
        return "\n".join(lines)

    def run_routing(self):
        if not self.board:
            QMessageBox.warning(self, "Warning", "Please open a PCB file first")
            return

        self.progress_bar.setVisible(True)
        self.status_bar.showMessage("Routing...")

        options = PipelineOptions(
            place=self.cb_place.isChecked(),
            route=self.cb_route.isChecked(),
            attempts=self.attempts_spin.value(),
        )

        worker = RoutingWorker(self.board, options)
        worker.finished.connect(self._on_routing_complete)
        worker.error.connect(self._on_error)
        worker.start()
        self.workers.append(worker)

    def _on_routing_complete(self, result):
        self.progress_bar.setVisible(False)
        self.status_bar.showMessage("Routing complete")

        self.current_result = result
        if result and result.ok:
            self.status_bar.showMessage(f"Success! Nets routed: {len(result.routing.routed_nets)}")
        else:
            errors = result.geometry.violations if result and hasattr(result, 'geometry') else []
            if errors:
                self.status_bar.showMessage(f"Issues: {len(errors)} geometry violations")

    def clear_routes(self):
        if not self.board:
            return
        QMessageBox.information(self, "Clear", "Clear routes not yet implemented")

    def _on_error(self, msg):
        self.progress_bar.setVisible(False)
        self.status_bar.showMessage(f"Error: {msg}")
        QMessageBox.critical(self, "Error", msg)


def run():
    app = QApplication(sys.argv)
    window = MainWindow()
    window.show()
    sys.exit(app.exec())
