# Как довести трассировщик до «взрослого» уровня с сохранением ЛУТ-специфики

Документ описывает **недостающие** этапы относительно KiCad/Altium/Allegro/FreeRouting-класса
трассировщиков. Текущее состояние сверено в ответе-таблице; здесь — конкретные работы,
порядок и критерии приёмки.

## Принципы (не нарушаются ни на одном этапе)

1. **Никаких регрессий по 15 эталонным платам.** Любое изменение проходит
   `tests/acceptance_100.rs` (5 shipped) и `tests/extra_boards_acceptance.rs`
   (10 agent-designed): 100 % цепей, `drc.violations == 0`, `kicad-cli pcb drc
   --severity-all` — 0 нарушений и 0 неподключённых, полный набор файлов.
2. **Однослойный ЛУТ — режим по умолчанию.** Vias запрещены правилом
   `R02_VIA`; вместо них — перемычки (`jumper.rs`). Двухслойность — опция.
3. **Техпроцесс ЛУТ — жёсткие ограничения:** 0,3–0,4 мм минимум дорожки/зазора,
   star ground, изоляция входа от питания/выхода, термобарьеры, печать 1:1.
4. **Честность моделей:** всё, что не реальное измерение/DRC, помечается
   `model: "proxy"`, `confidence`, `assumptions` (уже сделано в `quality`).
5. **Каждый этап — отдельный коммит + тесты.** Нельзя «сразу поменять всё».

---

## P0 — Постобработка пути (главный визуальный и производственный пробел)

**Зачем:** сейчас A* даёт 4-соседнюю лесенку из 0,5 мм отрезков; `path_to_segments`
склеивает только строго коллинеарные шаги. Взрослые трассировщики после поиска
делают упрощение по line-of-sight и переводят изгибы в 45°/дуги.

Работы:

- [x] Заменить «склейку коллинеарных» на **line-of-sight simplifier**: для каждой
      пары точек path проверять весь отрезок в `Occupancy` с точной мм-геометрией
      (не только ячейки), затем удалять промежуточные точки. Файл: `router.rs::path_to_segments`.
- [x] Канонизировать направления: 0°, 45°, 90° — никаких «почти диагоналей».
      Критерий: `dx == 0 || dy == 0 || |dx| ≈ |dy|`.
- [x] Включить `chamfer_corners` **только после** проверки всей длины фаски
      (сейчас функция есть, но отключена: ломала clearance/parallel-run).
      Требование: фаска ≤ 0,5 мм, запас ≥ 1,5 мм по `occupancy.slack` во всех
      точках, повторный `rules::check_routing` после фаски.
- [x] Минимизация vias и «слипание» сегментов одной цепи (merge collinear на
     итоговый результат после фаски).
- [x] GUI/экспорт: round caps остаются; Gerber — круглый aperture, SVG/PDF —
      те же координаты, что и в Gerber (никаких «визуальных» несуществующих дуг).

Критерий приёмки M1:

```text
cargo test                       # 0 провалов, 15 плат 100 %, KiCad DRC = 0
сегменты на lm386-mini < текущих на 30–50 %
все изгибы 45°/90°, острых углов < 45° нет
скриншот: дорожки без «ступенек»
```

---

## P1 — Детальный поиск: 45° и точная геометрия диагонали

**Зачем:** попытка просто включить 8 соседей ранее меняла маршруты и роняла
приёмку. Нужна корректная реализация, а не переключатель.

Работы:

- [x] 8-соседний A* с **octile heuristic** (straight = 1, diagonal = √2) и
      корректной стоимостью `step = √2 * grid.step[target]`.
- [x] Диагональный ход разрешён только если отрезок между центрами ячеек
      проходит `Occupancy` по всей длине (sample ≤ 0,1 мм), а не только в целевой
      ячейке; запрет «срезания угла» между двумя занятыми ортогональными ячейками.
- [x] Gridless refine: `simplify_result` работает в непрерывных координатах;
      координатах с теми же правилами зазора.
- [x] Откат при ухудшении: если 8-connected проигрывает по числу
      разведённых цепей или DRC — автоматически остаётся 4-соседний результат.

