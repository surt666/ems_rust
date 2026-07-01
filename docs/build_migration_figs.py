#!/usr/bin/env python3
"""Render diagram PNGs for the counter-measurement migration doc.
Run via: uv run --with matplotlib python3 build_migration_figs.py
Outputs into docs/img/.
"""
# pyright: reportMissingImports=false
import os
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.patches import FancyBboxPatch, FancyArrowPatch

NAVY, BLUE, TEAL, GREEN = "#1F3A4D", "#2E6E8E", "#3A8FA0", "#3B7A57"
AMBER, GRAY, LGRAY, LIGHT, INK, WHITE = "#B07D2A", "#8A8A8A", "#DDE3E7", "#EEF2F4", "#22303A", "#FFFFFF"
FONT = "DejaVu Sans"
OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "img")
os.makedirs(OUT, exist_ok=True)


def newfig(w, h):
    fig, ax = plt.subplots(figsize=(w, h), dpi=150)
    ax.set_xlim(0, w * 10)
    ax.set_ylim(0, h * 10)
    ax.axis("off")
    fig.subplots_adjust(left=0.01, right=0.99, top=0.99, bottom=0.01)
    return fig, ax


def box(ax, x, y, w, h, text, fc=LIGHT, ec=LGRAY, tc=INK, fs=10, bold=True, sub=None, subc=None, subfs=None):
    ax.add_patch(FancyBboxPatch((x, y), w, h, boxstyle="round,pad=0.1,rounding_size=1.0",
                                linewidth=1.1, edgecolor=ec, facecolor=fc))
    cy = y + h / 2 + (1.3 if sub else 0)
    ax.text(x + w / 2, cy, text, ha="center", va="center", fontsize=fs, color=tc,
            fontweight="bold" if bold else "normal", family=FONT)
    if sub:
        ax.text(x + w / 2, y + h / 2 - 1.5, sub, ha="center", va="center",
                fontsize=subfs or (fs - 2.5), color=subc or GRAY, family=FONT)


def arrow(ax, x1, y1, x2, y2, color=GRAY, lw=1.7):
    ax.add_patch(FancyArrowPatch((x1, y1), (x2, y2), arrowstyle="-|>", mutation_scale=13,
                                 linewidth=lw, color=color, shrinkA=1, shrinkB=1))


def title(ax, x, y, text, fs=13, color=INK):
    ax.text(x, y, text, ha="center", va="center", fontsize=fs, color=color, fontweight="bold", family=FONT)


def save(fig, name):
    p = os.path.join(OUT, name)
    fig.savefig(p, dpi=150, bbox_inches="tight", pad_inches=0.12, facecolor=WHITE)
    plt.close(fig)
    print("wrote", p)


# ---------- 1. current vs target ----------
fig, ax = newfig(12, 6)
ax.plot([60, 60], [4, 56], color=LGRAY, lw=1, ls="--")
title(ax, 32, 57.5, "Today — services coupled around measurements", 11, INK)
title(ax, 90, 57.5, "Target — read from one/two services", 11, GREEN)
# current
box(ax, 22, 49, 20, 4.5, "Frontend", fc=NAVY, tc=WHITE, ec=NAVY)
arrow(ax, 32, 49, 32, 46, LGRAY)
box(ax, 13, 41, 17, 4.5, "yggdrasil", fc=LIGHT, ec=GRAY, fs=9)
box(ax, 34, 41, 17, 4.5, "ems-backend", fc=LIGHT, ec=GRAY, fs=9)
arrow(ax, 32, 41, 32, 38, LGRAY)
box(ax, 8, 32.5, 48, 5, "~15 services", fc=LIGHT, ec=LGRAY, fs=9,
    sub="energy-model · alarms · reporting · export · benchmark · …", subfs=6.5)
