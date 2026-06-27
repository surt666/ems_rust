#!/usr/bin/env python3
"""
Generate docs/product-vision.pptx — an executive (technical-leadership) vision deck
for the Enity EMS platform: Frontend -> API -> Intelligence -> Data.

Reproducible build:
    uv run --with python-pptx python3 docs/build_product_vision_pptx.py

Diagrams are native PowerPoint shapes (rounded rectangles + connectors), so the
deck stays fully editable in PowerPoint / Keynote / Google Slides. The palette
mirrors docs/system-design-presentation.html (the existing dark house style).
"""

from pptx import Presentation
from pptx.util import Inches as In, Pt
from pptx.dml.color import RGBColor
from pptx.enum.text import PP_ALIGN, MSO_ANCHOR
from pptx.enum.shapes import MSO_SHAPE, MSO_CONNECTOR
from pptx.oxml.ns import qn

# ---------------------------------------------------------------- palette
def rgb(h): return RGBColor.from_string(h)
BG      = rgb("0F1117")
SURFACE = rgb("1A1D27")
SURF2   = rgb("151823")
BORDER  = rgb("2A2D3A")
FAINT   = rgb("20222C")
TEXT    = rgb("E2E4E9")
MUTED   = rgb("8B8FA3")
ACCENT  = rgb("6C8CFF")
GREEN   = rgb("4ADE80")
RED     = rgb("F87171")
ORANGE  = rgb("FB923C")
YELLOW  = rgb("FACC15")
PURPLE  = rgb("A78BFA")
CYAN    = rgb("22D3EE")
TEAL    = rgb("2DD4BF")
FONT    = "Inter"

# ---------------------------------------------------------------- deck setup
prs = Presentation()
prs.slide_width  = In(13.333)
prs.slide_height = In(7.5)
BLANK = prs.slide_layouts[6]
TOTAL = 16

def new_slide():
    s = prs.slides.add_slide(BLANK)
    s.background.fill.solid()
    s.background.fill.fore_color.rgb = BG
    return s

# ---------------------------------------------------------------- primitives
def _noshadow(sp):
    sp.shadow.inherit = False

def text(slide, x, y, w, h, s, size: float = 14, color=TEXT, bold=False, italic=False,
         align=PP_ALIGN.LEFT, anchor=MSO_ANCHOR.TOP, spacing=None, font=FONT):
    tb = slide.shapes.add_textbox(In(x), In(y), In(w), In(h))
    tf = tb.text_frame
    tf.word_wrap = True
    tf.vertical_anchor = anchor
    tf.margin_left = tf.margin_right = tf.margin_top = tf.margin_bottom = 0
    for i, line in enumerate(str(s).split("\n")):
        p = tf.paragraphs[0] if i == 0 else tf.add_paragraph()
        p.alignment = align
        if spacing:
            p.line_spacing = spacing
        r = p.add_run()
        r.text = line
        r.font.size = Pt(size)
        r.font.bold = bold
        r.font.italic = italic
        r.font.color.rgb = color
        r.font.name = font
    return tb

def node(slide, x, y, w, h, title, subtitle=None, color=ACCENT, fill=SURFACE,
         round=0.12, title_size=12, sub_size=8.5, title_color=None, line_w=1.25):
    sp = slide.shapes.add_shape(MSO_SHAPE.ROUNDED_RECTANGLE, In(x), In(y), In(w), In(h))
    try:
        sp.adjustments[0] = round
    except Exception:
        pass
    sp.fill.solid(); sp.fill.fore_color.rgb = fill
    sp.line.color.rgb = color; sp.line.width = Pt(line_w)
    _noshadow(sp)
    tf = sp.text_frame
    tf.word_wrap = True
    tf.vertical_anchor = MSO_ANCHOR.MIDDLE
    tf.margin_left = tf.margin_right = In(0.06)
    tf.margin_top = tf.margin_bottom = In(0.02)
    p = tf.paragraphs[0]; p.alignment = PP_ALIGN.CENTER
    r = p.add_run(); r.text = title
    r.font.size = Pt(title_size); r.font.bold = True
    r.font.color.rgb = title_color or color; r.font.name = FONT
    if subtitle:
        p2 = tf.add_paragraph(); p2.alignment = PP_ALIGN.CENTER
        r2 = p2.add_run(); r2.text = subtitle
        r2.font.size = Pt(sub_size); r2.font.color.rgb = MUTED; r2.font.name = FONT
    return sp

def arrow(slide, x1, y1, x2, y2, color=ACCENT, width=1.25, dash=False,
          head=True, tail=False):
    cn = slide.shapes.add_connector(MSO_CONNECTOR.STRAIGHT, In(x1), In(y1), In(x2), In(y2))
    cn.line.color.rgb = color; cn.line.width = Pt(width)
    _noshadow(cn)
    ln = cn.line._get_or_add_ln()
    if dash:
        ln.append(ln.makeelement(qn('a:prstDash'), {'val': 'dash'}))
    if head:
        ln.append(ln.makeelement(qn('a:tailEnd'), {'type': 'triangle', 'w': 'med', 'len': 'med'}))
    if tail:
        ln.append(ln.makeelement(qn('a:headEnd'), {'type': 'triangle', 'w': 'med', 'len': 'med'}))
    return cn

def hline(slide, x1, x2, y, color=BORDER, width=1.0):
    cn = slide.shapes.add_connector(MSO_CONNECTOR.STRAIGHT, In(x1), In(y), In(x2), In(y))
    cn.line.color.rgb = color; cn.line.width = Pt(width)
    _noshadow(cn)
    return cn

