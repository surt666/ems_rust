#!/usr/bin/env python3
"""Generate architecture diagrams for the data pipeline documentation."""

import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
import matplotlib.patches as mpatches
from matplotlib.patches import FancyBboxPatch, FancyArrowPatch
import matplotlib.patheffects as pe
import numpy as np

# ── Color palette ──
C = {
    "bg":           "#FFFFFF",
    "person":       "#08427B",
    "system":       "#1168BD",
    "system_ext":   "#999999",
    "container":    "#438DD5",
    "component":    "#85BBF0",
    "store":        "#2D882D",
    "stream":       "#E07020",
    "error":        "#CC3333",
    "lambda":       "#D4740E",
    "glue":         "#6B3FA0",
    "text_light":   "#FFFFFF",
    "text_dark":    "#333333",
    "arrow":        "#555555",
    "arrow_err":    "#CC3333",
    "border":       "#333333",
}

def draw_box(ax, x, y, w, h, label, sublabel="", color="#438DD5",
             text_color="#FFFFFF", fontsize=11, sublabel_size=8, alpha=1.0,
             border_color=None, style="round,pad=0.02", zorder=2):
    box = FancyBboxPatch(
        (x - w/2, y - h/2), w, h,
        boxstyle=style,
        facecolor=color, edgecolor=border_color or color,
        linewidth=1.5, alpha=alpha, zorder=zorder
    )
    ax.add_patch(box)
    if sublabel:
        ax.text(x, y + 0.02, label, ha="center", va="bottom",
                fontsize=fontsize, fontweight="bold", color=text_color, zorder=zorder+1)
        ax.text(x, y - 0.02, sublabel, ha="center", va="top",
                fontsize=sublabel_size, color=text_color, alpha=0.85, zorder=zorder+1,
                style="italic")
    else:
        ax.text(x, y, label, ha="center", va="center",
                fontsize=fontsize, fontweight="bold", color=text_color, zorder=zorder+1)


def draw_arrow(ax, x1, y1, x2, y2, label="", color="#555555",
               fontsize=7, style="->", lw=1.5, label_offset=(0, 0.02)):
    ax.annotate(
        "", xy=(x2, y2), xytext=(x1, y1),
        arrowprops=dict(arrowstyle=style, color=color, lw=lw),
        zorder=1
    )
    if label:
        mx, my = (x1+x2)/2 + label_offset[0], (y1+y2)/2 + label_offset[1]
        ax.text(mx, my, label, ha="center", va="bottom", fontsize=fontsize,
                color=color, zorder=5,
                bbox=dict(boxstyle="round,pad=0.15", facecolor="white",
                          edgecolor="none", alpha=0.85))


def draw_cylinder(ax, x, y, w, h, label, sublabel="", color="#2D882D",
                  text_color="#FFFFFF", fontsize=10, sublabel_size=8):
    """Draw a database cylinder shape."""
    from matplotlib.patches import Ellipse
    body = FancyBboxPatch(
        (x - w/2, y - h/2), w, h * 0.8,
        boxstyle="round,pad=0.01",
        facecolor=color, edgecolor=color, linewidth=1.5, zorder=2
    )
    ax.add_patch(body)
    top = Ellipse((x, y + h*0.3), w, h*0.25, facecolor=color,
                  edgecolor="white", linewidth=1, zorder=3, alpha=0.7)
    ax.add_patch(top)
    if sublabel:
        ax.text(x, y + 0.01, label, ha="center", va="bottom",
                fontsize=fontsize, fontweight="bold", color=text_color, zorder=4)
        ax.text(x, y - 0.02, sublabel, ha="center", va="top",
                fontsize=sublabel_size, color=text_color, alpha=0.85, zorder=4,
                style="italic")
    else:
        ax.text(x, y, label, ha="center", va="center",
                fontsize=fontsize, fontweight="bold", color=text_color, zorder=4)


