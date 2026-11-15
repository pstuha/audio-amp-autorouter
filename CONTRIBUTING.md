# Contributing

## Development setup

```sh
git clone https://github.com/pstuha/audio-amp-autorouter.git
cd audio-amp-autorouter
python -m venv .venv
# Windows: .venv\Scripts\activate
# Linux/macOS: source .venv/bin/activate
python -m pip install -e ".[dev]"
```

Optional extras:

```sh
pip install -e ".[gui]"   # PySide6 desktop app
pip install -e ".[mcp]"   # MCP server for AI agents
```

## Checks

```sh
python -m pytest    # tests
ruff check .        # lint
```

CI runs both on every push to `main` and on every pull request.

## Repository layout

- `amproute_studio/` — the application (core, adapters, GUI, CLI).
- `examples/` — sample boards used by tests.
- `tests/` — test suite.
- `legacy/` — archived KiCad 10 plugin, kept for reference only.

## Pull requests

- Keep PRs focused: one logical change per PR.
- Describe *why*, not only *what*. Placement/routing changes must include the
  before/after report (`amprote run ...`).
- Never commit generated outputs (`examples/*_out.kicad_pcb`, `*.report.json`,
  `*.report.md`, `__pycache__/`, `.venv/`).
- By submitting a PR you agree that your contribution is licensed under MIT.

## Reporting bugs

Use the issue templates. Attach the JSON report produced by `amprote run`
(`*.report.json`) and the board file if it can be shared.