arrow(ax, 22, 32.5, 22, 29.5, LGRAY)
arrow(ax, 42, 32.5, 42, 29.5, LGRAY)
box(ax, 13, 24.5, 17, 4.5, "analysis_service", fc=BLUE, tc=WHITE, ec=BLUE, fs=8.5)
box(ax, 34, 24.5, 17, 4.5, "meter_service", fc=BLUE, tc=WHITE, ec=BLUE, fs=8.5)
arrow(ax, 21.5, 24.5, 21.5, 21.5, LGRAY)
arrow(ax, 42.5, 24.5, 42.5, 21.5, LGRAY)
box(ax, 13, 16.5, 17, 4.5, "counter_meas. DB", fc=WHITE, ec=GRAY, fs=8, tc=GRAY)
box(ax, 34, 16.5, 17, 4.5, "meter / me2 DB", fc=WHITE, ec=GRAY, fs=8, tc=GRAY)
# target
box(ax, 80, 49, 20, 4.5, "Frontend", fc=NAVY, tc=WHITE, ec=NAVY)
arrow(ax, 90, 49, 90, 46, LGRAY)
box(ax, 78, 41.5, 24, 4, "yggdrasil (temporary)", fc=LIGHT, ec=GRAY, fs=8.5)
arrow(ax, 90, 41.5, 90, 38.5, LGRAY)
box(ax, 70, 32.5, 18, 5, "aggregations", fc=GREEN, tc=WHITE, ec=GREEN, fs=9, sub="measurement data", subc=LGRAY)
box(ax, 92, 32.5, 18, 5, "hierarchy", fc=GREEN, tc=WHITE, ec=GREEN, fs=9, sub="identity", subc=LGRAY)
arrow(ax, 90, 32.5, 90, 29.5, LGRAY)
box(ax, 66, 21.5, 48, 6.5, "Data pipeline", fc=TEAL, tc=WHITE, ec=TEAL, fs=10,
    sub="Kinesis → Flink → S3/Iceberg → Spark rollup → DynamoDB", subc=LGRAY, subfs=7)
save(fig, "fig1-current-vs-target.png")

# ---------- 2. pipeline flow ----------
fig, ax = newfig(12, 3.4)
chain = [("Sources", LIGHT, INK, "Electrocom · MQTT/IoT · multi-tenant API · CSV · manual · me2"),
         ("Kinesis", LIGHT, INK, "via EventBridge Pipes"),
         ("Flink", TEAL, WHITE, "enrich + resample"),
         ("logical_sensor_data", TEAL, WHITE, "S3 / Iceberg"),
         ("Spark rollup", TEAL, WHITE, "hour / day"),
         ("measurements_\naggregate", GREEN, WHITE, "DynamoDB"),
         ("aggregations API", NAVY, WHITE, "→ Frontend")]
n = len(chain)
bw, gap = 14.5, 2.5
x = 3
y = 12
for i, (label, fc, tc, sub) in enumerate(chain):
    box(ax, x, y, bw, 9, label, fc=fc, ec=(LGRAY if fc == LIGHT else fc), tc=tc, fs=8.5,
        sub=sub, subc=(LGRAY if tc == WHITE else GRAY), subfs=6)
    if i < n - 1:
        arrow(ax, x + bw, y + 4.5, x + bw + gap, y + 4.5, GRAY)
    x += bw + gap
save(fig, "fig2-pipeline.png")

# ---------- 3. compute lanes -> item ----------
fig, ax = newfig(12, 5.2)
title(ax, 30, 49, "Three lanes write columns onto one item", 11, INK)
lanes = [("A · Spark rollup", BLUE, "energy, flow, temperatures, cooling"),
         ("B · Flink stream", TEAL, "active-hours, standby (sub-hour)"),
         ("C · Denormalised (broadcast join)", GREEN, "cost, CO2, degree-days (weather/factors)")]
ly = 38
for name, col, desc in lanes:
    box(ax, 4, ly, 44, 8, name, fc=col, tc=WHITE, ec=col, fs=9.5, sub=desc, subc=LGRAY, subfs=6.5)
    arrow(ax, 48, ly + 4, 55, ly + 4, GRAY)
    ly -= 12
