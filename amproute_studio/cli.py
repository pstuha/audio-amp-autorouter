"""Command line interface for AmpRoute Studio."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from .adapters.kicad_pcb import load_board, save_board
from .core.pipeline import PipelineOptions, run_pipeline
from .core.report import to_json, to_markdown
from .core.simulations import run_full_simulation


def _cmd_simulate(args) -> int:
    import os
    from .core.analysis_visuals import (
        analyze_thermal_coupling,
        analyze_thermal_gradient,
        generate_bode_plot,
        generate_df_heatmap,
        generate_loop_map,
        simulation_to_markdown,
    )
    from .core.report import to_json

    board = load_board(args.input)
    output_dir = Path(args.output).parent if args.output else Path(".")
    os.makedirs(output_dir, exist_ok=True)

    # route first so loop/IR/power simulations see real geometry
    result = run_pipeline(board, PipelineOptions(place=args.place, route=args.route, attempts=args.attempts))
    report = run_full_simulation(
        board,
        segments=result.routing.segments if args.route else None,
        jumpers=result.jumpers.jumpers if args.route else None,
        ambient_temp=args.ambient,
        load_impedance=args.load,
        generate_heatmap=args.thermal,
        output_dir=str(output_dir),
    )
    report.thermal_gradient = analyze_thermal_gradient(board, report.thermal_results)
    report.thermal_coupling = analyze_thermal_coupling(board, report.thermal_results)

    data = report.to_dict()
    data["thermal_gradient"] = report.thermal_gradient
    data["thermal_coupling"] = report.thermal_coupling
    for key, renderer, filename in (
        ("loop_map", generate_loop_map, "loop_map.png"),
        ("df_heatmap", generate_df_heatmap, "df_heatmap.png"),
        ("bode", generate_bode_plot, "bode.png"),
    ):
        try:
            path = os.path.join(str(output_dir), filename)
            if renderer is generate_loop_map:
                saved = renderer(board, report.current_loops, path)
            elif renderer is generate_df_heatmap:
                saved = renderer(board, report.audio.damping_factor, path)
            else:
                saved = renderer(report.stability, path)
            data[key] = saved or ""
        except Exception as exc:  # pragma: no cover - optional visuals
            data[key] = f"error: {exc}"

    out = Path(args.output or "simulation_report.json")
    out.write_text(json.dumps(data, indent=2, ensure_ascii=False), encoding="utf-8")
    md = out.with_suffix(".md")
    md.write_text(simulation_to_markdown(report), encoding="utf-8")
    print(json.dumps({"ok": True, "output": str(out), "markdown": str(md)}))
    return 0


def _cmd_plan(args) -> int:
    board = load_board(args.input)
    result = run_pipeline(board, PipelineOptions(place=True, route=False, attempts=args.attempts))
    out = Path(args.output or "plan.json")
    out.write_text(to_json(result), encoding="utf-8")
    print(json.dumps({"ok": result.ok, "output": str(out)}))
    return 0 if result.ok else 2


def _cmd_route(args) -> int:
    board = load_board(args.input)
    result = run_pipeline(board, PipelineOptions(place=args.place, route=True, attempts=args.attempts))
    save_board(board, args.output, result.routing.segments, result.jumpers.jumpers)
    report = Path(args.output).with_suffix(".report.json")
    report.write_text(to_json(result), encoding="utf-8")
    print(json.dumps({"ok": result.ok, "output": args.output, "report": str(report)}))
    return 0 if result.ok else 2


def _cmd_validate(args) -> int:
    board = load_board(args.input)
    result = run_pipeline(board, PipelineOptions(place=False, route=False))
    out = Path(args.output or "report.json")
    out.write_text(to_json(result), encoding="utf-8")
    print(json.dumps({"ok": result.ok, "output": str(out)}))
    return 0 if result.ok else 2


def _cmd_run(args) -> int:
    board = load_board(args.input)
    result = run_pipeline(board, PipelineOptions(place=True, route=True, attempts=args.attempts))
    save_board(board, args.output, result.routing.segments, result.jumpers.jumpers)
    report = Path(args.output).with_suffix(".report.json")
    report.write_text(to_json(result), encoding="utf-8")
    md = Path(args.output).with_suffix(".report.md")
    md.write_text(to_markdown(result), encoding="utf-8")
    print(json.dumps({"ok": result.ok, "output": args.output, "report": str(report)}))
    return 0 if result.ok else 2


def _cmd_mcp(args) -> int:
    try:
        from .adapters.mcp import serve
    except ImportError as exc:
        print(f"MCP dependencies missing: {exc}")
        return 1
    serve(host=args.host, port=args.port)
    return 0


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(prog="amprote", description="AmpRoute Studio")
    sub = parser.add_subparsers(dest="command", required=True)

    p = sub.add_parser("plan", help="placement only")
    p.add_argument("input")
    p.add_argument("-o", "--output")
    p.add_argument("--attempts", type=int, default=6)
    p.set_defaults(func=_cmd_plan)

    r = sub.add_parser("route", help="route existing board")
    r.add_argument("input")
    r.add_argument("-o", "--output", required=True)
    r.add_argument("--place", action="store_true")
    r.add_argument("--attempts", type=int, default=6)
    r.set_defaults(func=_cmd_route)

    v = sub.add_parser("validate", help="validate only")
    v.add_argument("input")
    v.add_argument("-o", "--output")
    v.set_defaults(func=_cmd_validate)

    run = sub.add_parser("run", help="full pipeline")
    run.add_argument("input")
    run.add_argument("-o", "--output", required=True)
    run.add_argument("--attempts", type=int, default=6)
    run.set_defaults(func=_cmd_run)

    m = sub.add_parser("mcp", help="start MCP server")
    m.add_argument("--host", default="127.0.0.1")
    m.add_argument("--port", type=int, default=8765)
    m.set_defaults(func=_cmd_mcp)

    s = sub.add_parser("simulate", help="run electrical/thermal/audio simulations")
    s.add_argument("input")
    s.add_argument("-o", "--output")
    s.add_argument("--place", action="store_true", help="place components first")
    s.add_argument("--no-route", dest="route", action="store_false", help="skip routing (use raw board geometry)")
    s.add_argument("--attempts", type=int, default=4)
    s.add_argument("--thermal", action="store_true", help="generate thermal heatmap")
    s.add_argument("--current-loops", action="store_true", help="analyze current loops")
    s.add_argument("--audio-params", action="store_true", help="predict audio parameters")
    s.add_argument("--ambient", type=float, default=25.0, help="ambient temperature (°C)")
    s.add_argument("--load", type=float, default=4.0, help="load impedance (Ω)")
    s.set_defaults(func=_cmd_simulate, route=True)

    args = parser.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