def tag(slide, x, y, label, color=ACCENT):
    w = 0.105 * len(label) + 0.34
    sp = slide.shapes.add_shape(MSO_SHAPE.ROUNDED_RECTANGLE, In(x), In(y), In(w), In(0.31))
    try:
        sp.adjustments[0] = 0.5
    except Exception:
        pass
    sp.fill.solid(); sp.fill.fore_color.rgb = SURFACE
    sp.line.color.rgb = color; sp.line.width = Pt(1)
    _noshadow(sp)
    tf = sp.text_frame; tf.vertical_anchor = MSO_ANCHOR.MIDDLE
    tf.margin_left = tf.margin_right = In(0.08); tf.margin_top = tf.margin_bottom = 0
    p = tf.paragraphs[0]; p.alignment = PP_ALIGN.CENTER
    r = p.add_run(); r.text = label.upper()
    r.font.size = Pt(9); r.font.bold = True; r.font.color.rgb = color; r.font.name = FONT
    return sp

def card(slide, x, y, w, h, title, body, color=ACCENT, title_size: float = 13.5, body_size: float = 11):
    sp = slide.shapes.add_shape(MSO_SHAPE.ROUNDED_RECTANGLE, In(x), In(y), In(w), In(h))
    try:
        sp.adjustments[0] = 0.06
    except Exception:
        pass
    sp.fill.solid(); sp.fill.fore_color.rgb = SURFACE
    sp.line.color.rgb = BORDER; sp.line.width = Pt(1)
    _noshadow(sp)
    # left accent strip
    strip = slide.shapes.add_shape(MSO_SHAPE.ROUNDED_RECTANGLE, In(x), In(y), In(0.07), In(h))
    try:
        strip.adjustments[0] = 0.5
    except Exception:
        pass
    strip.fill.solid(); strip.fill.fore_color.rgb = color; strip.line.fill.background()
    _noshadow(strip)
    tf = sp.text_frame; tf.word_wrap = True; tf.vertical_anchor = MSO_ANCHOR.TOP
    tf.margin_left = In(0.18); tf.margin_right = In(0.14); tf.margin_top = In(0.13); tf.margin_bottom = In(0.1)
    p = tf.paragraphs[0]; p.alignment = PP_ALIGN.LEFT
    r = p.add_run(); r.text = title
    r.font.size = Pt(title_size); r.font.bold = True; r.font.color.rgb = color; r.font.name = FONT
    p2 = tf.add_paragraph(); p2.space_before = Pt(5); p2.line_spacing = 1.08
    r2 = p2.add_run(); r2.text = body
    r2.font.size = Pt(body_size); r2.font.color.rgb = MUTED; r2.font.name = FONT
    return sp

def bullets(slide, x, y, w, items, size: float = 13, marker="›", marker_color=ACCENT,
            text_color=TEXT, space_after=7, h=5.0):
    h = min(h, max(0.5, 7.3 - y))  # keep the frame box on-canvas (text never clips)
    tb = slide.shapes.add_textbox(In(x), In(y), In(w), In(h))
    tf = tb.text_frame; tf.word_wrap = True
    tf.margin_left = tf.margin_right = tf.margin_top = tf.margin_bottom = 0
    for i, it in enumerate(items):
        clr = text_color
        if isinstance(it, tuple):
            it, clr = it
        head, tail = (it.split("::", 1) + [""])[:2] if "::" in it else (None, it)
        p = tf.paragraphs[0] if i == 0 else tf.add_paragraph()
        p.space_after = Pt(space_after); p.line_spacing = 1.05
        rm = p.add_run(); rm.text = marker + "  "
        rm.font.size = Pt(size); rm.font.bold = True; rm.font.color.rgb = marker_color; rm.font.name = FONT
        if head is not None:
            rh = p.add_run(); rh.text = head + "  "
            rh.font.size = Pt(size); rh.font.bold = True; rh.font.color.rgb = clr if clr != text_color else TEXT
            rh.font.name = FONT
        rt = p.add_run(); rt.text = tail
        rt.font.size = Pt(size); rt.font.color.rgb = (MUTED if head is not None else clr); rt.font.name = FONT
    return tb

def grid_table(slide, x, y, total_w, col_fracs, headers, rows, row_h=0.46,
               header_size: float = 10.5, body_size: float = 12):
    xs, cx = [], x
    for f in col_fracs:
        xs.append(cx); cx += total_w * f
    widths = [total_w * f for f in col_fracs]
    for i, hh in enumerate(headers):
        text(slide, xs[i], y, widths[i] - 0.08, row_h, hh, size=header_size,
             color=MUTED, bold=True, anchor=MSO_ANCHOR.MIDDLE)
    hline(slide, x, x + total_w, y + row_h, BORDER, 1.2)
    ry = y + row_h + 0.05
    for r in rows:
        for i, cell in enumerate(r):
            ctext, cclr = cell if isinstance(cell, tuple) else (cell, TEXT)
            text(slide, xs[i], ry, widths[i] - 0.08, row_h, ctext, size=body_size,
                 color=cclr, bold=(i == 0 and cclr == TEXT), anchor=MSO_ANCHOR.MIDDLE)
        hline(slide, x, x + total_w, ry + row_h, FAINT, 0.75)
        ry += row_h + 0.03
    return ry

def chrome(slide, tg, tg_color, title, subtitle, number):
    tag(slide, 0.55, 0.48, tg, tg_color)
    text(slide, 0.5, 0.86, 11.0, 0.7, title, size=27, bold=True, color=TEXT)
    rule = slide.shapes.add_shape(MSO_SHAPE.RECTANGLE, In(0.57), In(1.52), In(0.62), In(0.045))
    rule.fill.solid(); rule.fill.fore_color.rgb = tg_color; rule.line.fill.background()
    _noshadow(rule)
    if subtitle:
        text(slide, 0.55, 1.62, 12.0, 0.5, subtitle, size=13.5, color=MUTED)
    text(slide, 11.7, 7.04, 1.5, 0.3, f"{number} / {TOTAL}", size=10, color=MUTED, align=PP_ALIGN.RIGHT)
    text(slide, 0.55, 7.04, 4.0, 0.3, "Enity EMS — Product Vision", size=9.5, color=rgb("5A5E70"))

