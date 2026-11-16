"""3D Visualization for AmpRoute Studio using matplotlib."""

from __future__ import annotations

from typing import Dict, List, Optional, Tuple
import math

try:
    import matplotlib
    matplotlib.use("Qt5Agg")
    from mpl_toolkits.mplot3d import Axes3D, art3d
    from mpl_toolkits.mplot3d.art3d import Poly3DCollection, Line3DCollection
    import matplotlib.pyplot as plt
    from matplotlib.patches import FancyArrowPatch
    from matplotlib.colors import LinearSegmentedColormap
    import numpy as np
except ImportError:
    raise SystemExit("matplotlib required: pip install matplotlib")

from ..core.models import Board, RouteSegment, Via
from ..core.simulations import SimulationReport


class BoardVisualizer3D:
    """Interactive 3D visualization of PCB with simulation overlays."""

    def __init__(self, board: Optional[Board] = None):
        self.board = board
        self.fig = None
        self.ax = None
        self.thermal_data: Dict[str, float] = {}
        self.crosstalk_data: Dict[Tuple[str, str], float] = {}
        self.emi_data: Dict[str, float] = {}
        self.power_bus_data: Dict[str, float] = {}

    def set_board(self, board: Board):
        """Set the board to visualize."""
        self.board = board

    def set_thermal_data(self, data: Dict[str, float]):
        """Set thermal simulation data: component_ref -> temperature_C."""
        self.thermal_data = data

    def set_crosstalk_data(self, data: Dict[Tuple[str, str], float]):
        """Set crosstalk data: (net1, net2) -> crosstalk_dB."""
        self.crosstalk_data = data

    def set_emi_data(self, data: Dict[str, float]):
        """Set EMI data: net_name -> emi_score."""
        self.emi_data = data

    def set_power_bus_data(self, data: Dict[str, float]):
        """Set power bus data: rail_name -> impedance_mohm."""
        self.power_bus_data = data

    def _get_thermal_color(self, temp_c: float, min_temp: float = 30, max_temp: float = 120) -> Tuple[float, float, float]:
        """Get color for temperature (red-yellow-green gradient)."""
        norm = min(1.0, max(0.0, (temp_c - min_temp) / (max_temp - min_temp)))
        if norm < 0.5:
            return (1.0, norm * 2, 0.0)
        else:
            return (1.0 - (norm - 0.5) * 2, 1.0, 0.0)

    def _draw_board_outline(self, ax: Axes3D):
        """Draw board outline as a box."""
        if not self.board or not self.board.bounds_mm:
            return

        minx, miny, maxx, maxy = self.board.bounds_mm
        z = 0

        outline = [
            [minx, miny, z],
            [maxx, miny, z],
            [maxx, maxy, z],
            [minx, maxy, z],
            [minx, miny, z],
        ]
        xs, ys, zs = zip(*outline)
        ax.plot(xs, ys, zs, 'k-', linewidth=3)

        verts = [
            [minx, miny, 0],
            [maxx, miny, 0],
            [maxx, maxy, 0],
            [minx, maxy, 0],
        ]

        width = maxx - minx
        depth = maxy - miny
        verts_depth = [
            [minx, miny, 0.3],
            [maxx, miny, 0.3],
            [maxx, maxy, 0.3],
            [minx, maxy, 0.3],
        ]

        for verts in [[minx, miny, 0], [maxx, miny, 0], [maxx, maxy, 0], [minx, maxy, 0]]:
            pass

        quad = Poly3DCollection([
            [minx, miny, 0], [maxx, miny, 0], [maxx, maxy, 0], [minx, maxy, 0]
        ], alpha=0.3, facecolor='#2a2a2a', edgecolor='#444444', linewidth=1)
        ax.add_collection3d(quad)

    def _draw_components(self, ax: Axes3D):
        """Draw components as 3D boxes."""
        if not self.board:
            return

        for ref, comp in self.board.components.items():
            x, y = comp.x_mm, comp.y_mm
            w, h = getattr(comp, 'width_mm', 5), getattr(comp, 'height_mm', 5)
            z_bottom = 0
            z_top = 0.4

            temp_c = self.thermal_data.get(ref, None)
            if temp_c is not None:
                color = self._get_thermal_color(temp_c)
            else:
                color = '#4488ff'

            for z in [z_bottom, z_top]:
                corners = [
                    [x - w/2, y - h/2, z],
                    [x + w/2, y - h/2, z],
                    [x + w/2, y + h/2, z],
                    [x - w/2, y + h/2, z],
                ]
                if z == z_bottom:
                    ax.plot([c[0] for c in corners] + [corners[0][0]],
                           [c[1] for c in corners] + [corners[0][1]],
                           [z]*5, color='#666666', linewidth=0.5)
                else:
                    verts = corners
                    quad = Poly3DCollection([verts], alpha=0.9, facecolor=matplotlib.colors.to_hex(color), edgecolor='#ffffff', linewidth=0.5)
                    ax.add_collection3d(quad)

            for i in range(4):
                ax.plot([corners[i][0], corners[i][0]],
                       [corners[i][1], corners[i][1]],
                       [z_bottom, z_top], color='#888888', linewidth=0.5)

            ax.text(x, y, z_top + 1, ref, fontsize=6, ha='center', color='white')

    def _draw_tracks(self, ax: Axes3D, segments: List[RouteSegment]):
        """Draw routed tracks as 3D lines."""
        if not segments:
            return

        colors = {
            'power': '#ff4444',
            'ground': '#44ff44',
            'signal': '#4488ff',
            'feedback': '#ffaa00',
            'high_current': '#ff8800',
            'control': '#aa44ff',
            'default': '#88aaff',
        }

        for seg in segments:
            layer = seg.layer if hasattr(seg, 'layer') else 'F.Cu'
            z = 0.2 if layer == 'B.Cu' else 0.1

            net_class = getattr(seg, 'net_class', 'default')
            color = colors.get(net_class, colors['default'])

            if seg.x1 == seg.x2:
                ax.plot([seg.x1, seg.x2], [seg.y1, seg.y2], [z]*2,
                       color=color, linewidth=2, alpha=0.9)
            elif seg.y1 == seg.y2:
                ax.plot([seg.x1, seg.x2], [seg.y1, seg.y2], [z]*2,
                       color=color, linewidth=2, alpha=0.9)
            else:
                ax.plot([seg.x1, seg.x2], [seg.y1, seg.y2], [z]*2,
                       color=color, linewidth=2, alpha=0.9)

    def _draw_vias(self, ax: Axes3D, vias: List[Via]):
        """Draw vias as 3D cylinders."""
        from mpl_toolkits.mplot3d import art3d

        for via in vias:
            x, y = via.x, via.y
            z_bottom = 0.1
            z_top = 0.3

            theta = np.linspace(0, 2*np.pi, 16)
            r = 1.0

            verts = [[x + r*np.cos(t), y + r*np.sin(t), z] for z in [z_bottom, z_top] for t in theta]

            cylinder = Poly3DCollection([verts[:16]], alpha=0.8, facecolor='#ffcc00', edgecolor='#ffaa00')
            ax.add_collection3d(cylinder)

    def create_figure(self) -> Tuple:
        """Create the 3D figure."""
        self.fig = plt.figure(figsize=(12, 9), facecolor='#1a1a2e')
        self.ax = self.fig.add_subplot(111, projection='3d', facecolor='#1a1a2e')

        self.ax.set_facecolor('#1a1a2e')

        self._draw_board_outline(self.ax)

        if self.board:
            self._draw_components(self.ax)

            if hasattr(self.board, 'segments'):
                self._draw_tracks(self.ax, self.board.segments)

            if hasattr(self.board, 'vias'):
                self._draw_vias(self.ax, self.board.vias)

        self.ax.set_xlabel('X (mm)', color='white', fontsize=10)
        self.ax.set_ylabel('Y (mm)', color='white', fontsize=10)
        self.ax.set_zlabel('Z (mm)', color='white', fontsize=10)

        self.ax.tick_params(colors='white')
        self.ax.xaxis.pane.fill = False
        self.ax.yaxis.pane.fill = False
        self.ax.zaxis.pane.fill = False
        self.ax.xaxis.pane.set_edgecolor('#333333')
        self.ax.yaxis.pane.set_edgecolor('#333333')
        self.ax.zaxis.pane.set_edgecolor('#333333')

        self.ax.set_title('AmpRoute Studio - 3D Board View', color='white', fontsize=14, pad=20)

        self.ax.view_init(elev=25, azim=45)

        return self.fig, self.ax

    def add_thermal_overlay(self, segments: List[RouteSegment]):
        """Color tracks based on thermal simulation data."""
        if not self.thermal_data or not segments:
            return

        for seg in segments:
            pass

    def render(self):
        """Render the visualization."""
        if self.fig:
            plt.show(block=False)

    def update_view(self, elev: float = None, azim: float = None):
        """Update the 3D view angle."""
        if self.ax:
            if elev is not None:
                self.ax.view_init(elev=elev, azim=azim if azim is not None else self.ax.azim)
            elif azim is not None:
                self.ax.view_init(elev=self.ax.elev, azim=azim)

    def close(self):
        """Close the visualization."""
        if self.fig:
            plt.close(self.fig)
            self.fig = None
            self.ax = None


class SimulationOverlay:
    """Draws simulation result overlays on the 3D view."""

    def __init__(self, ax: Axes3D):
        self.ax = ax
        self.overlays = []

    def add_thermal_heatmap(self, board: Board, thermal_results: List, segments: List[RouteSegment]):
        """Add thermal heatmap overlay to tracks."""
        if not thermal_results:
            return

        colors = ['#00ff00', '#44ff00', '#88ff00', '#ccff00', '#ffff00', '#ffcc00', '#ff8800', '#ff4400', '#ff0000']

        return colors

    def add_crosstalk_heatmap(self, crosstalk_pairs: List):
        """Add crosstalk visualization as colored zones between affected tracks."""
        pass

    def add_emi_radiation(self, emi_sources: Dict):
        """Visualize EMI sources as radiating arcs."""
        pass


def create_board_viewer(board: Optional[Board] = None) -> BoardVisualizer3D:
    """Create a new board visualizer."""
    return BoardVisualizer3D(board)


if __name__ == "__main__":
    import sys
    if len(sys.argv) > 1:
        from amproute_studio.adapters.kicad_pcb import load_board
        board = load_board(sys.argv[1])
        viz = BoardVisualizer3D(board)
        viz.create_figure()
        viz.render()
        plt.show()