# ═══════════════════════════════════════════════════════════
# DIAGRAM 1: System Context (C4 Level 1)
# ═══════════════════════════════════════════════════════════
def diagram_context():
    fig, ax = plt.subplots(1, 1, figsize=(14, 9))
    ax.set_xlim(0, 1)
    ax.set_ylim(0, 1)
    ax.set_aspect("equal")
    ax.axis("off")
    fig.patch.set_facecolor(C["bg"])

    ax.text(0.5, 0.97, "System Context — DAQ Data Pipeline",
            ha="center", va="top", fontsize=18, fontweight="bold", color=C["text_dark"])
    ax.text(0.5, 0.93, "How the pipeline fits into the EMS ecosystem",
            ha="center", va="top", fontsize=11, color="#666666")

    # IoT devices (external)
    draw_box(ax, 0.15, 0.75, 0.22, 0.10, "IoT Devices & Gateways",
             "EMU, Kamstrup, Adeunis,\nBluemetering, MIVO, ...",
             color=C["system_ext"], fontsize=10, sublabel_size=8)

    # Data Pipeline (main system)
    draw_box(ax, 0.50, 0.50, 0.42, 0.16, "DAQ Data Pipeline",
             "Ingests raw IoT readings, enriches\nwith meter identity, bins (interpolation /\ntime-proportional split), writes to data lake",
             color=C["system"], fontsize=13, sublabel_size=9)

    # Meter Registry (external)
    draw_box(ax, 0.88, 0.75, 0.18, 0.10, "Meter Registry",
             "DynamoDB\nmeter-identity",
             color=C["system_ext"], fontsize=10, sublabel_size=8)

    # Data Lake (downstream)
    draw_box(ax, 0.50, 0.20, 0.28, 0.10, "Iceberg Data Lake",
             "S3 Tables: raw readings\n+ enriched meter data",
             color=C["store"], fontsize=11, sublabel_size=9)

    # Downstream consumers
    draw_box(ax, 0.15, 0.20, 0.20, 0.10, "Analytics & Dashboards",
             "Athena queries,\nBI tools, APIs",
             color=C["system_ext"], fontsize=10, sublabel_size=8)

    draw_box(ax, 0.85, 0.20, 0.20, 0.10, "Alerting & Monitoring",
             "Error stream consumers,\nCloudWatch alarms",
             color=C["system_ext"], fontsize=10, sublabel_size=8)

    # Arrows
    draw_arrow(ax, 0.26, 0.70, 0.36, 0.57, "JSON readings\nvia Kinesis", fontsize=8)
    draw_arrow(ax, 0.82, 0.70, 0.68, 0.57, "Meter mappings\n(bootstrap + CDC)", fontsize=8)
    draw_arrow(ax, 0.50, 0.42, 0.50, 0.26, "Enriched records\n(Iceberg append)", fontsize=8)
    draw_arrow(ax, 0.38, 0.22, 0.25, 0.25, "", fontsize=7)
    draw_arrow(ax, 0.62, 0.22, 0.75, 0.22, "", fontsize=7)
    draw_arrow(ax, 0.60, 0.42, 0.80, 0.25, "Error records", fontsize=8,
               color=C["arrow_err"])

    # Legend
    for i, (label, col) in enumerate([
        ("Core System", C["system"]),
        ("External System", C["system_ext"]),
        ("Data Store", C["store"]),
    ]):
        draw_box(ax, 0.12 + i*0.18, 0.06, 0.14, 0.04, label,
                 color=col, fontsize=8, text_color=C["text_light"])

    fig.savefig("/home/sla/projects/EMS/infra/daq/data_pipeline/docs/01-system-context.png",
                dpi=150, bbox_inches="tight", facecolor=C["bg"])
    plt.close(fig)
    print("  01-system-context.png")