# ================================================================ SLIDE 1 — title
s = new_slide()
# faint top + bottom accent bars
bar = s.shapes.add_shape(MSO_SHAPE.RECTANGLE, In(0), In(0), In(13.333), In(0.16))
bar.fill.solid(); bar.fill.fore_color.rgb = ACCENT; bar.line.fill.background(); _noshadow(bar)
text(s, 0, 2.35, 13.333, 0.5, "ENITY  EMS", size=18, bold=True, color=ACCENT, align=PP_ALIGN.CENTER)
text(s, 0, 2.85, 13.333, 1.1, "Product Vision", size=58, bold=True, color=TEXT, align=PP_ALIGN.CENTER)
text(s, 0, 4.05, 13.333, 0.5, "Frontend  →  API  →  Intelligence  →  Data",
     size=20, color=MUTED, align=PP_ALIGN.CENTER)
hline(s, 4.9, 8.43, 4.75, BORDER, 1.0)
text(s, 0, 4.95, 13.333, 0.5,
     "A serverless, partner-extensible energy management platform",
     size=14, color=TEXT, align=PP_ALIGN.CENTER)
text(s, 0, 5.3, 13.333, 0.4,
     "Slim static edge  ·  Cognito-scoped API  ·  CQRS on Graviton  ·  Tiered storage  ·  Models on your own data",
     size=12, color=MUTED, align=PP_ALIGN.CENTER)
text(s, 0, 7.0, 13.333, 0.4, "Technical Architecture Briefing  ·  2026",
     size=11, color=rgb("5A5E70"), align=PP_ALIGN.CENTER)

# ================================================================ SLIDE 2 — thesis
s = new_slide()
chrome(s, "Thesis", ACCENT, "Four moving parts, one coherent platform",
       "Each layer is thin, independently scalable, and pay-per-use — nothing runs idle.", 2)
cw, gap, cx0, cy = 2.92, 0.22, 0.6, 2.35
ch = 3.25
cards = [
    ("01  Slim static edge", "Astro/HTMX site on S3, served from CloudFront. No SSR servers, no frontend fleet — globally cached bytes. Optionally a host shell for partner micro-frontends.", TEAL),
    ("02  Thin scoped API", "Cognito-issued tokens validated at API Gateway. Every request is tenant-scoped at the edge before a line of business logic runs.", ACCENT),
    ("03  CQRS serverless compute", "One Rust/Graviton Lambda per bounded context. Command and query are different API Gateway paths into the same function — fewer Lambdas, lower Datadog cost.", PURPLE),
    ("04  Tiered storage + intelligence", "Hot, small, recent data in DynamoDB; massive history in S3 Tables. Models train on that history and serve predictions back through the same API.", GREEN),
]
for i, (t, b, c) in enumerate(cards):
    card(s, cx0 + i * (cw + gap), cy, cw, ch, t, b, c, title_size=14, body_size=11.5)
text(s, 0.6, 5.95, 12.1, 0.8,
     "Pay-per-use · scales to zero · multi-account isolated · extensible by partners without forking the core.",
     size=13.5, color=TEXT, bold=True, align=PP_ALIGN.CENTER, anchor=MSO_ANCHOR.MIDDLE)
boxr = s.shapes.add_shape(MSO_SHAPE.ROUNDED_RECTANGLE, In(0.6), In(5.9), In(12.13), In(0.62))
boxr.adjustments[0] = 0.5
boxr.fill.background(); boxr.line.color.rgb = BORDER; boxr.line.width = Pt(1); _noshadow(boxr)

# ================================================================ SLIDE 3 — architecture at a glance
s = new_slide()
chrome(s, "Architecture", ACCENT, "The platform at a glance",
       "Request path left→right; ingestion feeds the data tiers; intelligence trains on the cold tier and serves back.", 3)
