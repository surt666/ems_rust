# Docs

## DAQ measurement pipeline — account `891377204778`
- **[system-design.md](system-design.md)** — the pipeline end-to-end (ingestion → Flink → resample
  → Iceberg → the `measurements_aggregate` view). Canonical, current.
- **[system-design-presentation.html](system-design-presentation.html)** — the same as a slide deck
  (open in a browser; arrow keys / click to navigate).
- Scenario walkthroughs and generated diagrams remain under
  `infra/daq/data_pipeline/docs/` (`data-ingestion-scenarios.*`, `*.png`, `generate_diagrams.py`).

## Hierarchy service — account `339712745226` (OCaml)
- **[architecture.md](architecture.md)** — onion architecture, effects surface, storage, CQRS API,
  permissions.
- **[hierarchy-and-sensors.md](hierarchy-and-sensors.md)** — node hierarchy (HN0–HN9) + sensor model.
- **[api.md](api.md)** — HTTP API reference.

Deploy procedures for every stack live in the repo-root **`CLAUDE.md`**.