# ═══════════════════════════════════════════════════════════
# DIAGRAM 2: Container View (C4 Level 2)
# ═══════════════════════════════════════════════════════════
def diagram_containers():
    fig, ax = plt.subplots(1, 1, figsize=(16, 11))
    ax.set_xlim(0, 1)
    ax.set_ylim(0, 1)
    ax.set_aspect("equal")
    ax.axis("off")
    fig.patch.set_facecolor(C["bg"])

    ax.text(0.5, 0.98, "Container View — AWS Components & Data Flow",
            ha="center", va="top", fontsize=17, fontweight="bold", color=C["text_dark"])
    ax.text(0.5, 0.95, "All infrastructure components and how data moves between them",
            ha="center", va="top", fontsize=10, color="#666666")

    # ── Row 1: Sources ──
    draw_box(ax, 0.12, 0.84, 0.18, 0.07, "IoT Gateways",
             "10+ device types",
             color=C["system_ext"], fontsize=10, sublabel_size=8)

    draw_box(ax, 0.38, 0.84, 0.22, 0.07, "DAQ_INPUT_STREAM",
             "Kinesis Data Stream",
             color=C["stream"], fontsize=10, sublabel_size=8)

    draw_box(ax, 0.88, 0.84, 0.18, 0.07, "meter-identity",
             "DynamoDB table",
             color=C["store"], fontsize=10, sublabel_size=8)

    draw_box(ax, 0.68, 0.84, 0.14, 0.07, "DDB CDC Stream",
             "Kinesis",
             color=C["stream"], fontsize=9, sublabel_size=7)

    # Arrow: IoT → Kinesis
    draw_arrow(ax, 0.21, 0.84, 0.27, 0.84, "JSON", fontsize=7)
    # Arrow: DDB → CDC
    draw_arrow(ax, 0.82, 0.81, 0.75, 0.81, "Changes", fontsize=7)

    # ── Row 2: Flink (large box) ──
    # Outer boundary
    flink_box = FancyBboxPatch(
        (0.04, 0.40), 0.72, 0.34,
        boxstyle="round,pad=0.015",
        facecolor="#E8F0FE", edgecolor=C["container"],
        linewidth=2.5, linestyle="--", zorder=1
    )
    ax.add_patch(flink_box)
    ax.text(0.40, 0.73, "Apache Flink Application  (Managed Service for Apache Flink)",
            ha="center", va="top", fontsize=12, fontweight="bold",
            color=C["container"], zorder=2)

    # Inside Flink
    draw_box(ax, 0.14, 0.62, 0.16, 0.06, "JSON Parser",
             "10 device processors",
             color=C["component"], text_color=C["text_dark"],
             fontsize=9, sublabel_size=7)

    draw_box(ax, 0.35, 0.62, 0.16, 0.06, "Meter Enrichment",
             "Broadcast state join",
             color=C["component"], text_color=C["text_dark"],
             fontsize=9, sublabel_size=7)

    draw_box(ax, 0.56, 0.62, 0.14, 0.06, "Binning",
             "Per-bin emit (linear interp /\ntime-proportional)",
             color=C["component"], text_color=C["text_dark"],
             fontsize=9, sublabel_size=7)

    draw_box(ax, 0.14, 0.48, 0.14, 0.05, "Raw Sink",
             "Iceberg writer",
             color=C["component"], text_color=C["text_dark"],
             fontsize=9, sublabel_size=7)

    draw_box(ax, 0.56, 0.48, 0.14, 0.05, "Enriched Sink",
             "Iceberg writer",
             color=C["component"], text_color=C["text_dark"],
             fontsize=9, sublabel_size=7)

    draw_box(ax, 0.35, 0.48, 0.14, 0.05, "Error Sink",
             "Kinesis writer",
             color="#D08080", text_color=C["text_dark"],
             fontsize=9, sublabel_size=7)

    # Flink internal arrows
    draw_arrow(ax, 0.22, 0.62, 0.27, 0.62, "", fontsize=6, lw=1)
    draw_arrow(ax, 0.43, 0.62, 0.49, 0.62, "", fontsize=6, lw=1)
    draw_arrow(ax, 0.14, 0.59, 0.14, 0.51, "", fontsize=6, lw=1)
    draw_arrow(ax, 0.56, 0.59, 0.56, 0.51, "", fontsize=6, lw=1)
    # Error arrows from each stage
    draw_arrow(ax, 0.18, 0.59, 0.30, 0.51, "", fontsize=6, lw=1,
               color=C["arrow_err"])
    draw_arrow(ax, 0.38, 0.59, 0.36, 0.51, "", fontsize=6, lw=1,
               color=C["arrow_err"])
    draw_arrow(ax, 0.56, 0.59, 0.40, 0.51, "", fontsize=6, lw=1,
               color=C["arrow_err"])

    # Arrow: Kinesis Input → Flink
    draw_arrow(ax, 0.38, 0.80, 0.20, 0.66, "Raw JSON", fontsize=7)
    # Arrow: CDC → Flink enrichment
    draw_arrow(ax, 0.68, 0.80, 0.40, 0.66, "CDC events", fontsize=7)
    # Arrow: DDB bootstrap (dashed-ish)
    draw_arrow(ax, 0.88, 0.80, 0.42, 0.66, "Bootstrap scan", fontsize=7,
               style="-|>", lw=1, color="#888888")

    # ── Row 3: Data stores ──
    draw_cylinder(ax, 0.14, 0.28, 0.18, 0.10, "raw_data",
                  "Iceberg table\n(cumulative values)",
                  color=C["store"], fontsize=10, sublabel_size=7)

    draw_cylinder(ax, 0.56, 0.28, 0.24, 0.10,
                  "logical_meter_data",
                  "Iceberg table\n(enriched deltas)",
                  color=C["store"], fontsize=9, sublabel_size=7)

    draw_box(ax, 0.35, 0.30, 0.14, 0.06, "Error Stream",
             "Kinesis",
             color=C["error"], fontsize=9, sublabel_size=7)

    # Arrows: sinks → stores
    draw_arrow(ax, 0.14, 0.45, 0.14, 0.34, "", fontsize=6, lw=1.5)
    draw_arrow(ax, 0.56, 0.45, 0.56, 0.34, "", fontsize=6, lw=1.5)
    draw_arrow(ax, 0.35, 0.45, 0.35, 0.34, "", fontsize=6, lw=1.5,
               color=C["arrow_err"])

    # ── Row 4: Late recomputation ──
    draw_box(ax, 0.35, 0.15, 0.16, 0.06, "Lambda Trigger",
             "late-arrival-trigger",
             color=C["lambda"], fontsize=9, sublabel_size=7)

    draw_box(ax, 0.60, 0.15, 0.20, 0.06, "Glue Spark Job",
             "late-data-recomputation",
             color=C["glue"], fontsize=9, sublabel_size=7)

    # Arrows: error stream → lambda → glue → enriched table
    draw_arrow(ax, 0.35, 0.27, 0.35, 0.19, "late_arrival\nrecords",
               fontsize=7, color=C["arrow_err"])
    draw_arrow(ax, 0.43, 0.15, 0.50, 0.15, "Start job", fontsize=7)
    draw_arrow(ax, 0.65, 0.19, 0.58, 0.24, "Recomputed\ndeltas",
               fontsize=7, color=C["glue"])
    # Glue reads raw
    draw_arrow(ax, 0.55, 0.13, 0.20, 0.24, "Read raw data",
               fontsize=7, color=C["glue"], style="-|>", lw=1)

    # Legend
    for i, (label, col) in enumerate([
        ("Kinesis Stream", C["stream"]),
        ("Flink Operator", C["component"]),
        ("Data Store", C["store"]),
        ("Error Path", C["error"]),
        ("Batch Recompute", C["glue"]),
    ]):
        draw_box(ax, 0.10 + i*0.17, 0.04, 0.14, 0.035, label,
                 color=col, fontsize=7, text_color=C["text_light"])

    fig.savefig("/home/sla/projects/EMS/infra/daq/data_pipeline/docs/02-container-view.png",
                dpi=150, bbox_inches="tight", facecolor=C["bg"])
    plt.close(fig)
    print("  02-container-view.png")