# --- main request path
y = 2.55; h = 1.0
node(s, 0.45, y, 1.85, h, "Browser", "+ partner micro-frontends", color=TEAL)
node(s, 2.55, y, 1.8, h, "CloudFront", "edge cache + S3 static", color=TEAL)
node(s, 4.55, y, 1.15, h, "Cognito", "authorizer", color=YELLOW)
node(s, 5.9, y, 1.55, h, "API Gateway", "scoped routes", color=ACCENT)
node(s, 7.65, y - 0.12, 1.95, h + 0.24, "Context Lambdas", "one per context · CQRS inside", color=PURPLE)
# storage tiers (right, stacked)
node(s, 9.95, 2.18, 1.75, 0.82, "DynamoDB", "hot · small · recent", color=ORANGE)
node(s, 9.95, 3.18, 1.75, 0.82, "S3 Tables", "cold · massive · Iceberg", color=GREEN)
# intelligence
node(s, 11.95, 2.68, 1.3, 0.82, "Inference", "models", color=CYAN)
# arrows main path
arrow(s, 2.30, y + h/2, 2.55, y + h/2, TEAL)
arrow(s, 4.35, y + h/2, 4.55, y + h/2, TEAL)
arrow(s, 5.70, y + h/2, 5.9, y + h/2, YELLOW)
arrow(s, 7.45, y + h/2, 7.65, y + h/2, ACCENT)
arrow(s, 9.60, y + h/2, 9.95, 2.59, PURPLE)   # -> DynamoDB
arrow(s, 9.60, y + h/2, 9.95, 3.59, PURPLE)   # -> S3 Tables
arrow(s, 11.70, 3.45, 11.95, 3.05, GREEN)     # S3 Tables -> Inference (train)
arrow(s, 12.55, 2.68, 11.70, 2.5, CYAN, dash=True)  # Inference -> DynamoDB (cache preds)
text(s, 11.75, 2.18, 1.5, 0.25, "predictions", size=7.5, color=CYAN)
text(s, 11.78, 3.18, 1.4, 0.22, "train", size=7.5, color=GREEN)
# --- ingestion lane (bottom)
hline(s, 0.45, 12.85, 4.62, FAINT, 1.0)
text(s, 0.45, 4.72, 4.0, 0.3, "DATA INGESTION", size=9.5, bold=True, color=MUTED)
iy = 5.15; ih = 0.82
node(s, 0.45, iy, 2.0, ih, "Sources", "LoRaWAN/MQTT/HTTPS", color=PURPLE, title_size=11)
node(s, 2.65, iy, 1.4, ih, "Kinesis", "stream", color=ACCENT, title_size=11)
node(s, 4.25, iy, 1.65, ih, "Flink", "streaming enrich", color=ACCENT, title_size=11)
node(s, 6.1, iy, 1.7, ih, "Glue", "rollup · late-data", color=ORANGE, title_size=11)
arrow(s, 2.45, iy + ih/2, 2.65, iy + ih/2, ACCENT)
arrow(s, 4.05, iy + ih/2, 4.25, iy + ih/2, ACCENT)
arrow(s, 5.90, iy + ih/2, 6.1, iy + ih/2, ORANGE)
arrow(s, 5.9, iy, 10.0, 3.9, GREEN, width=1.0)         # Flink -> S3 Tables (Iceberg)
arrow(s, 7.8, iy, 10.4, 3.0, ORANGE, width=1.0, dash=True)  # Glue -> DynamoDB (materialized view)
text(s, 8.0, 4.85, 2.4, 0.25, "Iceberg append-only", size=8, color=GREEN)
text(s, 8.1, 3.95, 2.6, 0.25, "hourly materialized view", size=8, color=ORANGE)
# legend
text(s, 9.6, 6.55, 3.7, 0.9,
     "Account 339712745226 — frontend, API, hierarchy\nAccount 891377204778 — pipeline, S3 Tables, aggregations\nBoth eu-central-1",
     size=9, color=MUTED, spacing=1.15)

# ================================================================ SLIDE 4 — slim static frontend
s = new_slide()
chrome(s, "Frontend", TEAL, "A slim, mostly-static edge",
       "Astro builds to static assets on S3, served from CloudFront. Frontend operations approach zero.", 4)
bullets(s, 0.6, 2.35, 6.6, [
    ("Static build → S3::Astro compiles to HTML/CSS/JS; no server-side runtime to operate or scale.", TEAL),
    ("CloudFront edge::Global cache; the same artifact is served everywhere with low latency.", TEAL),
    ("HTML-over-the-wire::HTMX hypermedia — interactions fetch server-rendered HTML fragments, not client-side JSON rendering.", TEAL),
    ("Content-only deploys::A deploy swaps the asset bundle; the bucket and distribution are untouched (RETAIN). Trivial, instant rollback.", TEAL),
    ("API via the edge::CloudFront proxies each context's namespace (/hierarchy/*, /aggregations/*) to the API — calls stay relative, no CORS dance.", TEAL),
], size=13.5, marker_color=TEAL, space_after=11)
# mini diagram
bx = 7.7
node(s, bx, 2.7, 1.7, 0.85, "Browser", color=TEAL)
node(s, bx + 2.05, 2.7, 1.9, 0.85, "CloudFront", "edge cache", color=TEAL)
node(s, bx + 2.05, 3.95, 1.9, 0.85, "S3", "static assets", color=GREEN)
node(s, bx + 2.05, 5.2, 1.9, 0.85, "API Gateway", "proxied paths", color=ACCENT)
arrow(s, bx + 1.7, 3.12, bx + 2.05, 3.12, TEAL)
arrow(s, bx + 3.0, 3.55, bx + 3.0, 3.95, GREEN)
arrow(s, bx + 3.0, 4.8, bx + 3.0, 5.2, ACCENT, dash=True)
text(s, bx + 0.05, 6.2, 4.0, 0.6, "Static path cached at the edge; only dynamic\ncalls reach the API.", size=10, color=MUTED, spacing=1.12)
text(s, 0.6, 6.55, 6.6, 0.5, "Result: frontend ops ≈ 0. Cost scales with bytes served, not servers run.",
     size=13, bold=True, color=TEXT)

# ================================================================ SLIDE 5 — micro-frontend option
s = new_slide()
chrome(s, "Extensibility — Optional", PURPLE, "Partner micro-frontends, by choice",
       "The core ships as one static app. A host shell lets partners mount features — opt-in, never required.", 5)
# host shell diagram
hx, hy = 0.7, 2.45
shell = s.shapes.add_shape(MSO_SHAPE.ROUNDED_RECTANGLE, In(hx), In(hy), In(6.0), In(3.9))
shell.adjustments[0] = 0.04
shell.fill.solid(); shell.fill.fore_color.rgb = SURF2
shell.line.color.rgb = PURPLE; shell.line.width = Pt(1.4); _noshadow(shell)
text(s, hx + 0.25, hy + 0.18, 5.5, 0.4, "Host shell  (Enity core)", size=13, bold=True, color=PURPLE)
text(s, hx + 0.25, hy + 0.62, 5.5, 0.4, "routing · auth context · design system · slots", size=10.5, color=MUTED)
mods = [("Core views", GREEN), ("Partner A module", TEAL), ("Partner B module", ORANGE), ("3rd-party widget", CYAN)]
for i, (mname, mc) in enumerate(mods):
    col = i % 2; row = i // 2
    node(s, hx + 0.3 + col * 2.78, hy + 1.25 + row * 1.15, 2.6, 0.92, mname,
         "mounted at a slot", color=mc, title_size=12)
