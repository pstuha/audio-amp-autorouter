# AmpRoute Studio

[![CI](https://github.com/pstuha/amproute-studio/actions/workflows/ci.yml/badge.svg)](https://github.com/pstuha/amproute-studio/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

**AmpRoute Studio** — авторазмещение, автотрассировка и симуляции **однослойных плат УНЧ**
под метод изготовления **ЛУТ** (лазерно-утюжная технология). Полноценное приложение:
CLI, GUI (PySide6), MCP-сервер для ИИ-агентов и JSON API.

## Возможности

- **Чтение** `.kicad_pcb` (KiCad 8/9/10) через нормализованную модель платы.
- **Размещение** с учётом правил УНЧ: вход/выход далеко друг от друга, короткая ООС,
  развязка у силовых выводов, компактные токовые петли.
- **Трассировка** A* + **rip-up/reroute** с итеративным улучшением: меньше перемычек,
  короче дорожки, учёт классов цепей.
- **Перемычки** (wire jumpers) — штатное решение однослойных ЛУТ-плат.
- **Симуляции**: токовые петли, IR-падения, тепловой расчёт (Tj), предсказание
  DF/THD/PSRR/slew-rate, устойчивость (phase/gain margin, ёмкостная нагрузка).
- **Визуализация**: тепловая карта, карты токовых петель, графики bode (PNG/SVG).
- **Валидация** геометрии, электрики и технологичности ЛУТ.
- **Экспорт**: `.kicad_pcb`, отчёты JSON/Markdown, Gerber (через kicad-cli / CI).

## Установка

```bash
pip install -e ".[all]"
# или по частям:
pip install -e ".[dev]"      # разработка
pip install -e ".[gui]"      # GUI
pip install -e ".[sim]"      # визуализация (matplotlib)
pip install -e ".[mcp]"      # MCP-сервер
```

## CLI

```bash
amprote run input.kicad_pcb -o output.kicad_pcb          # полный пайплайн
amprote plan input.kicad_pcb -o plan.json                # только размещение
amprote route input.kicad_pcb -o output.kicad_pcb        # только трассировка
amprote validate input.kicad_pcb -o report.json          # только валидация
amprote simulate input.kicad_pcb -o sim.json --thermal   # симуляции
amprote export input.kicad_pcb -o out.kicad_pcb          # экспорт
amprote mcp --port 8765                                  # MCP-сервер
```

## Python API

```python
from amproute_studio.adapters.kicad_pcb import load_board
from amproute_studio.core.pipeline import PipelineOptions, run_pipeline
from amproute_studio.core.simulations import run_full_simulation

board = load_board("input.kicad_pcb")
result = run_pipeline(board, PipelineOptions())
sim = run_full_simulation(board, result.routing.segments, result.jumpers.jumpers)
print(sim.to_dict())
```

## Структура

```
amproute_studio/
├── core/          # чистый Python: модели, правила, размещение, трассировка, симуляции
├── adapters/      # KiCad (.kicad_pcb), CLI, MCP
├── gui/           # PySide6
└── cli.py
tests/
docs/              # API reference, LUT guide
examples/          # пример платы УНЧ
```

## Правила УНЧ

- Вход и выход — на разных краях, далеко друг от друга.
- Обратная связь — короткая и рядом.
- Развязка — у силовых выводов.
- Силовые петли — компактные.
- Перемычки — минимум, технологичные.

## Правила ЛУТ

- Один слой (F.Cu / B.Cu).
- Зазор ≥ 0.3 мм, ширина ≥ 0.4 мм (пит. ≥ 1.2 мм).
- Углы 45° или ортогональные.
- Нет узких шеек и островков меди.

## Лицензия

MIT
