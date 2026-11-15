"""kicad-cli wrapper for DRC and export."""

from __future__ import annotations

import json
import subprocess
import tempfile
from pathlib import Path
from typing import Optional


def find_cli() -> Optional[Path]:
    candidates = [
        Path(r"C:\Program Files\KiCad\10.0\bin\kicad-cli.exe"),
        Path(r"C:\Program Files (x86)\KiCad\10.0\bin\kicad-cli.exe"),
        Path.home() / "AppData/Local/Programs/KiCad/10.0/bin/kicad-cli.exe",
    ]
    for path in candidates:
        if path.exists():
            return path
    return None


def run_drc(pcb_path: str | Path) -> dict:
    cli = find_cli()
    if cli is None:
        return {"ok": False, "error": "kicad-cli not found"}
    with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as handle:
        report = Path(handle.name)
    try:
        subprocess.run(
            [str(cli), "pcb", "drc", "--format", "json", "--severity-all", "-o", str(report), str(pcb_path)],
            capture_output=True, text=True, timeout=300, check=False,
        )
        data = json.loads(report.read_text(encoding="utf-8"))
        violations = data.get("violations", [])
        return {
            "ok": not violations,
            "violations": violations,
            "unconnected": len(data.get("unconnected_items", [])),
        }
    except Exception as exc:
        return {"ok": False, "error": str(exc)}
    finally:
        report.unlink(missing_ok=True)
