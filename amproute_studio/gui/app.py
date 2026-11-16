"""AmpRoute Studio GUI — Professional PySide6 interface with 3D visualization."""

from __future__ import annotations

import sys
from pathlib import Path
from typing import Dict, List, Optional, Any

try:
    from PySide6.QtCore import Qt, QThread, Signal
    from PySide6.QtWidgets import (
        QApplication, QFileDialog, QMainWindow, QMessageBox,
        QVBoxLayout, QHBoxLayout, QWidget, QLabel, QPushButton,
        QProgressBar, QStatusBar, QToolBar, QGroupBox, QFormLayout,
        QTabWidget, QTextEdit, QSplitter, QSlider, QGridLayout,
        QCheckBox, QSpinBox, QListWidget, QComboBox, QFrame,
    )
except ImportError as exc:
    raise SystemExit("AmpRoute Studio GUI requires PySide6. Install: pip install -e \".[gui]\"")

from matplotlib.backends.backend_qtagg import FigureCanvasQTAgg
from matplotlib.figure import Figure
from matplotlib.colors import LinearSegmentedColormap
import numpy as np

from ..adapters.kicad_pcb import load_board
from ..core.pipeline import PipelineOptions, run_pipeline
from ..core.models import Board, RouteSegment
from ..core.simulations import (
    analyze_thermal, analyze_crosstalk, estimate_emi, analyze_power_bus,
    analyze_current_loops, predict_damping_factor, estimate_thd,
    predict_psrr, estimate_phase_margin, run_full_simulation, ThermalResult,
)
from .styles import DARK_STYLESHEET


THERMAL_CMAP = LinearSegmentedColormap.from_list('thermal',
    ['#00ff00', '#44ff00', '#88ff00', '#ccff00', '#ffff00', '#ffcc00', '#ff8800', '#ff4400', '#ff0000'])


class SimulationWorker(QThread):
    finished = Signal(dict)
    progress = Signal(str)
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
    finished = Signal(object)
    progress = Signal(str)
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