bullets(s, 7.15, 2.45, 5.6, [
    ("Slim by default::Core is a single static bundle. Federation is an option you switch on, not a tax everyone pays.", PURPLE),
    ("Composition models::Web components, module federation, or iframed islands — partners ship independently.", PURPLE),
    ("Scoped to their surface::A partner module can only call the API its Cognito scopes allow — isolation holds in the browser.", PURPLE),
    ("Ecosystem, not a fork::Partners add functionality without branching the core or gating on our release train.", PURPLE),
], size=13, marker_color=PURPLE, space_after=12)
text(s, 7.15, 6.35, 5.6, 0.5, "Decision: keep the option open in the architecture; ship core without it.",
     size=12.5, bold=True, color=TEXT)

# ================================================================ SLIDE 6 — identity & scoped access
s = new_slide()
chrome(s, "Security", YELLOW, "Cognito-scoped access at the edge",
       "Authorization is resolved at API Gateway — before business logic. Least privilege, per tenant.", 6)
# flow
y = 2.7
node(s, 0.55, y, 2.0, 1.0, "Cognito", "user pool · tokens", color=YELLOW)
node(s, 3.0, y, 2.1, 1.0, "API Gateway", "JWT authorizer", color=ACCENT)
node(s, 5.55, y, 2.1, 1.0, "Scoped routes", "tenant / partner claims", color=GREEN)
node(s, 8.1, y, 2.0, 1.0, "CQRS Lambda", "runs trusted", color=PURPLE)
arrow(s, 2.55, y + 0.5, 3.0, y + 0.5, YELLOW)
arrow(s, 5.1, y + 0.5, 5.55, y + 0.5, ACCENT)
arrow(s, 7.65, y + 0.5, 8.1, y + 0.5, GREEN)
text(s, 0.55, y + 1.15, 9.6, 0.4, "Invalid or unscoped token → rejected at the gateway; no Lambda invocation, no cost.",
     size=11, color=MUTED, italic=True)
bullets(s, 0.6, 4.55, 12.1, [
    ("Edge enforcement::Tokens are validated by an API Gateway authorizer; unauthorized requests never reach compute.", YELLOW),
    ("Per-tenant scopes::Claims bound a caller to its company/partner subtree — least privilege by construction.", YELLOW),
    ("In-request provisioning::The hierarchy Lambda creates/deletes Cognito users synchronously, with rollback — no out-of-band sync lag.", YELLOW),
    ("Account isolation::Backend (339712745226) and data pipeline (891377204778) are separate AWS accounts — blast radius is bounded.", YELLOW),
], size=13, marker_color=YELLOW, space_after=10)

# ================================================================ SLIDE 7 — API & CQRS (one Lambda per bounded context)
s = new_slide()
chrome(s, "API · CQRS", ACCENT, "One gateway, one Lambda per bounded context",
       "API Gateway maps /<context>/command and /<context>/query to the same function — CQRS is a path split, not a compute split.", 7)
node(s, 5.4, 2.3, 2.5, 0.82, "API Gateway", "single front door", color=ACCENT, title_size=14)

def context_box(x, name, sub, color, routes, note=None, dashed=False):
    w, y, h = 3.85, 3.75, 2.25
    box = s.shapes.add_shape(MSO_SHAPE.ROUNDED_RECTANGLE, In(x), In(y), In(w), In(h))
    box.adjustments[0] = 0.05
    box.fill.solid(); box.fill.fore_color.rgb = SURF2
    box.line.color.rgb = color; box.line.width = Pt(1.4)
    if dashed:
        ln = box.line._get_or_add_ln(); ln.append(ln.makeelement(qn('a:prstDash'), {'val': 'dash'}))
    _noshadow(box)
    text(s, x + 0.22, y + 0.15, w - 0.4, 0.36, name, size=13.5, bold=True, color=color)
    text(s, x + 0.22, y + 0.52, w - 0.4, 0.3, sub, size=9.5, italic=True, color=MUTED)
    ry = y + 0.92
    for verb, path, kind, kc in routes:
        text(s, x + 0.22, ry, 0.72, 0.32, verb, size=10, bold=True, color=kc, anchor=MSO_ANCHOR.MIDDLE)
        text(s, x + 0.92, ry, w - 1.1, 0.32, path, size=11.5, bold=True, color=TEXT, anchor=MSO_ANCHOR.MIDDLE)
        text(s, x + 0.22, ry + 0.30, w - 0.4, 0.26, kind, size=9, color=MUTED)
        ry += 0.58
    if note:
        text(s, x + 0.22, y + h - 0.34, w - 0.4, 0.3, note, size=9.5, italic=True, color=MUTED)
    return x + w / 2

cx1 = context_box(0.6, "Hierarchy", "one Lambda · command + query", PURPLE, [
    ("POST", "/hierarchy/command", "writes · validation · Cognito", ORANGE),
    ("GET", "/hierarchy/query/*", "reads · cacheable", GREEN),
])
cx2 = context_box(4.74, "Aggregations", "one Lambda · query-only today", CYAN, [
    ("GET", "/aggregations/query", "reads · JSON + HTML", GREEN),
], note="no command side — add it without a new Lambda")
cx3 = context_box(8.88, "… next context", "= one more Lambda", MUTED, [
    ("POST", "/<ctx>/command", "writes", ORANGE),
    ("GET", "/<ctx>/query", "reads", GREEN),
], dashed=True)