# ═══════════════════════════════════════════════════════════
# DIAGRAM 3: Flink Internals (C4 Level 3)
# ═══════════════════════════════════════════════════════════
def diagram_flink_internals():
    fig, ax = plt.subplots(1, 1, figsize=(16, 10))
    ax.set_xlim(0, 1)
    ax.set_ylim(0, 1)
    ax.set_aspect("equal")
    ax.axis("off")
    fig.patch.set_facecolor(C["bg"])

    ax.text(0.5, 0.98, "Flink Application — Internal Processing Stages",
            ha="center", va="top", fontsize=17, fontweight="bold", color=C["text_dark"])
    ax.text(0.5, 0.95, "How raw JSON becomes enriched meter readings inside the Flink job",
            ha="center", va="top", fontsize=10, color="#666666")

    # ── Stage 1: Parsing ──
    draw_box(ax, 0.12, 0.82, 0.18, 0.08, "Kinesis Source",
             "DAQ_INPUT_STREAM\nTRIM_HORIZON",
             color=C["stream"], fontsize=10, sublabel_size=7)

    draw_box(ax, 0.38, 0.82, 0.22, 0.08, "JSON Parser",
             "Routes by schematype:\nemu, gwb143, std, pulse,\nflowiq, bluemetering, ...",
             color=C["component"], text_color=C["text_dark"],
             fontsize=10, sublabel_size=7)

    draw_arrow(ax, 0.21, 0.82, 0.27, 0.82, "Raw JSON", fontsize=8)

    # Stage 1 outputs
    ax.text(0.50, 0.77, "SensorRecord", ha="center", va="top",
            fontsize=9, color=C["container"], fontweight="bold",
            bbox=dict(boxstyle="round,pad=0.2", facecolor="#E8F0FE",
                      edgecolor=C["container"], alpha=0.8))

    # ── Fork: Raw branch + Enrichment branch ──
    draw_arrow(ax, 0.38, 0.77, 0.15, 0.71, "Raw branch", fontsize=7)
    draw_arrow(ax, 0.52, 0.77, 0.52, 0.71, "Enrichment branch", fontsize=7)

    # Raw branch
    draw_box(ax, 0.15, 0.67, 0.18, 0.06, "Raw Iceberg Sink",
             "Append to raw_data",
             color=C["store"], fontsize=9, sublabel_size=7)

    # ── Stage 2: Watermark ──
    draw_box(ax, 0.52, 0.67, 0.22, 0.06, "Watermark Assignment",
             "1h out-of-orderness, 24h idle",
             color=C["component"], text_color=C["text_dark"],
             fontsize=9, sublabel_size=7)

    draw_arrow(ax, 0.52, 0.64, 0.52, 0.58, "", fontsize=6)

    # ── Stage 3: Enrichment ──
    # Broadcast state box
    enrich_bg = FancyBboxPatch(
        (0.26, 0.44), 0.52, 0.14,
        boxstyle="round,pad=0.01",
        facecolor="#FFF8E8", edgecolor=C["lambda"],
        linewidth=1.5, linestyle="--", zorder=1
    )
    ax.add_patch(enrich_bg)
    ax.text(0.52, 0.575, "Meter Enrichment  (BroadcastProcessFunction)", ha="center",
            fontsize=10, fontweight="bold", color=C["lambda"], zorder=2)

    draw_box(ax, 0.36, 0.49, 0.14, 0.05, "Bootstrap Cache",
             "Parallel DDB scan",
             color="#D4A76A", text_color=C["text_dark"],
             fontsize=8, sublabel_size=7)

    draw_box(ax, 0.52, 0.49, 0.14, 0.05, "Broadcast State",
             "CDC updates",
             color="#D4A76A", text_color=C["text_dark"],
             fontsize=8, sublabel_size=7)

    draw_box(ax, 0.68, 0.49, 0.08, 0.05, "Join",
             "",
             color=C["component"], text_color=C["text_dark"],
             fontsize=9, sublabel_size=7)

    draw_arrow(ax, 0.59, 0.49, 0.64, 0.49, "", fontsize=6, lw=1)
    draw_arrow(ax, 0.43, 0.49, 0.48, 0.49, "", fontsize=6, lw=1)

    # CDC source
    draw_box(ax, 0.88, 0.54, 0.16, 0.05, "DDB CDC Stream",
             "Kinesis",
             color=C["stream"], fontsize=8, sublabel_size=7)
    draw_arrow(ax, 0.82, 0.52, 0.59, 0.50, "INSERT/MODIFY/\nREMOVE", fontsize=7)

    # DDB bootstrap
    draw_box(ax, 0.88, 0.46, 0.16, 0.05, "meter-identity",
             "DynamoDB",
             color=C["store"], fontsize=8, sublabel_size=7)
    draw_arrow(ax, 0.82, 0.46, 0.43, 0.48, "Startup scan\n(20 segments)", fontsize=7,
               color="#888888")

    # Output of enrichment
    ax.text(0.52, 0.42, "(EnrichedRecord, meterType)", ha="center", va="top",
            fontsize=9, color=C["container"], fontweight="bold",
            bbox=dict(boxstyle="round,pad=0.2", facecolor="#E8F0FE",
                      edgecolor=C["container"], alpha=0.8))

    draw_arrow(ax, 0.52, 0.44, 0.52, 0.425, "", fontsize=6)

    # ── Stage 4: BinningFunction ──
    delta_bg = FancyBboxPatch(
        (0.15, 0.17), 0.70, 0.20,
        boxstyle="round,pad=0.01",
        facecolor="#F0F8F0", edgecolor=C["store"],
        linewidth=1.5, linestyle="--", zorder=1
    )
    ax.add_patch(delta_bg)
    ax.text(0.50, 0.365, "BinningFunction  (KeyedProcessFunction, keyed by logicalId)",
            ha="center", fontsize=10, fontweight="bold", color=C["store"], zorder=2)

    draw_arrow(ax, 0.52, 0.41, 0.52, 0.37, "", fontsize=6)

    # Gauge path: linear interpolation per bin
    draw_box(ax, 0.22, 0.30, 0.12, 0.05, "Gauge",
             "Linear interp at bin B",
             color="#88BB88", text_color=C["text_dark"],
             fontsize=9, sublabel_size=7)

    # Counter path
    draw_box(ax, 0.40, 0.30, 0.12, 0.05, "Buffer",
             "MapState\n[ts \u2192 value]",
             color="#88BB88", text_color=C["text_dark"],
             fontsize=9, sublabel_size=7)

    draw_box(ax, 0.58, 0.30, 0.12, 0.05, "Per-bin emit",
             "delta * overlap / period",
             color="#88BB88", text_color=C["text_dark"],
             fontsize=9, sublabel_size=7)

    draw_arrow(ax, 0.46, 0.30, 0.52, 0.30, "", fontsize=6, lw=1)

    # Outputs
    draw_box(ax, 0.40, 0.20, 0.16, 0.04, "Enriched Iceberg Sink",
             color=C["store"], fontsize=9, sublabel_size=7)

    draw_box(ax, 0.68, 0.20, 0.14, 0.04, "Error Outputs",
             color=C["error"], fontsize=9, sublabel_size=7)

    draw_arrow(ax, 0.22, 0.27, 0.35, 0.22, "raw value", fontsize=7)
    draw_arrow(ax, 0.58, 0.27, 0.45, 0.22, "delta \u2265 0", fontsize=7)
    draw_arrow(ax, 0.64, 0.30, 0.68, 0.23, "anomaly /\nlate_arrival",
               fontsize=7, color=C["arrow_err"])

    # Parse error
    draw_arrow(ax, 0.49, 0.82, 0.72, 0.78, "parse_error",
               fontsize=7, color=C["arrow_err"])
    draw_box(ax, 0.80, 0.78, 0.10, 0.03, "Errors",
             color=C["error"], fontsize=8)

    # Dead letter
    draw_arrow(ax, 0.72, 0.47, 0.80, 0.47, "dead_letter",
               fontsize=7, color=C["arrow_err"])

    # Error collection box
    draw_box(ax, 0.86, 0.47, 0.06, 0.03, "",
             color=C["error"], fontsize=8)

    fig.savefig("/home/sla/projects/EMS/infra/daq/data_pipeline/docs/03-flink-internals.png",
                dpi=150, bbox_inches="tight", facecolor=C["bg"])
    plt.close(fig)
    print("  03-flink-internals.png")


