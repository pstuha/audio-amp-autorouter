"""Stage C: thermal gradient/coupling and PNG visualizations for simulations."""

from __future__ import annotations

import math

try:
    import numpy as np  # noqa: F401
    _HAS_NP = True
except Exception:  # pragma: no cover
    _HAS_NP = False


def analyze_thermal_gradient(board, thermal_results):
    """Estimate board surface temperature gradient from heat sources."""
    if not thermal_results:
        return {"max_gradient_c_per_mm": 0.0, "hottest_component": None, "hot_spots": [], "notes": ["no thermal data"]}
    rows = []
    for tr in thermal_results:
        comp = board.components.get(tr.component_ref)
        if comp is not None:
            rows.append((comp.x_mm, comp.y_mm, tr.junction_temp_c, tr.component_ref))
    if len(rows) < 2:
        return {"max_gradient_c_per_mm": 0.0, "hottest_component": None, "hot_spots": [], "notes": ["too few heat sources"]}
    max_grad = 0.0
    hottest = max(rows, key=lambda r: r[2])
    for x1, y1, t1, _ in rows:
        for x2, y2, t2, _ in rows:
            d = math.hypot(x2 - x1, y2 - y1)
            if d > 1e-3:
                max_grad = max(max_grad, abs(t2 - t1) / d)
    hot_spots = sorted(
        ({"ref": r, "x_mm": x, "y_mm": y, "temp_c": t} for x, y, t, r in rows if t > 60.0),
        key=lambda h: -h["temp_c"],
    )
    return {
        "max_gradient_c_per_mm": round(max_grad, 3),
        "hottest_component": hottest[3],
        "hot_spots": hot_spots,
        "notes": [],
    }


def analyze_thermal_coupling(board, thermal_results):
    """Pairwise thermal coupling between power components."""
    comps = [
        (tr.component_ref, tr.power_dissipation_w, board.components[tr.component_ref].x_mm, board.components[tr.component_ref].y_mm)
        for tr in thermal_results
        if tr.component_ref in board.components
    ]
    couplings = []
    for i in range(len(comps)):
        for j in range(i + 1, len(comps)):
            r1, p1, x1, y1 = comps[i]
            r2, p2, x2, y2 = comps[j]
            d = math.hypot(x2 - x1, y2 - y1)
            if d < 1e-3:
                continue
            coupling = (p1 * p2) / (d * d + 1.0)
            couplings.append({
                "pair": f"{r1} <-> {r2}",
                "distance_mm": round(d, 2),
                "coupling_metric": round(coupling, 4),
                "critical": d < 15.0 and (p1 > 1.0 or p2 > 1.0),
            })
    couplings.sort(key=lambda c: -c["coupling_metric"])
    return {"count": len(couplings), "critical_pairs": [c for c in couplings if c["critical"]], "all": couplings}


def _mpl():
    import matplotlib
    import matplotlib.pyplot as plt
    import matplotlib.patches as patches  # noqa: F401
    return plt, patches


def generate_loop_map(board, current_loops, output_path="loop_map.png"):
    """Render current-loop blocks over the board outline."""
    try:
        plt, patches_ = _mpl()
    except Exception:
        return ""
    fig, ax = plt.subplots(1, 1, figsize=(12, 8))
    if board.bounds_mm:
        bx1, by1, bx2, by2 = board.bounds_mm
        ax.add_patch(patches_.Rectangle((bx1, by1), bx2 - bx1, by2 - by1, fill=False, edgecolor="black", linewidth=2))
        ax.set_xlim(bx1 - 10, bx2 + 10)
        ax.set_ylim(by1 - 10, by2 + 10)
    for loop in current_loops:
        cx, cy = loop.centroid
        size = max(4.0, (loop.area_mm2 ** 0.5) / 2.0)
        colour = "#c0392b" if loop.critical else "#2e86c1"
        ax.add_patch(patches_.Rectangle((cx - size / 2, cy - size / 2), size, size, linewidth=1.5,
                                        edgecolor=colour, facecolor=colour, alpha=0.25))
        ax.annotate(f"{loop.net_name}\n{loop.area_mm2:.0f}mm2 {loop.inductance_nh:.0f}nH",
                    (cx, cy), ha="center", va="center", fontsize=7, color=colour)
    ax.set_aspect("equal")
    ax.set_title("Current loop map")
    ax.grid(True, alpha=0.3)
    plt.tight_layout()
    plt.savefig(output_path, dpi=150)
    plt.close()
    return output_path


