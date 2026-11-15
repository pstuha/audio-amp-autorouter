"""Minimal MCP server exposing the pipeline to AI agents."""

from __future__ import annotations

from typing import Any

from ..core.pipeline import PipelineOptions, run_pipeline
from ..core.report import to_dict
from .kicad_pcb import load_board, save_board


def build_server(host: str = "127.0.0.1", port: int = 8765):
    try:
        from mcp.server.fastmcp import FastMCP
    except ImportError as exc:
        raise ImportError("Install amproute-studio[mcp] to use the MCP server") from exc

    mcp = FastMCP("amproute-studio", host=host, port=port)

    @mcp.tool()
    def amprote_run(input_path: str, output_path: str) -> dict[str, Any]:
        """Run the full pipeline and save the routed board."""
        board = load_board(input_path)
        result = run_pipeline(board, PipelineOptions())
        save_board(board, output_path, result.routing.segments, result.jumpers.jumpers)
        return to_dict(result)

    @mcp.tool()
    def amprote_validate(input_path: str) -> dict[str, Any]:
        """Validate a board without modifying it."""
        board = load_board(input_path)
        result = run_pipeline(board, PipelineOptions(place=False, route=False))
        return to_dict(result)

    @mcp.tool()
    def amprote_simulate(
        input_path: str,
        ambient_temp: float = 25.0,
        load_impedance: float = 4.0,
        generate_heatmap: bool = True,
    ) -> dict[str, Any]:
        """Run electrical, thermal and audio simulations on a board.

        Analyzes:
        - Current loops (EMI-critical paths)
        - IR drop on power distribution
        - Thermal dissipation and junction temperatures
        - Audio parameters (damping factor, THD, PSRR, slew rate)
        - Stability (phase margin, capacitive load)
        """
        from ..core.simulations import run_full_simulation

        board = load_board(input_path)
        report = run_full_simulation(
            board,
            ambient_temp=ambient_temp,
            load_impedance=load_impedance,
            generate_heatmap=generate_heatmap,
        )
        return report.to_dict()

    return mcp


def serve(host: str = "127.0.0.1", port: int = 8765) -> None:
    build_server(host, port).run(transport="sse")