arrow(s, 6.05, 3.12, cx1, 3.75, PURPLE)
arrow(s, 6.65, 3.12, cx2, 3.75, CYAN)
arrow(s, 7.25, 3.12, cx3, 3.75, MUTED)

text(s, 0.6, 6.55, 12.1, 0.7,
     "Command and query are different API Gateway paths into the same Lambda. One Lambda per bounded context — "
     "fewer Datadog-instrumented functions, lower observability cost, smaller security surface.",
     size=12.5, bold=True, color=TEXT, align=PP_ALIGN.CENTER)

# ================================================================ SLIDE 8 — content negotiation
s = new_slide()
chrome(s, "Content Negotiation", CYAN, "One GET, many shapes",
       "The same query endpoint returns JSON or an HTML fragment by parameter — and Avro can be added later.", 8)
grid_table(s, 0.6, 2.45, 12.1, [0.16, 0.30, 0.54],
    ["Format", "Chosen by", "Consumer & use"],
    [
        [("JSON", ACCENT), "?format=json / Accept", "Charts, programmatic clients, partner integrations — structured data."],
        [("HTML", TEAL), "?format=html / Accept", "HTMX hypermedia UI — server-rendered fragments swapped straight into the page."],
        [("Avro", PURPLE), "(future) Accept", "High-volume / binary transport — added without changing the route surface."],
    ], row_h=0.82, body_size=12.5)
bullets(s, 0.6, 5.55, 12.1, [
    ("Hypermedia-first::HTML-over-the-wire keeps rendering on the server; the browser stays thin and the UI stays in lockstep with the model.", CYAN),
    ("Same contract, many encodings::Consumers pick the representation; the endpoint, auth, and query logic are shared.", CYAN),
    ("Forward-compatible::Avro slots in as another representation — no breaking change for existing JSON/HTML callers.", CYAN),
], size=12.5, marker_color=CYAN, space_after=9)

# ================================================================ SLIDE 9 — compute: Rust on Graviton
s = new_slide()
chrome(s, "Compute", PURPLE, "Rust on Graviton",
       "arm64 Lambdas (provided.al2023) built with cargo-lambda — predictable latency at low cost per request.", 9)
bullets(s, 0.6, 2.4, 6.5, [
    ("Why Rust + arm64::Low cold start, high throughput per dollar, memory safety, tiny binaries — a strong fit for per-request billing.", PURPLE),
    ("One Lambda per bounded context::A single function serves both command and query for its context. Fewer Lambdas means fewer Datadog-instrumented functions — directly lower observability cost — and a smaller security surface.", PURPLE),
    ("Synchronous where it matters::The hierarchy service provisions Cognito in-request with rollback — no eventual-consistency gaps.", PURPLE),
    ("Server-side heavy lifting::Dedup, max-by-recency, and aggregation run in the Lambda, not the browser.", PURPLE),
], size=13, marker_color=PURPLE, space_after=12)
# proof points
card(s, 7.4, 2.4, 5.3, 1.85, "rust-lambda-hierarchy",
     "One Lambda, its own namespace: POST /hierarchy/command + GET /hierarchy/query/* route to the same function. Synchronous Cognito create/delete with rollback.",
     PURPLE, title_size=13.5, body_size=11.5)
card(s, 7.4, 4.45, 5.3, 1.95, "measurements-aggregations-api",
     "One Lambda, query-only context: /aggregations/query (JSON from DynamoDB) plus a /measurements HTML fragment (S3 Tables via Athena). A command path can be added to the same function.",
     CYAN, title_size=13.5, body_size=11.5)
text(s, 0.6, 6.5, 6.5, 0.5, "Net: predictable latency, lower cost/request than JVM or Node baselines.",
     size=12.5, bold=True, color=TEXT)

# ================================================================ SLIDE 10 — storage tiering
s = new_slide()
chrome(s, "Storage", GREEN, "Two tiers, one decision rule",
       "Latency need picks the store: hot/small/recent → DynamoDB; massive/historical/analytical → S3 Tables.", 10)
# two columns
node(s, 0.7, 2.45, 5.7, 0.7, "DynamoDB  —  hot tier", color=ORANGE, title_size=15, fill=SURF2)
bullets(s, 0.95, 3.35, 5.4, [
    ("Single-digit-ms reads::Serves the interactive, recent slice of data.", ORANGE),
    ("Tables::hierarchy_new, measurements_aggregate (rollups, TTL), meter-identity.", ORANGE),
    ("On-demand::Scales with traffic; nothing provisioned idle.", ORANGE),
    ("Fed by rollups::Glue writes hourly/day materialized views here.", ORANGE),
], size=12.5, marker_color=ORANGE, space_after=9)
node(s, 6.95, 2.45, 5.7, 0.7, "S3 Tables (Iceberg)  —  cold tier", color=GREEN, title_size=15, fill=SURF2)
bullets(s, 7.2, 3.35, 5.4, [
    ("Unbounded history::Years of readings, append-only, cheap at rest.", GREEN),
    ("Tables::raw_data, logical_meter_data.", GREEN),
    ("Queried analytically::Athena ⇄ Redshift Spectrum (toggle) for big scans.", GREEN),
    ("Training substrate::The dataset the inference models learn from.", GREEN),
], size=12.5, marker_color=GREEN, space_after=9)
text(s, 0.6, 6.55, 12.1, 0.55,
     "The same query Lambda chooses the tier by the latency the response needs — callers don't know or care which store answered.",
     size=12.5, bold=True, color=TEXT, align=PP_ALIGN.CENTER)