def generate_df_heatmap(board, damping_factor, output_path="df_heatmap.png"):
    """Damping-factor quality heatmap. DF>50 excellent, 30-50 good, <30 poor."""
    try:
        plt, patches_ = _mpl()
    except Exception:
        return ""
    fig, ax = plt.subplots(1, 1, figsize=(12, 8))
    if board.bounds_mm:
        bx1, by1, bx2, by2 = board.bounds_mm
        ax.add_patch(patches_.Rectangle((bx1, by1), bx2 - bx1, by2 - by1, fill=False, edgecolor="black", linewidth=2))
    xs = [comp.x_mm for comp in board.components.values()]
    ys = [comp.y_mm for comp in board.components.values()]
    if xs:
        sc = ax.scatter(xs, ys, c=[damping_factor] * len(xs), cmap="RdYlGn", s=180, vmin=10, vmax=60)
        fig.colorbar(sc, ax=ax, label="Damping factor")
    ax.set_aspect("equal")
    ax.set_title("Damping-factor quality map")
    ax.grid(True, alpha=0.3)
    plt.tight_layout()
    plt.savefig(output_path, dpi=150)
    plt.close()
    return output_path


def generate_bode_plot(stability, output_path="bode.png"):
    """Render representative magnitude/phase plot from the stability estimate."""
    try:
        plt, _ = _mpl()
    except Exception:
        return ""
    freqs = np.logspace(0, 6, 300)
    f0 = max(1.0, min(1e6, stability.unity_gain_freq_mhz * 1e6))
    mag_db = 60.0 - 20 * np.log10(1 + freqs / (f0 * 0.1))
    phase = -190 * np.log10(1 + freqs / (f0 * 0.1)) * 0.7
    phase = np.maximum(phase, -270)
    fig, (ax1, ax2) = plt.subplots(2, 1, figsize=(9, 7))
    ax1.semilogx(freqs, mag_db, color="#2d6cdf")
    ax1.set_ylabel("Gain (dB)")
    ax1.axhline(0, color="grey", lw=0.8)
    ax1.grid(True, alpha=0.3)
    ax2.semilogx(freqs, phase, color="#c0392b")
    ax2.set_xlabel("Frequency (Hz)")
    ax2.set_ylabel("Phase (deg)")
    ax2.grid(True, alpha=0.3)
    ax2.annotate(f"PM={stability.phase_margin_deg:.0f}°  GM={stability.gain_margin_db:.0f}dB",
                 xy=(0.03, 0.08), xycoords="axes fraction", fontsize=10,
                 bbox=dict(boxstyle="round", fc="white", alpha=0.8))
    fig.suptitle("Estimated loop response (bode)")
    plt.tight_layout()
    plt.savefig(output_path, dpi=150)
    plt.close()
    return output_path


def simulation_to_markdown(report) -> str:
    """Render the simulation report as Markdown text."""
    lines = [
        "# AmpRoute Studio — simulation report",
        "",
        f"Board: **{getattr(report, 'board_name', 'unknown')}**",
        "",
        "## Current loops",
        f"- Count: {len(report.current_loops)}",
        f"- Critical: {len([c for c in report.current_loops if c.critical])}",
    ]
    for c in report.current_loops[:8]:
        lines.append(f"  - {c.net_name}: {c.area_mm2:.0f} mm², {c.inductance_nh:.1f} nH, "
                     f"{'CRITICAL' if c.critical else 'ok'}")
    lines += ["", "## Thermal", f"- Components analysed: {len(report.thermal_results)}"]
    hot = [t for t in report.thermal_results if t.junction_temp_c > 80]
    if hot:
        lines.append(f"- **Hot (Tj>80°C): {len(hot)}**")
    for t in report.thermal_results[:10]:
        lines.append(f"  - {t.component_ref}: {t.power_dissipation_w:.2f} W → Tj {t.junction_temp_c:.0f} °C")
    lines += ["", "## Audio predictions"]
    lines.append(f"- Damping factor: **{report.audio.damping_factor:.1f}**")
    lines.append(f"- THD @1kHz (estimate): {report.audio.thd_percent:.4f} %")
    lines.append(f"- Slew rate: {report.audio.slew_rate_v_us:.1f} V/µs")
    lines.append(f"- PSRR 100Hz/1kHz: {report.audio.psrr_pos_100hz:.0f}/{report.audio.psrr_pos_1khz:.0f} dB")
    lines += ["", "## Stability"]
    lines.append(f"- Phase margin: **{report.stability.phase_margin_deg:.1f}°**")
    lines.append(f"- Gain margin: {report.stability.gain_margin_db:.1f} dB")
    stbl = max(report.audio.stable_capacitive_load, default=0) * 1e6
    lines.append(f"- Capacitive load stable up to: {stbl:.0f} µF")
    lines.append(f"- **Stable: {'YES' if report.stability.stable else 'NO'}**")
    lines += ["", "## IR drop"]
    for net, drop in report.ir_drop.items():
        lines.append(f"  - {net}: {drop:.1f} mV")
    if report.recommendations:
        lines += ["", "## Recommendations"]
        lines.extend(f"- {r}" for r in report.recommendations)
    if report.warnings:
        lines += ["", "## Warnings"]
        lines.extend(f"- {w}" for w in report.warnings)
    return "\n".join(lines)
