#!/usr/bin/env bash
# Regenerate the diagram PNGs and the Word (.docx) version of the migration doc.
# Post-processes the docx to landscape A4 + narrow margins AND rescales every
# table to the full text width (pandoc hardcodes tables to ~5.5", leaving them
# at half-width on a landscape page).
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
print("converted")
PY

# 3. landscape A4 + narrow margins + rescale tables to fill the text width
uv run --with python-docx python3 - <<'PY'
from docx import Document
from docx.enum.section import WD_ORIENT
from docx.shared import Cm
from docx.oxml.ns import qn
d = Document("counter-measurement-migration.docx")
s = d.sections[0]
s.orientation = WD_ORIENT.LANDSCAPE
s.page_width, s.page_height = Cm(29.7), Cm(21.0)          # A4 landscape
s.left_margin = s.right_margin = Cm(1.4)
s.top_margin = s.bottom_margin = Cm(1.3)
text_tw = int((s.page_width - s.left_margin - s.right_margin) / 635)  # EMU->twips
W = qn("w:w"); TYPE = qn("w:type")
for tbl in d.tables:
    cols = tbl._tbl.tblGrid.findall(qn("w:gridCol"))
    cur = sum(int(c.get(W) or 0) for c in cols)
    if cur <= 0:
        continue
    scale = text_tw / cur
    for c in cols:
        c.set(W, str(int(int(c.get(W)) * scale)))
    for tc in tbl._tbl.iter(qn("w:tc")):           # each cell once (no merges here)
        tcPr = tc.find(qn("w:tcPr"))
        tcW = tcPr.find(qn("w:tcW")) if tcPr is not None else None
        if tcW is not None and tcW.get(TYPE) == "dxa":
            tcW.set(W, str(int(int(tcW.get(W)) * scale)))
d.save("counter-measurement-migration.docx")
print(f"landscape A4, 1.4cm margins, tables scaled to {round(text_tw/1440,2)} in")
PY

echo "built counter-measurement-migration.docx"