# ================================================================ SLIDE 11 — write model -> read model
s = new_slide()
chrome(s, "Data Flow", ACCENT, "One write model, two read tiers",
       "An append-only streaming pipeline populates both the hot and cold tiers the API reads from.", 11)
y = 2.7; h = 1.0
node(s, 0.5, y, 1.8, h, "Sources", "meters · APIs · CSV", color=PURPLE)
node(s, 2.55, y, 1.5, h, "Kinesis", "ingest stream", color=ACCENT)
node(s, 4.3, y, 1.8, h, "Flink", "streaming enrich", color=ACCENT)
node(s, 6.35, y, 2.0, h, "S3 Tables", "Iceberg · append-only", color=GREEN)
node(s, 8.95, 2.2, 2.0, 0.95, "Glue rollup", "hourly / late-data", color=ORANGE)
node(s, 11.2, 2.2, 1.9, 0.95, "DynamoDB", "materialized view", color=ORANGE)
node(s, 8.95, 3.7, 4.15, 0.95, "Context query paths + Inference", "read both tiers", color=ACCENT)
arrow(s, 2.3, y + 0.5, 2.55, y + 0.5, ACCENT)
arrow(s, 4.05, y + 0.5, 4.3, y + 0.5, ACCENT)
arrow(s, 6.1, y + 0.5, 6.35, y + 0.5, ACCENT)
arrow(s, 8.35, 2.9, 8.95, 2.67, GREEN)            # S3 Tables -> Glue
arrow(s, 10.95, 2.67, 11.2, 2.67, ORANGE)         # Glue -> DynamoDB
arrow(s, 8.35, 3.3, 9.6, 3.7, GREEN)              # S3 Tables -> read (cold path)
arrow(s, 12.15, 3.15, 11.6, 3.7, ORANGE)          # DynamoDB -> read (hot path)
text(s, 0.6, 5.2, 12.1, 0.4, "Correctness by construction", size=13, bold=True, color=TEXT)
bullets(s, 0.6, 5.6, 12.1, [
    ("Append-only::Corrections are new events with a later timestamp; consumers read max-by-recency. No destructive updates.", ACCENT),
    ("Streaming-first::Flink handles real time; Glue recomputes only late arrivals (>6h) and backfills.", ACCENT),
    ("One source of truth::Both read tiers derive from the same write model — the hot view is just a rollup of the cold history.", ACCENT),
], size=12, marker_color=ACCENT, space_after=7)

# ================================================================ SLIDE 12 — intelligence layer
s = new_slide()
chrome(s, "Intelligence", CYAN, "Models trained on your own data",
       "The cold tier isn't just storage — it's the training substrate. Predictions flow back through the same API.", 12)
# loop diagram
node(s, 0.6, 2.55, 2.3, 1.0, "S3 Tables", "raw_data · logical_meter_data", color=GREEN, title_size=13)
node(s, 3.35, 2.55, 2.3, 1.0, "Training", "batch · Athena/Spark", color=PURPLE, title_size=13)
node(s, 6.1, 2.55, 2.3, 1.0, "Model / endpoint", "inference service", color=CYAN, title_size=13)
node(s, 8.85, 2.05, 2.0, 0.8, "DynamoDB", "cached predictions", color=ORANGE, title_size=12)
node(s, 8.85, 3.05, 2.0, 0.8, "Scoped API", "same gateway", color=ACCENT, title_size=12)
node(s, 11.1, 2.55, 1.85, 1.0, "Frontend", "charts · HTML", color=TEAL, title_size=13)
arrow(s, 2.9, 3.05, 3.35, 3.05, GREEN)
arrow(s, 5.65, 3.05, 6.1, 3.05, PURPLE)
arrow(s, 8.4, 2.85, 8.85, 2.55, CYAN)             # endpoint -> DynamoDB cache
arrow(s, 8.4, 3.15, 8.85, 3.45, CYAN)             # endpoint -> API
arrow(s, 10.85, 2.55, 11.1, 2.95, ORANGE)         # cache -> frontend
arrow(s, 10.85, 3.45, 11.1, 3.15, ACCENT)         # API -> frontend
bullets(s, 0.6, 4.05, 12.1, [
    ("Trained on history::Years of metered readings in S3 Tables feed forecasting, anomaly & leak detection, load disaggregation, and peer benchmarking.", CYAN),
    ("Served through the same door::Inference sits behind the same Cognito-scoped API Gateway — one auth model, one contract.", CYAN),
    ("Hot when it must be::Frequent predictions are cached into DynamoDB so the UI reads them at single-digit-ms latency.", CYAN),
    ("Consumed like any query::The frontend renders predictions as JSON charts or HTML fragments — no special client path.", CYAN),
    ("Closes the loop::cold data → models → hot predictions → UI — the platform's data compounds into a product advantage.", CYAN),
], size=12.5, marker_color=CYAN, space_after=8)

# ================================================================ SLIDE 13 — request lifecycle
s = new_slide()
chrome(s, "Lifecycle", ACCENT, "One request, end to end",
       "From click to rendered fragment — the path every interactive read takes.", 13)