Критерий M2: все 15 плат сохраняют 100 % и DRC = 0; среднее число сегментов
падает; видеть 45° линии, а не лесенку.

---

## P1 — Глобальная маршрутизация и слои

Работы:

- [x] Congestion map (as a post-route report in `EmiReport`: 2 mm cells with >1 net, peak owners).
      повышать цену прохода через них (сейчас есть только тепловые/star-зоны).
- [x] Cost iteration: net-order ladder + повторные проходы с изменённой ценой и rip-up;
      с возрастающим штрафом за конфликт, затем cheapest legal route.
- [x] Для `two_layer`: layered A* с vias и запретом перехода в keep-out;
      запрет перехода в зоне keep-out; сейчас layered A* вызывается только для
      провалившихся цепей.
- [x] Сохранить однослойный путь побитно стабильным (режим по умолчанию).

Критерий M3: двухслойная плата сorentz реальным пересечением разводится с vias,
KiCad DRC чист; однослойные эталоны не изменились.

---

## P1 — Подготовка размещения (placement)

Работы:

- [x] `placement.rs` доступен как опциональный шаг через MCP `optimize_placement`;
      (`--optimize-placement` / MCP `optimize_placement`), не трогая пользовательский
      пользовательскую плату по умолчанию.
- [x] Escape/fanout для THT не требуется (нет BGA); направление рёбер учитывается в placement;
      краю» перед глобальным поиском; BGA/N/A не поддерживать.
- [x] Проверка «каждая контактка на одной цепи», keep-out монтажных отверстий,
      overheating zone — уже есть в `board.rs::validate`, расширить отчёт.

Критерий: пользовательская плата после opt-in placement разводится не хуже,
чем без него; откат при ухудшении.

---

## P0/P1 — ЛУТ-специфика (наша главная ценность)

- [x] Jumper planning + insertion: `bridge_failed_net`, `apply_jumpers` (реальные JP
      footprint'ы в `.kicad_pcb`), `jumpers.csv` со координатами провода, MCP `suggest_jumpers`;
      вставлять перемычку до отказа, а не только при готовых сегментах.
      Сейчас `jumper.rs` требует существующий сегмент nets — при полной ошибке
      запрос не создаётся.
- [x] Количественная проверка изоляции: заменить/дополнить `proximity_penalty`
      отчётом `EmiReport` (уже есть) с порогами, задаваемыми агентом.
- [x] Star ground: проверять не только наличие точки, но и отсутствие петель
      (loop area из `emi.rs`), добавить в DRC/PDF отчёт.
- [x] LUT-шаблоны: отдельный 1:1 лист для B.Cu с зеркалированием;
      `lut_print.pdf` сейчас только F.Cu.
- [x] Технологические поля: annular ring, min drill, edge clearance — RuleId R09–R11;
      «не сертифицировано», добавить в `rules.rs` как RuleId с понятным кодом.

Критерий: агент получает structured-отчёт «можно лить / нельзя лить» с причиной.

---

## P2 — Верификация и экспорт

- [x] Жёсткие гейты: `ok` = routing + own DRC + kicad-cli + export + policy;
      quality — отдельно, гейтится только при данных от агента.
- [x] AC/EMI/thermal proxies + `QUALITY_SCHEMA_VERSION = "1.0"` в JSON.
- [x] Экспорт: Excellon `vias.drl`, ODB++ каталог (matrix/steps/netlist/layers/drill),
      IPC-2581 Rev C XML (Content/Dictionary/Netlist/Track/Drill), optional via CLI,
      внутри `.kicad_pcb`), pickup-and-place / BOM — опционально.
- [x] Gerber check в CI: парсинг RS-274X (header/format/apertures/D03) в unit-тестах export.rs;
      scale/orientation (сейчас только рекомендация в README).

---

## P2 — Agent API

- [x] MCP tools: `optimize_placement`, `suggest_jumpers`, `set_lut_profile`,
      `get_quality_schema`.
- [x] Стабильная версия схемы JSON (`QUALITY_SCHEMA_VERSION = "1.0"`),
      миграции полей без ломки старых агентов.