class MplCanvas3D(FigureCanvasQTAgg):
    """Matplotlib 3D canvas for PCB visualization."""

    def __init__(self, parent=None, width=10, height=7, dpi=80):
        self.fig = Figure(figsize=(width, height), dpi=dpi, facecolor='#1a1a2e')
        self.ax = self.fig.add_subplot(111, projection='3d', facecolor='#1a1a2e')
        super().__init__(self.fig)
        self.setParent(parent)
        self.fig.tight_layout(pad=0)

    def draw_board(self, board: Board, segments: List[RouteSegment] = None,
                   thermal_data: Dict[str, float] = None, show_thermal: bool = False):
        """Draw the board in 3D with optional thermal overlay."""
        self.ax.clear()
        self.ax.set_facecolor('#1a1a2e')

        if not board or not board.bounds_mm:
            self.draw_empty()
            return

        minx, miny, maxx, maxy = board.bounds_mm

        # Board outline
        self.ax.plot([minx, maxx, maxx, minx, minx],
                    [miny, miny, maxy, maxy, miny], [0]*5, 'w-', linewidth=2, alpha=0.5)

        # Board surface
        xx, yy = np.meshgrid([minx, maxx], [miny, maxy])
        self.ax.plot_surface(xx, yy, np.zeros_like(xx), alpha=0.2, color='#2a2a3a')

        # Components
        if board.components:
            for ref, comp in board.components.items():
                cx, cy = comp.x_mm, comp.y_mm
                w, h = getattr(comp, 'width_mm', 4), getattr(comp, 'height_mm', 4)

                if show_thermal and thermal_data and ref in thermal_data:
                    temp = thermal_data[ref]
                    norm = min(1.0, max(0.0, (temp - 30) / 90))
                    color = (1.0, norm * 2, 0.0) if norm < 0.5 else (1.0 - (norm - 0.5) * 2, 1.0, 0.0)
                else:
                    color = '#4488ff'

                self.ax.bar3d(cx - w/2, cy - h/2, 0, w, h, 0.3,
                            color=color, alpha=0.9, edgecolor='white', linewidth=0.5)
                self.ax.text(cx, cy, 0.4, ref, fontsize=5, ha='center', color='white')

        # Tracks
        if segments:
            colors = {'power': '#ff4444', 'ground': '#44ff44', 'signal': '#4488ff',
                      'feedback': '#ffaa00', 'high_current': '#ff8800', 'control': '#aa44ff'}
            for seg in segments:
                layer = getattr(seg, 'layer', 'F.Cu')
                z = 0.2 if layer == 'B.Cu' else 0.15
                net_class = getattr(seg, 'net_class', 'signal')
                color = colors.get(net_class, '#88aaff')
                self.ax.plot([seg.x1, seg.x2], [seg.y1, seg.y2], [z]*2,
                           color=color, linewidth=2.5, alpha=0.9)

        self.ax.set_xlabel('X (mm)', color='white', fontsize=9)
        self.ax.set_ylabel('Y (mm)', color='white', fontsize=9)
        self.ax.set_zlabel('Z (mm)', color='white', fontsize=9)
        self.ax.tick_params(colors='white', labelsize=7)
        for axis in [self.ax.xaxis, self.ax.yaxis, self.ax.zaxis]:
            axis.pane.fill = False
            axis.pane.set_edgecolor('#333333')
        self.ax.set_title('3D Board View', color='white', fontsize=12, pad=10)
        self.ax.view_init(elev=25, azim=45)
        self.draw()

    def draw_thermal_map(self, board: Board, thermal_results: List[ThermalResult]):
        """Draw thermal heatmap."""
        self.ax.clear()
        self.ax.set_facecolor('#1a1a2e')

        if not board or not board.bounds_mm:
            self.draw_empty()
            return

        minx, miny, maxx, maxy = board.bounds_mm
        self.ax.plot([minx, maxx, maxx, minx, minx],
                    [miny, miny, maxy, maxy, miny], [0]*5, 'w-', linewidth=2)

        if thermal_results:
            for t in thermal_results:
                if t.component_ref in board.components:
                    comp = board.components[t.component_ref]
                    norm = min(1.0, max(0.0, (t.junction_temp_c - 30) / 90))
                    color = (1.0, norm * 2, 0.0) if norm < 0.5 else (1.0 - (norm - 0.5) * 2, 1.0, 0.0)
                    self.ax.bar3d(comp.x_mm - 2, comp.y_mm - 2, 0, 4, 4, 0.5,
                                color=color, alpha=0.95, edgecolor='white')
                    self.ax.text(comp.x_mm, comp.y_mm, 0.6, f'{t.junction_temp_c:.1f}°C',
                               fontsize=7, ha='center', color='white')

        self.ax.set_xlabel('X (mm)', color='white')
        self.ax.set_ylabel('Y (mm)', color='white')
        self.ax.set_zlabel('°C', color='white')
        self.ax.set_title('Thermal Distribution', color='white', pad=10)
        self.ax.tick_params(colors='white')
        for axis in [self.ax.xaxis, self.ax.yaxis, self.ax.zaxis]:
            axis.pane.fill = False
            axis.pane.set_edgecolor('#333333')
        self.ax.view_init(elev=30, azim=45)
        self.draw()

    def draw_empty(self):
        self.ax.clear()
        self.ax.set_facecolor('#1a1a2e')
        self.ax.text(0.5, 0.5, 0.5, 'Open a PCB file to visualize',
                    transform=self.ax.transAxes, color='gray', fontsize=14,
                    ha='center', va='center')
        self.ax.set_axis_off()
        self.draw()

    def update_view_angle(self, elev: float, azim: float):
        self.ax.view_init(elev=elev, azim=azim)
        self.draw()