steps = [
    ("1", "Browser request", "User action triggers an HTMX/JSON fetch to a relative path.", TEAL),
    ("2", "CloudFront", "Serves from edge cache, or proxies the dynamic call onward.", TEAL),
    ("3", "Cognito @ API Gateway", "Authorizer validates the token and resolves tenant scope.", YELLOW),
    ("4", "Route → context Lambda", "Gateway routes by path (e.g. /hierarchy/query) to that context's one Lambda.", ACCENT),
    ("5", "Tier select", "Hot → DynamoDB · cold → S3 Tables via Athena · predicted → inference.", GREEN),
    ("6", "Render & return", "Lambda emits JSON or an HTML fragment per the format parameter.", PURPLE),
]
yy = 2.45
for num, title, body, c in steps:
    badge = s.shapes.add_shape(MSO_SHAPE.OVAL, In(0.7), In(yy), In(0.52), In(0.52))
    badge.fill.solid(); badge.fill.fore_color.rgb = SURFACE
    badge.line.color.rgb = c; badge.line.width = Pt(1.5); _noshadow(badge)
    tf = badge.text_frame; tf.vertical_anchor = MSO_ANCHOR.MIDDLE
    p = tf.paragraphs[0]; p.alignment = PP_ALIGN.CENTER
    r = p.add_run(); r.text = num; r.font.size = Pt(15); r.font.bold = True; r.font.color.rgb = c; r.font.name = FONT
    text(s, 1.45, yy - 0.04, 3.1, 0.6, title, size=14.5, bold=True, color=c, anchor=MSO_ANCHOR.MIDDLE)
    text(s, 4.75, yy - 0.04, 8.0, 0.6, body, size=12.5, color=MUTED, anchor=MSO_ANCHOR.MIDDLE)
    if num != "6":
        arrow(s, 0.96, yy + 0.52, 0.96, yy + 0.72, c)
    yy += 0.72

# ================================================================ SLIDE 14 — why this architecture
s = new_slide()
chrome(s, "Value", GREEN, "Why this shape wins",
       "Every layer was chosen for cost, scale, security, or optionality — and they compound.", 14)
items = [
    ("Cost", "Pay-per-use end to end. Scale-to-zero compute, on-demand storage, edge cache — no idle fleet.", GREEN),
    ("Scale", "Edge caching + serverless concurrency absorb spikes; S3 Tables holds unbounded history cheaply.", ACCENT),
    ("Security", "Cognito-scoped at the edge; multi-account isolation bounds blast radius.", YELLOW),
    ("Ecosystem", "Partner micro-frontends + scoped API let others build without forking the core.", PURPLE),
    ("Evolvability", "Add Avro, add a model, add a partner module — no re-platforming, no broken contracts.", TEAL),
    ("Intelligence", "Models trained on proprietary metered history turn data exhaust into product advantage.", CYAN),
]
cw, ch, gx, gy = 3.95, 1.7, 0.22, 0.24
x0, y0 = 0.6, 2.4
for i, (t, b, c) in enumerate(items):
    col = i % 3; row = i // 3
    card(s, x0 + col * (cw + gx), y0 + row * (ch + gy), cw, ch, t, b, c, title_size=15, body_size=11.5)
text(s, 0.6, 6.65, 12.1, 0.5,
     "Thin layers, clean seams: each can change on its own clock.",
     size=13, bold=True, color=TEXT, align=PP_ALIGN.CENTER)

# ================================================================ SLIDE 15 — maturity map
s = new_slide()
chrome(s, "Maturity", ACCENT, "Live today vs. optional / roadmap",
       "An honest map — the spine exists now; intelligence and partner extension are deliberate next moves.", 15)
grid_table(s, 0.6, 2.4, 12.1, [0.60, 0.18, 0.22],
    ["Capability", "Status", "Notes"],
    [
        ["Slim static frontend (S3 / CloudFront)", ("● Live", GREEN), ("Astro + HTMX, content-only deploys", MUTED)],
        ["Cognito-scoped API Gateway", ("● Live", GREEN), ("Authorizer + per-tenant scopes", MUTED)],
        ["CQRS Rust / Graviton Lambdas", ("● Live", GREEN), ("One Lambda per context; /command + /query paths", MUTED)],
        ["JSON + HTML by parameter", ("● Live", GREEN), ("Hypermedia + structured clients", MUTED)],
        ["DynamoDB + S3 Tables tiering", ("● Live", GREEN), ("Hot rollups + cold Iceberg history", MUTED)],
        ["Streaming pipeline → Iceberg → rollups", ("● Live", GREEN), ("Flink + Glue, append-only", MUTED)],
        ["Avro responses", ("● Planned", ORANGE), ("Added as another representation", MUTED)],
        ["Partner micro-frontends", ("● Optional", YELLOW), ("Host shell, opt-in federation", MUTED)],
        ["Inference services on S3 Tables", ("● Roadmap", CYAN), ("Train cold → serve hot via API", MUTED)],
    ], row_h=0.45, body_size=12)

# ================================================================ SLIDE 16 — closing
s = new_slide()
bar = s.shapes.add_shape(MSO_SHAPE.RECTANGLE, In(0), In(7.34), In(13.333), In(0.16))
bar.fill.solid(); bar.fill.fore_color.rgb = ACCENT; bar.line.fill.background(); _noshadow(bar)
text(s, 0, 2.0, 13.333, 0.5, "THE VISION", size=15, bold=True, color=ACCENT, align=PP_ALIGN.CENTER)
text(s, 1.4, 2.7, 10.5, 2.2,
     "A slim static edge, a scoped serverless API, and intelligence\ntrained on our own data —\nextensible by partners, billed by the request.",
     size=29, bold=True, color=TEXT, align=PP_ALIGN.CENTER, spacing=1.18)
hline(s, 4.9, 8.43, 5.25, BORDER, 1.0)
text(s, 0, 5.5, 13.333, 0.5,
     "Frontend  →  API  →  Intelligence  →  Data",
     size=16, color=MUTED, align=PP_ALIGN.CENTER)

# ---------------------------------------------------------------- save
import os
out = os.path.join(os.path.dirname(os.path.abspath(__file__)), "product-vision.pptx")
prs.save(out)
print(f"wrote {out} — {len(prs.slides._sldIdLst)} slides")