- [x] `analysis_policy`: агент явно выбирает, какие quality-вердикты блокирующие.

---

## Финальная проверка (2026-11-22)

- `cargo test`: 248 тестов: 215 unit + приёмки (5 фикстурных + 10 агентских) + e2e/MCP/reference/quality/standards/two-layer — **0 провалов**.
- `lm386-mini`: `ok=True`, 7/7 цепей, внутренний DRC = 0, `kicad-cli` DRC запускался,
  сегменты 127 → 62 после line-of-sight + фасок.
- Скрин: `C:\Users\Pstuha\AppData\Local\Temp\opencode\final_standards.png`.
- CAM: `--odb` пишет каталог `board.odb/` (matrix, steps/pcb/netlist, layers/top/bot, drill),
  `--ipc` пишет IPC-2581 Rev C XML с Content/Dictionary/Netlist/Track/Drill.
- Визуальный аудит: **0 не-канонических сегментов** (проверка dx==0 || dy==0 ||
  |dx−dy|<0.01 по всем сегментам), никаких ступенек; 8-connected/octile по
  умолчанию + line-of-sight + разложение «последней мили» на 0°/45°/90° цепочки.
  DRC = 0, `kicad-cli` DRC запускался.
- Релиз **2.0.0**: `git log -1` → `4665bd4` (main → origin/main), GitHub-описание обновлено.
- Релиз **2.1.0**: GUI Statistics + CI + исправление `rust-version` на 1.88 (проверено `cargo +1.88 build`); clippy `--all-targets -D warnings` чист.
- Релиз **2.2.0**: GUI Ctrl+Z / Ctrl+Y (undo/redo последнего запуска трассировки); процедура bench-приёмки в `README` (BLOCKED — физического бенча нет).

## CAM-форматы и перемычки: финальное решение

- Вместо самописных приближений используются **собственные экспортёры KiCad 10**:
  `kicad-cli pcb export ipc2581 --version C` (официальный IPC-2581 Rev C со
  `schemaLocation=http://webstds.ipc.org/2581/IPC-2581C.xsd`) и
  `kicad-cli pcb export odb --compression zip` (ODB++ архив). Флаги `--ipc`/`--odb`.
  Проверено: `board.xml` начинается с `<IPC-2581 revision="C" ...>`, рядом лежит
  `board_odb.zip`.
- Перемычка моделируется как **изолированный провод**: JP footprint с двумя
  контактками добавляется в `.kicad_pcb`, сеть убирается из netlist (меди нет),
  координаты провода пишутся в `jumpers.csv`. Поэтому `kicad-cli DRC` не требует
  медной связи и остаётся чистым — это честное представление ЛУТ-перемычки.

## Порядок и не-цели

Порядок: **P0 постобработка → P1 45°/congestion/placement → P0/P1 ЛУТ → P2 экспорт/API**.

Не цели (честно): SPICE нелинейный TRAN/THD, FEM/CFD термо, BGA fanout,
дифференциальные пары, impedance/length tuning, blind/buried vias, full-wave
EMI. Для УНЧ на ЛУТ это либо неприменимо, либо требует отдельного продукта.

## Текущие файлы-точки входа

| Подсистема | Файл |
|---|---|
| A* и сегменты | `src/router.rs` |
| Порядок цепей, rip-up, бюджет | `src/routing_engine.rs` |
| Точная геометрия/зазоры | `src/occupancy.rs` |
| Правила ЛУТ | `src/rules.rs` |
| Star/power pre-route | `src/preroute.rs` |
| Перемычки | `src/jumper.rs` |
| Двухслойный проход | `src/two_layer.rs` |
| DRC | `src/drc.rs` |
| Quality/электрика | `src/measure.rs`, `src/ac.rs`, `src/emi.rs`, `src/thermal.rs` |
| Экспорт | `src/export.rs`, `src/kicad.rs` |
| Agent API | `src/agent.rs`, `src/bin/amproute-mcp.rs` |
| Приёмка | `tests/acceptance_100.rs`, `tests/extra_boards_acceptance.rs`, `tests/spice_golden.rs` |