# ═══════════════════════════════════════════════════════════
# DIAGRAM 4: Error & Late Arrival Recovery Flow
# ═══════════════════════════════════════════════════════════
def diagram_error_flow():
    fig, ax = plt.subplots(1, 1, figsize=(15, 9))
    ax.set_xlim(0, 1)
    ax.set_ylim(0, 1)
    ax.set_aspect("equal")
    ax.axis("off")
    fig.patch.set_facecolor(C["bg"])

    ax.text(0.5, 0.97, "Error Handling & Late Arrival Recovery",
            ha="center", va="top", fontsize=17, fontweight="bold", color=C["text_dark"])
    ax.text(0.5, 0.93, "Four error types flow through a unified error stream, with automatic recovery for late arrivals",
            ha="center", va="top", fontsize=10, color="#666666")

    # ── Error sources (left column) ──
    draw_box(ax, 0.15, 0.80, 0.22, 0.06, "PARSE_ERROR",
             "Invalid JSON / unknown schema",
             color="#E07070", text_color=C["text_dark"],
             fontsize=10, sublabel_size=8)

    draw_box(ax, 0.15, 0.70, 0.22, 0.06, "DEAD_LETTER",
             "No meter mapping for daq_id",
             color="#E09050", text_color=C["text_dark"],
             fontsize=10, sublabel_size=8)

    draw_box(ax, 0.15, 0.60, 0.22, 0.06, "ANOMALY",
             "Negative counter delta (reset?)",
             color="#D0A030", text_color=C["text_dark"],
             fontsize=10, sublabel_size=8)

    draw_box(ax, 0.15, 0.50, 0.22, 0.06, "LATE_ARRIVAL",
             "Predecessor purged from buffer",
             color=C["error"], fontsize=10, sublabel_size=8)

    # ── Union → Error Stream ──
    draw_box(ax, 0.48, 0.65, 0.14, 0.24, "Union",
             "",
             color="#DDDDDD", text_color=C["text_dark"],
             fontsize=10, border_color="#999999")

    for y_src in [0.80, 0.70, 0.60, 0.50]:
        draw_arrow(ax, 0.26, y_src, 0.41, 0.65, "", fontsize=6,
                   color=C["arrow_err"], lw=1)

    draw_box(ax, 0.70, 0.65, 0.18, 0.08, "Error Stream",
             "Kinesis\nflink-*-errors",
             color=C["error"], fontsize=11, sublabel_size=8)

    draw_arrow(ax, 0.55, 0.65, 0.61, 0.65, "ErrorRecord\nJSON", fontsize=8,
               color=C["arrow_err"])

    # ── Lambda filter ──
    draw_box(ax, 0.70, 0.45, 0.22, 0.08, "Lambda Trigger",
             "Filters type=late_arrival\n5-min batch window",
             color=C["lambda"], fontsize=10, sublabel_size=8)

    draw_arrow(ax, 0.70, 0.61, 0.70, 0.50, "All error records", fontsize=8)

    # Show filter
    ax.text(0.82, 0.53, "parse_error  \u2717\ndead_letter   \u2717\nanomaly         \u2717\nlate_arrival   \u2713",
            fontsize=8, color=C["text_dark"], family="monospace",
            va="top", ha="left",
            bbox=dict(boxstyle="round,pad=0.3", facecolor="#FFF8F0",
                      edgecolor=C["lambda"], alpha=0.9))

    # ── Glue job ──
    draw_box(ax, 0.48, 0.28, 0.24, 0.08, "Glue Spark Job",
             "Reads raw_data, joins identity,\ncomputes deltas via LAG()",
             color=C["glue"], fontsize=10, sublabel_size=8)

    draw_arrow(ax, 0.70, 0.41, 0.58, 0.33, "StartJobRun\n(daq_id, time_range)",
               fontsize=8)

    # ── Data stores ──
    draw_cylinder(ax, 0.18, 0.22, 0.16, 0.08, "raw_data",
                  "Read cumulative values",
                  color=C["store"], fontsize=9, sublabel_size=7)

    draw_cylinder(ax, 0.18, 0.10, 0.16, 0.08, "meter-identity",
                  "DDB lookup",
                  color=C["store"], fontsize=9, sublabel_size=7)

    draw_arrow(ax, 0.26, 0.22, 0.38, 0.28, "", fontsize=6, color=C["glue"])
    draw_arrow(ax, 0.26, 0.12, 0.38, 0.26, "", fontsize=6, color=C["glue"])

    # Output
    draw_cylinder(ax, 0.75, 0.18, 0.24, 0.08,
                  "logical_meter_data",
                  "Append recomputed deltas\n(created=now)",
                  color=C["store"], fontsize=9, sublabel_size=7)

    draw_arrow(ax, 0.60, 0.25, 0.66, 0.20, "Corrected records", fontsize=8,
               color=C["glue"])

    # Event sourcing note
    ax.text(0.75, 0.10, "Event sourcing: consumers use\nROW_NUMBER() OVER (...ORDER BY created DESC) = 1\nto get the latest version of each reading",
            fontsize=8, color="#666666", ha="center", va="top",
            family="monospace",
            bbox=dict(boxstyle="round,pad=0.3", facecolor="#F8F8F8",
                      edgecolor="#CCCCCC"))

    fig.savefig("/home/sla/projects/EMS/infra/daq/data_pipeline/docs/04-error-recovery-flow.png",
                dpi=150, bbox_inches="tight", facecolor=C["bg"])
    plt.close(fig)
    print("  04-error-recovery-flow.png")


if __name__ == "__main__":
    print("Generating diagrams...")
    diagram_context()
    diagram_containers()
    diagram_flink_internals()
    diagram_error_flow()
    print("Done!")
