# Docs

## DAQ measurement pipeline — account `891377204778`
- **[system-design.md](system-design.md)** — the pipeline end-to-end (ingestion → Flink → resample
  → Iceberg → the `measurements_aggregate` view). Canonical, current.
- **[system-design-presentation.html](system-design-presentation.html)** — the same as a slide deck
  (open in a browser; arrow keys / click to navigate).
- Scenario walkthroughs and generated diagrams remain under
  `infra/daq/data_pipeline/docs/` (`data-ingestion-scenarios.*`, `*.png`, `generate_diagrams.py`).

## Hierarchy service — account `339712745226` (Rust)
- **[architecture.md](architecture.md)** — onion layout, the injected-closure repository
  pattern, storage, CQRS API, edge-driven permissions.
- **[hierarchy-and-sensors.md](hierarchy-and-sensors.md)** — node hierarchy (HN0–HN9), the
  per-company **type-graph schema (v2)**, and the sensor model.
- **[api.md](api.md)** — HTTP API reference.

The service is a Rust Lambda (arm64, `provided.al2023`) — `crates/model` (pure domain + logic +
repository adapters) and `crates/services/hierarchy` (the Lambda: routing, CQRS dispatch, maud
HTML fragments). The Astro frontend lives in `frontend/`.

Deploy procedures for every stack live in the repo-root **`CLAUDE.md`**.