class MainWindow(QMainWindow):
    """Professional AmpRoute Studio interface."""

    def __init__(self):
        super().__init__()
        self.board: Optional[Board] = None
        self.current_result: Optional[object] = None
        self.segments: List[RouteSegment] = []
        self.thermal_data: Dict[str, float] = {}
        self.workers: List[QThread] = []

        self.setup_ui()
        self.setStyleSheet(DARK_STYLESHEET)

    def setup_ui(self):
        self.setWindowTitle("AmpRoute Studio")
        self.setGeometry(100, 100, 1600, 1000)
        self.setMinimumSize(1200, 800)

        # Toolbar
        toolbar = QToolBar("Main Toolbar")
        toolbar.setMovable(False)
        self.addToolBar(toolbar)

        open_btn = QPushButton("Open PCB")
        open_btn.clicked.connect(self.open_file)
        open_btn.setObjectName("primaryButton")
        toolbar.addWidget(open_btn)

        toolbar.addSeparator()

        route_btn = QPushButton("Auto Route")
        route_btn.clicked.connect(self.run_routing)
        route_btn.setObjectName("primaryButton")
        toolbar.addWidget(route_btn)

        sim_btn = QPushButton("Run All")
        sim_btn.clicked.connect(lambda: self.run_simulation("all"))
        toolbar.addWidget(sim_btn)

        toolbar.addSeparator()
        self.status_label = QLabel("Ready")
        toolbar.addWidget(self.status_label)

        # Central widget
        central = QWidget()
        self.setCentralWidget(central)
        main_layout = QVBoxLayout(central)

        # Main splitter
        splitter = QSplitter(Qt.Horizontal)
        main_layout.addWidget(splitter)

        # Left panel
        left_panel = self._create_left_panel()
        splitter.addWidget(left_panel)

        # Center
        center = QWidget()
        center_layout = QVBoxLayout(center)

        # 3D View
        self.view_3d = MplCanvas3D(self, width=10, height=7, dpi=80)
        self.view_3d.draw_empty()
        center_layout.addWidget(self.view_3d, 3)

        # View controls
        slider_container = QWidget()
        slider_layout = QHBoxLayout(slider_container)
        slider_layout.addWidget(QLabel("Elevation:"))
        self.elev_slider = QSlider(Qt.Horizontal)
        self.elev_slider.setRange(0, 90)
        self.elev_slider.setValue(25)
        self.elev_slider.valueChanged.connect(self._on_view_change)
        slider_layout.addWidget(self.elev_slider)
        slider_layout.addWidget(QLabel("Azimuth:"))
        self.azim_slider = QSlider(Qt.Horizontal)
        self.azim_slider.setRange(0, 360)
        self.azim_slider.setValue(45)
        self.azim_slider.valueChanged.connect(self._on_view_change)
        slider_layout.addWidget(self.azim_slider)
        reset_btn = QPushButton("Reset")
        reset_btn.clicked.connect(self._reset_view)
        slider_layout.addWidget(reset_btn)
        center_layout.addWidget(slider_container)

        # Tabs
        self.tabs = QTabWidget()
        center_layout.addWidget(self.tabs, 2)

        self._setup_sim_tab()
        self._setup_route_tab()
        self._setup_results_tab()

        splitter.addWidget(center)
        splitter.setStretchFactor(0, 0)
        splitter.setStretchFactor(1, 1)

        # Status bar
        self.status_bar = QStatusBar()
        self.setStatusBar(self.status_bar)
        self.progress_bar = QProgressBar()
        self.progress_bar.setMaximumWidth(200)
        self.progress_bar.setVisible(False)
        self.status_bar.addPermanentWidget(self.progress_bar)

    def _create_left_panel(self):
        panel = QWidget()
        layout = QVBoxLayout(panel)

        info_group = QGroupBox("Project")
        info_layout = QVBoxLayout(info_group)
        self.info_label = QLabel("No file loaded")
        info_layout.addWidget(self.info_label)
        layout.addWidget(info_group)

        comp_group = QGroupBox("Components")
        comp_layout = QVBoxLayout(comp_group)
        self.comp_list = QListWidget()
        comp_layout.addWidget(self.comp_list)
        layout.addWidget(comp_group)

        nets_group = QGroupBox("Nets")
        nets_layout = QVBoxLayout(nets_group)
        self.nets_list = QListWidget()
        nets_layout.addWidget(self.nets_list)
        layout.addWidget(nets_group)

        stats_group = QGroupBox("Statistics")
        stats_layout = QFormLayout(stats_group)
        self.stats_routed = QLabel("0")
        self.stats_jumpers = QLabel("0")
        self.stats_thermal = QLabel("N/A")
        stats_layout.addRow("Routed:", self.stats_routed)
        stats_layout.addRow("Jumpers:", self.stats_jumpers)
        stats_layout.addRow("Max Temp:", self.stats_thermal)
        layout.addWidget(stats_group)

        return panel

    def _setup_sim_tab(self):
        self.sim_tab = QWidget()
        self.tabs.addTab(self.sim_tab, "Simulations")
        layout = QVBoxLayout(self.sim_tab)
        layout.addWidget(QLabel("Select simulation:"))

        sim_grid = QGridLayout()
        simulations = [
            ("Thermal", "sim_thermal", "thermal"),
            ("Loops", "sim_loops", "current_loops"),
            ("DF", "sim_df", "df"),
            ("THD", "sim_thd", "thd"),
            ("PSRR", "sim_psrr", "psrr"),
            ("Crosstalk", "sim_crosstalk", "crosstalk"),
            ("EMI", "sim_emi", "emi"),
            ("Power", "sim_power", "power_bus"),
            ("Stable", "sim_stability", "stability"),
        ]

        for i, (name, obj_name, key) in enumerate(simulations):
            btn = QPushButton(name)
            btn.setObjectName(obj_name)
            btn.clicked.connect(lambda checked, k=key: self.run_simulation(k))
            btn.setMinimumHeight(40)
            sim_grid.addWidget(btn, i // 3, i % 3)

        layout.addLayout(sim_grid)

        run_all_btn = QPushButton("Run All Simulations")
        run_all_btn.setObjectName("run_all")
        run_all_btn.setMinimumHeight(50)
        run_all_btn.clicked.connect(lambda: self.run_simulation("all"))
        layout.addWidget(run_all_btn)

        view_group = QGroupBox("Visualization")
        view_layout = QHBoxLayout(view_group)
        self.cb_show_thermal = QCheckBox("Show Thermal on 3D")
        self.cb_show_thermal.stateChanged.connect(self._update_3d_view)
        view_layout.addWidget(self.cb_show_thermal)
        layout.addWidget(view_group)
        layout.addStretch()

    def _setup_route_tab(self):
        self.route_tab = QWidget()
        self.tabs.addTab(self.route_tab, "Routing")
        layout = QVBoxLayout(self.route_tab)

        opt_group = QGroupBox("Options")
        opt_layout = QFormLayout(opt_group)
        self.cb_place = QCheckBox("Run Placement")
        self.cb_place.setChecked(True)
        opt_layout.addRow("", self.cb_place)
        self.cb_route = QCheckBox("Run Routing")
        self.cb_route.setChecked(True)
        opt_layout.addRow("", self.cb_route)
        self.cb_sim_feedback = QCheckBox("Simulation-Aware")
        self.cb_sim_feedback.setChecked(True)
        opt_layout.addRow("", self.cb_sim_feedback)
        attempts_layout = QHBoxLayout()
        attempts_layout.addWidget(QLabel("Iterations:"))
        self.attempts_spin = QSpinBox()
        self.attempts_spin.setRange(1, 20)
        self.attempts_spin.setValue(5)
        attempts_layout.addWidget(self.attempts_spin)
        opt_layout.addRow("", attempts_layout)
        layout.addWidget(opt_group)

        route_btn = QPushButton("Run Routing with Simulations")
        route_btn.setObjectName("primaryButton")
        route_btn.setMinimumHeight(50)
        route_btn.clicked.connect(self.run_routing)
        layout.addWidget(route_btn)

        clear_btn = QPushButton("Clear Routes")
        clear_btn.setObjectName("dangerButton")
        clear_btn.clicked.connect(self.clear_routes)
        layout.addWidget(clear_btn)

        self.route_status = QLabel("Ready")
        layout.addWidget(self.route_status)
        layout.addStretch()

    def _setup_results_tab(self):
        self.results_tab = QWidget()
        self.tabs.addTab(self.results_tab, "Results")
        layout = QVBoxLayout(self.results_tab)
        layout.addWidget(QLabel("Simulation Results:"))
        self.results_text = QTextEdit()
        self.results_text.setReadOnly(True)
        self.results_text.setObjectName("resultsDisplay")
        layout.addWidget(self.results_text)

    def _on_view_change(self):
        self.view_3d.update_view_angle(self.elev_slider.value(), self.azim_slider.value())

    def _reset_view(self):
        self.elev_slider.setValue(25)
        self.azim_slider.setValue(45)
        self.view_3d.update_view_angle(25, 45)

    def _update_3d_view(self):
        if self.board:
            self.view_3d.draw_board(self.board, self.segments,
                                  self.thermal_data if self.cb_show_thermal.isChecked() else None,
                                  self.cb_show_thermal.isChecked())

    def open_file(self):
        path, _ = QFileDialog.getOpenFileName(
            self, "Open KiCad PCB", "", "KiCad PCB Files (*.kicad_pcb)"
        )
        if path:
            try:
                self.board = load_board(path)
                self.status_bar.showMessage(f"Loaded: {Path(path).name}")
                self.info_label.setText(f"File: {Path(path).name}\n"
                                        f"Components: {len(self.board.components)}\n"
                                        f"Nets: {len(self.board.nets)}")
                self.comp_list.clear()
                for ref in sorted(self.board.components.keys()):
                    self.comp_list.addItem(ref)
                self.nets_list.clear()
                for net in sorted(self.board.nets.keys())[:50]:
                    self.nets_list.addItem(net)
                self.view_3d.draw_board(self.board)
            except Exception as e:
                QMessageBox.critical(self, "Error", f"Failed to load PCB:\n{e}")

    def run_simulation(self, sim_type: str):
        if not self.board:
            QMessageBox.warning(self, "Warning", "Open a PCB file first")
            return

        self.progress_bar.setVisible(True)
        self.status_bar.showMessage(f"Running {sim_type}...")
        self.status_label.setText(f"Running {sim_type}...")

        funcs = {
            'thermal': (analyze_thermal, self.board),
            'current_loops': (analyze_current_loops, self.board, []),
            'df': (predict_damping_factor, self.board),
            'thd': (estimate_thd, self.board, []),
            'psrr': (predict_psrr, self.board, []),
            'crosstalk': (analyze_crosstalk, self.board, []),
            'emi': (estimate_emi, self.board),
            'power_bus': (analyze_power_bus, self.board, []),
            'stability': (estimate_phase_margin, self.board),
            'all': (run_full_simulation, self.board),
        }

        if sim_type not in funcs:
            return
        func, args = funcs[sim_type]

        worker = SimulationWorker(func, *args)
        worker.finished.connect(lambda r: self._on_sim_complete(sim_type, r))
        worker.error.connect(self._on_sim_error)
        worker.start()
        self.workers.append(worker)

    def _on_sim_complete(self, sim_type: str, result):
        self.progress_bar.setVisible(False)
        self.status_bar.showMessage("Simulation complete")
        self.status_label.setText("Ready")

        if sim_type == "all":
            self._handle_full_report(result)
        elif sim_type == "thermal" and isinstance(result, list):
            self._handle_thermal_result(result)
        else:
            self.results_text.append(f"\n=== {sim_type.upper()} ===\n{result}")

    def _handle_full_report(self, report):
        lines = ["Full Simulation Report", "=" * 50, ""]
        if hasattr(report, 'audio') and report.audio:
            lines.append(f"DF: {report.audio.damping_factor:.2f}")
            lines.append(f"THD: {report.audio.thd_percent:.4f}%")
        if hasattr(report, 'thermal_results') and report.thermal_results:
            lines.append(f"\nThermal ({len(report.thermal_results)} components):")
            for t in report.thermal_results[:5]:
                lines.append(f"  {t.component_ref}: {t.junction_temp_c:.1f}°C")
            self.thermal_data = {t.component_ref: t.junction_temp_c for t in report.thermal_results}
            self.stats_thermal.setText(f"{max(t.junction_temp_c for t in report.thermal_results):.1f}°C")
            self.view_3d.draw_thermal_map(self.board, report.thermal_results)
        if hasattr(report, 'crosstalk') and report.crosstalk:
            lines.append(f"\nCrosstalk: {len(report.crosstalk)} pairs")
        if hasattr(report, 'emi') and report.emi:
            lines.append(f"EMI: {report.emi.score}")
        if hasattr(report, 'recommendations') and report.recommendations:
            lines.append(f"\nRecommendations ({len(report.recommendations)}):")
            for r in report.recommendations[:5]:
                lines.append(f"  - {r}")
        self.results_text.setPlainText("\n".join(lines))

    def _handle_thermal_result(self, result):
        lines = ["Thermal Analysis", "=" * 50, ""]
        for t in result:
            lines.append(f"{t.component_ref}: {t.junction_temp_c:.1f}°C, P={t.power_dissipation_w:.2f}W")
        self.results_text.setPlainText("\n".join(lines))
        self.thermal_data = {t.component_ref: t.junction_temp_c for t in result}
        self.stats_thermal.setText(f"{max(t.junction_temp_c for t in result):.1f}°C")
        self.view_3d.draw_thermal_map(self.board, result)

    def _on_sim_error(self, msg: str):
        self.progress_bar.setVisible(False)
        self.status_bar.showMessage(f"Error: {msg}")
        QMessageBox.critical(self, "Error", msg)

    def run_routing(self):
        if not self.board:
            QMessageBox.warning(self, "Warning", "Open a PCB file first")
            return

        self.progress_bar.setVisible(True)
        self.status_bar.showMessage("Routing in progress...")
        self.status_label.setText("Routing...")
        self.route_status.setText("Routing...")

        options = PipelineOptions(
            place=self.cb_place.isChecked(),
            route=self.cb_route.isChecked(),
            attempts=self.attempts_spin.value(),
        )

        worker = RoutingWorker(self.board, options)
        worker.finished.connect(self._on_routing_complete)
        worker.error.connect(self._on_routing_error)
        worker.start()
        self.workers.append(worker)

    def _on_routing_complete(self, result):
        self.progress_bar.setVisible(False)
        self.status_bar.showMessage("Routing complete")
        self.status_label.setText("Ready")

        if result:
            self.current_result = result
            if hasattr(result, 'routing') and result.routing:
                routed = len(result.routing.routed_nets) if hasattr(result.routing, 'routed_nets') else 0
                self.stats_routed.setText(str(routed))
                self.route_status.setText(f"Routed: {routed}")
                if hasattr(result.routing, 'segments'):
                    self.segments = result.routing.segments
                self.view_3d.draw_board(self.board, self.segments,
                                      self.thermal_data if self.cb_show_thermal.isChecked() else None,
                                      self.cb_show_thermal.isChecked())
            if self.cb_sim_feedback.isChecked():
                self.run_simulation("all")
        else:
            self.route_status.setText("No result")
            QMessageBox.warning(self, "Routing", "No result returned")

    def _on_routing_error(self, msg: str):
        self.progress_bar.setVisible(False)
        self.status_bar.showMessage(f"Error: {msg}")
        self.status_label.setText("Error")
        self.route_status.setText("Error")
        QMessageBox.critical(self, "Routing Error", msg)

    def clear_routes(self):
        self.segments = []
        if self.board:
            self.view_3d.draw_board(self.board)
        self.stats_routed.setText("0")
        self.route_status.setText("Cleared")


def run():
    app = QApplication(sys.argv)
    app.setStyle('Fusion')
    window = MainWindow()
    window.show()
    sys.exit(app.exec())


if __name__ == "__main__":
    run()