# item container
box(ax, 56, 6, 60, 42, "", fc=LIGHT, ec=LGRAY)
ax.text(59, 44, "measurements_aggregate  —  one DynamoDB item", fontsize=9.5, color=INK, fontweight="bold", family=FONT, va="center")
ax.text(59, 40, "keyed by: node · resource · granularity · period", fontsize=7.5, color=GRAY, family=FONT, va="center")
box(ax, 59, 30, 54, 7, "from readings — Spark (A)", fc=BLUE, tc=WHITE, ec=BLUE, fs=8.5,
    sub="sum · count · min · max · last · temp · flow · cooling", subc=LGRAY, subfs=6.5)
box(ax, 59, 21.5, 54, 6.5, "fine resolution — Flink (B)", fc=TEAL, tc=WHITE, ec=TEAL, fs=8.5,
    sub="active_hours · standby_energy", subc=LGRAY, subfs=6.5)
box(ax, 59, 13, 54, 7, "reference data — denormalised (C)", fc=GREEN, tc=WHITE, ec=GREEN, fs=8.5,
    sub="cost · co2_scope_* · hdd · cdd · energy_cc", subc=LGRAY, subfs=6.5)
ax.text(59, 9.5, "a read = one DynamoDB lookup, no joins", fontsize=8.5, color=AMBER, fontweight="bold", family=FONT, va="center")
save(fig, "fig3-compute-lanes.png")

# ---------- 4. meter model 4 -> 1 ----------
fig, ax = newfig(12, 4.2)
title(ax, 26, 39, "Four concepts", 10.5, GRAY)
for lbl, gx, gy in [("main meter", 8, 26), ("sub meter", 30, 26),
                    ("calculation meter", 8, 15), ("\"part of summation\"", 30, 15)]:
    box(ax, gx, gy, 18, 8, lbl, fc=LGRAY, ec=GRAY, tc=INK, fs=9)
arrow(ax, 52, 21, 62, 21, TEAL, lw=2.4)
box(ax, 64, 16, 48, 12, "Sensor + formula", fc=GREEN, tc=WHITE, ec=GREEN, fs=13,
    sub="one concept · Identity / Zero / Expr", subc=LGRAY, subfs=8)
ax.text(88, 11, "sub→main = one reference · netting auto-generated · no second hierarchy",
        ha="center", fontsize=7.5, color=GRAY, family=FONT)
save(fig, "fig4-meter-model.png")

# ---------- 5. path ----------
fig, ax = newfig(12, 4.2)
box(ax, 3, 24, 20, 10, "Phase 0", fc=NAVY, tc=WHITE, ec=NAVY, fs=11, sub="Verify & size (now)", subc=LGRAY, subfs=7)
arrow(ax, 23, 29, 27, 29, GRAY)
box(ax, 28, 30, 34, 7, "Phase 1 — Extend the data", fc=GREEN, tc=WHITE, ec=GREEN, fs=9.5, sub="weather / cost / CO2 columns", subc=LGRAY, subfs=6.5)
box(ax, 28, 21, 34, 7, "Phase 2 — Formula & hierarchy", fc=BLUE, tc=WHITE, ec=BLUE, fs=9.5, sub="wire eval; migrate hierarchy", subc=LGRAY, subfs=6.5)
ax.text(45, 17.5, "(parallel)", ha="center", fontsize=7.5, color=GRAY, family=FONT)
arrow(ax, 62, 29, 66, 29, GRAY)
box(ax, 67, 24, 20, 10, "Phase 3", fc=NAVY, tc=WHITE, ec=NAVY, fs=11, sub="Consolidate & cut over", subc=LGRAY, subfs=7)
box(ax, 3, 10, 84, 6, "Track I — Ingestion (parallel foundation): parsers / EventBridge Pipes / IoT Core → all sources in the pipeline",
    fc=LIGHT, ec=AMBER, tc=INK, fs=8.5)
save(fig, "fig5-path.png")

print("done")
