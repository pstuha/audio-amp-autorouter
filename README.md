# AmpRoute Studio

[![CI](https://github.com/pstuha/audio-amp-autorouter/actions/workflows/ci.yml/badge.svg)](https://github.com/pstuha/audio-amp-autorouter/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

**AmpRoute Studio** — автотрассировщик и симулятор для **однослойных и двухсторонних плат УНЧ**
под метод изготовления **ЛУТ** (лазерно-утюжная технология). Полноценное приложение: CLI, GUI (PySide6), MCP-сервер для ИИ-агентов.

## Возможности

### Трассировка
- **A* routing** с **rip-up/reroute** — минимум перемычек
- **Однослойные платы** (F.Cu) — классический ЛУТ
- **Двухсторонние платы** (F.Cu + B.Cu + vias)
- Оптимизация перемычек

### Симуляции
- **Thermal** — junction temperature, thermal map
- **DF** — Damping Factor prediction  
- **THD** — distortion estimation
- **PSRR** — power supply rejection
- **Crosstalk** — наводки между дорожками
- **EMI** — излучение и EMC
- **Power Bus** — распределение питания, пульсации
- **Current Loops** — площадь петель, индуктивность
- **Phase Margin** — устойчивость

### Интерфейсы
- **CLI** — `amprote route|simulate|validate|run|mcp`
- **GUI** (PySide6) — визуализация платы
- **MCP** — для Claude, OpenCode и других ИИ агентов

## Установка

```bash
pip install -e ".[all]"
```

### Сборка exe
```bash
python build.py
# или
pyinstaller amproute-studio.spec --clean
```
exe: `dist/amproute-studio.exe`

## CLI

```bash
amprote run input.kicad_pcb -o output.kicad_pcb      # полный пайплайн
amprote route input.kicad_pcb -o output.kicad_pcb    # только трассировка
amprote simulate input.kicad_pcb -o sim.json --thermal  # симуляции
amprote validate input.kicad_pcb -o report.json       # валидация
amprote mcp --port 8765                              # MCP сервер
```

## Python API

```python
from amproute_studio.adapters.kicad_pcb import load_board
from amproute_studio.core.simulations import run_full_simulation

board = load_board("input.kicad_pcb")
report = run_full_simulation(board)

print(f"DF: {report.audio.damping_factor}")
print(f"EMI: {report.emi.score}")
print(f"Crosstalk pairs: {len(report.crosstalk)}")
print(f"Thermal: {len(report.thermal_results)} components")
```

## ЛУТ рекомендации

| Параметр | Значение |
|----------|----------|
| Ширина дорожки | ≥0.4 мм |
| Зазор | ≥0.3 мм |
| Двухсторонние | B.Cu = ground plane |

## Структура

```
amproute_studio/
├── core/           # routing, placement, rules, simulations
├── adapters/      # KiCad, CLI, MCP
├── gui/           # PySide6
└── tests/
```

## Лицензия

MIT
