#!/usr/bin/env bash
# Regenerate the diagram PNGs and the Word (.docx) version of the migration doc.
# Usage: bash build_migration_docx.sh
set -euo pipefail
cd "$(dirname "$0")"

# 1. diagrams -> img/*.png
uv run --with matplotlib python3 build_migration_figs.py

# 2. markdown (+ embedded figures) -> docx, with a table of contents
uv run --with pypandoc-binary python3 - <<'PY'
import pypandoc
pypandoc.convert_file(
    "counter-measurement-migration.md", "docx",
    outputfile="counter-measurement-migration.docx",
    extra_args=["--resource-path=.", "--toc", "--toc-depth=2"],
)
print("built counter-measurement-migration.docx")
PY
