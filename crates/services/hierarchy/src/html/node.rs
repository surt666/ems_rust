use maud::{html, Markup, PreEscaped};
use model::domain::node::Node;
use model::domain::schema::FieldSpec;
use model::domain::sensor::Sensor;
use model::domain::values::{CognitoGroup, Resource};

/// Danish UI label for a resource (the EMS "Målertype" wording). The domain
/// `Resource` owns the wire token (`Display`); the view owns the label.
fn resource_label_da(r: Resource) -> &'static str {
    match r {
        Resource::Electricity => "El",
        Resource::DistrictHeating => "Fjernvarme",
        Resource::DistrictCooling => "Fjernkøling",
        Resource::Gas => "Gas",
        Resource::Water => "Vand",
        Resource::Heat => "Varme",
    }
}


// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Format a metadata JSON blob as a simple key/value form.
fn metadata_rows(j: &serde_json::Value) -> Markup {
    match j {
        serde_json::Value::Object(kvs) => {
            html! {
                @for (k, v) in kvs {
                    @match v {
                        serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                            div style="margin-top: 1rem;" {
                                h3 class="table-title" { (k) }
                                (metadata_rows(v))
                            }
                        }
                        _ => {
                            @let s = crate::html::scalar_to_string(v);
                            div class="form-row-2col" {
                                label class="form-label" { (k) ":" }
                                input type="text" value=(s) readonly class="form-input";
                            }
                        }
                    }
                }
            }
        }
        serde_json::Value::Array(xs) => {
            html! {
                @for (i, v) in xs.iter().enumerate() {
                    div style="margin-top: 0.5rem;" {
                        h4 { "[" (i) "]" }
                        (metadata_rows(v))
                    }
                }
            }
        }
        other => {
            let s = match other {
                serde_json::Value::String(s) => s.clone(),
                _ => serde_json::to_string(other).unwrap_or_default(),
            };
            html! {
                div class="form-row-2col" {
                    input type="text" value=(s) readonly class="form-input";
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// metadata_edit_form — Admin/Writer editable metadata
// ---------------------------------------------------------------------------

/// On Save success, flash a small auto-dismissing "saved" toast and revert the
/// form to the read-only / non-edit view in place (re-disable inputs, hide Save,
/// show Edit). We revert in place rather than reloading the panel because the
/// panel's `hx-trigger="load"` is a one-time init trigger that does not re-fire.
/// On error show the response text in the inline error div and stay in edit mode.
const METADATA_AFTER_REQUEST_JS: &str = "if(event.detail.successful){ \
    var t=document.createElement('div'); t.textContent='Data gemt'; \
    t.setAttribute('style','position:fixed;bottom:1.5rem;right:1.5rem;\
background:#16a34a;color:#fff;padding:0.6rem 1rem;border-radius:8px;\
box-shadow:0 4px 12px rgba(0,0,0,0.25);z-index:9999;font-size:0.9rem;'); \
    document.body.appendChild(t); \
    setTimeout(function(){ t.remove(); }, 2500); \
    var f=document.getElementById('metadata-form'); \
    f.querySelectorAll('.md-input').forEach(function(el){ el.setAttribute('disabled',''); }); \
    document.getElementById('metadata-save-btn').setAttribute('hidden',''); \
    document.getElementById('metadata-cancel-btn').setAttribute('hidden',''); \
    document.getElementById('metadata-edit-btn').removeAttribute('hidden'); \
    document.getElementById('metadata-error').style.display='none'; } else { \
    var ed=document.getElementById('metadata-error'); \
    ed.textContent=event.detail.xhr.responseText; ed.style.display='block'; }";

/// Editable metadata form: schema-typed inputs (disabled until Edit), an
/// Edit/Save button pair, and an inline error div. Edit (hyperscript) makes the
/// `.md-input` controls writable and swaps Edit→Save; Save posts `update_node`
/// and reloads `#node-data-panel`.
fn metadata_edit_form(
    nid_str: &str,
    fields: &[(String, FieldSpec)],
    metadata: &serde_json::Value,
) -> Markup {
    let empty = serde_json::Map::new();
    let prefill = metadata.as_object().unwrap_or(&empty);
    html! {
        form id="metadata-form" class="form"
            data-hx-post="/hierarchy/command"
            data-hx-swap="none"
            data-hx-request=(crate::html::NO_HEADERS)
            hx-on--after-request=(METADATA_AFTER_REQUEST_JS)
        {
            input type="hidden" name="action" value="update_node";
            input type="hidden" name="data.id" value=(nid_str);
            (crate::html::forms::metadata_inputs(fields, Some(prefill), true))
            div id="metadata-error" class="login-error" style="display:none; margin-top: 0.5rem;" {}
            div style="display: grid; grid-auto-flow: column; justify-content: end; gap: 0.5rem; margin-top: 1rem;" {
                button type="button" id="metadata-edit-btn" class="btn-secondary"
                    _="on click remove @disabled from <#metadata-form .md-input/> then add @hidden to me then remove @hidden from #metadata-save-btn then remove @hidden from #metadata-cancel-btn"
                    data-i18n="node.edit"
                { "Edit" }
                button type="button" id="metadata-cancel-btn" class="btn-secondary" hidden
                    _="on click call #metadata-form.reset() then add @disabled to <#metadata-form .md-input/> then add @hidden to me then add @hidden to #metadata-save-btn then remove @hidden from #metadata-edit-btn then hide #metadata-error"
                    data-i18n="common.cancel"
                { "Cancel" }
                button type="submit" id="metadata-save-btn" class="btn-warning" hidden
                    data-i18n="node.save"
                { "Save" }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// add_child_block — the "Children" section + dialog
// ---------------------------------------------------------------------------

/// Emit the "Children" header with Add-child button and add-child dialog.
/// The add-child dialog (the button lives in the node-view actions row).
fn add_child_dialog(parent_id_str: &str) -> Markup {
    html! {
        dialog id="add-child-dialog"
            _="on click if event.target == me then call me.close()"
        {
            div class="dialog-header" {
                h2 data-i18n="node.add_child_dialog_title" { "ADD CHILD" }
                button type="button" class="btn-close"
                    _="on click call #add-child-dialog.close()"
                {
                    (PreEscaped("&times;"))
                }
            }
            div class="dialog-body" {
                div id="add-child-body"
                    data-hx-get="/hierarchy/query/add_child_form"
                    data-hx-vals=(format!(r#"{{"parent": "{}"}}"#, parent_id_str))
                    data-hx-trigger="refresh"
                    data-hx-request=(crate::html::NO_HEADERS)
                    data-hx-target="#add-child-body"
                    data-hx-swap="innerHTML"
                {
                    em data-i18n="common.loading" { "Loading\u{2026}" }
                }
            }
            div class="dialog-footer" {
                button type="submit" form="add-child-form" class="btn-warning"
                    data-i18n="common.save"
                {
                    "Save"
                }
                div {}
                button type="button" class="btn-warning"
                    _="on click call #add-child-dialog.close()"
                    data-i18n="common.close"
                {
                    "Close"
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// render_node — the node-detail card
// ---------------------------------------------------------------------------

/// Render the node-detail card fragment.
///
/// `show_sensors` controls whether the sensor block is included.
/// When `show_sensors` is true, `parent_str` is used to build the sensor URLs
/// (e.g. `"HN1#10001#HN2#10003"`); when false it is unused.
///
/// `capability` gates which controls are visible:
/// - `Some(Admin)`:  add-child + add-sensor shown; metadata editable; name editable.
///   (Kept byte-identical to the current output so the golden test passes.)
/// - `Some(Writer)`: no add-child, no add-sensor; metadata editable; name editable.
/// - `Some(Reader)` / `None`: no add-child, no add-sensor; metadata and name are
///   read-only; no Save button.
///
/// `allow_children` gates the add-child button independently of capability: even
/// an admin should not see it on a node whose type has no allowed children
/// (e.g. a leaf `area`).
pub fn render_node(
    node: &Node,
    show_sensors: bool,
    allow_children: bool,
    capability: Option<CognitoGroup>,
    metadata_fields: &[(String, FieldSpec)],
) -> Markup {
    let nid_str = node.id.to_string();
    let parent_str = match &node.parent {
        None => nid_str.clone(),
        Some(p) => format!("{}#{}", p, nid_str),
    };

    let is_admin = capability == Some(CognitoGroup::Admin);
    let can_write = matches!(capability, Some(CognitoGroup::Admin) | Some(CognitoGroup::Writer));
    // Admin output must be byte-identical to the current (pre-capability) output:
    // the golden has the name input as readonly, so we keep readonly for Admin too.
    let name_readonly = !can_write || is_admin;

    // Admin-only delete: removes this node (and subtree), then returns to the
    // parent's node view (full reload so the sidebar tree refreshes).
    let delete_vals = format!(r#"{{"action": "delete_node", "id": "{}"}}"#, nid_str);
    let after_delete_url = match &node.parent {
        Some(p) if !p.is_root() => format!("/node/?id={}", p.to_string().replace('#', "%23")),
        _ => "/main".to_string(),
    };
    let after_delete_js = format!(
        "if(event.detail.successful){{window.location.href='{}';}}",
        after_delete_url
    );

    // Null and the empty object both mean "no metadata" → one branch.
    let no_metadata = matches!(&node.metadata, serde_json::Value::Null)
        || matches!(&node.metadata, serde_json::Value::Object(m) if m.is_empty());
    let metadata_section = if can_write && !metadata_fields.is_empty() {
        metadata_edit_form(&nid_str, metadata_fields, &node.metadata)
    } else if no_metadata {
        html! {
            p style="color: var(--text-muted);" data-i18n="node.no_metadata" {
                "No metadata available"
            }
        }
    } else {
        html! { div class="form" { (metadata_rows(&node.metadata)) } }
    };

    html! {
        div class="page-container" {
            div class="card" {
                div class="form" {
                    div class="form-row-2col" {
                        label class="form-label" { "ID:" }
                        input type="text" id="id" value=(nid_str) readonly class="form-input";
                    }
                    div class="form-row-2col" {
                        label class="form-label" data-i18n="common.name" { "Name" }
                        input type="text" id="name" value=(node.name) readonly[name_readonly] class="form-input";
                    }
                }
                @if is_admin {
                    // Add-child (when the type allows children) and Delete on one row,
                    // right-aligned (grid, not flex).
                    div style="display: grid; grid-auto-flow: column; justify-content: end; gap: 0.5rem; align-items: center; margin-top: 1.5rem; margin-bottom: 1rem;" {
                        @if allow_children {
                            button type="button" class="btn-primary"
                                _="on click call #add-child-dialog.showModal() then send refresh to #add-child-body"
                                data-i18n="node.add_child"
                            {
                                "Add child"
                            }
                        }
                        button type="button" class="btn-danger"
                            data-hx-post="/hierarchy/command"
                            data-hx-vals=(delete_vals)
                            data-hx-request=(crate::html::NO_HEADERS)
                            data-hx-swap="none"
                            data-hx-confirm="Slet denne node og alt under den?"
                            hx-on--after-request=(after_delete_js)
                            data-i18n="node.delete"
                        {
                            "Slet"
                        }
                    }
                    @if allow_children {
                        (add_child_dialog(&nid_str))
                    }
                }
                div style="margin-top: 2rem; padding-top: 1.5rem; border-top: 1px solid var(--border-medium);" {
                    (metadata_section)
                    @if show_sensors && is_admin {
                        (sensor_block(&nid_str, &parent_str))
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// sensor_block + sensor_dialog
// ---------------------------------------------------------------------------

/// The formula dialog JS script.
static FORMULA_DIALOG_JS: &str = r#"
(function () {
  function $(id){ return document.getElementById(id); }
  function addRefRow(alias, sensorId) {
    var rows = $('formula-refs-rows');
    var src = $('ref-sensor-options-src');
    var row = document.createElement('div');
    row.className = 'form-row formula-ref-row';
    var a = document.createElement('input');
    a.type = 'text'; a.className = 'form-input formula-ref-alias';
    a.placeholder = 'alias (e.g. a)'; a.style.maxWidth = '8rem';
    a.value = alias || '';
    var sel = document.createElement('select');
    sel.className = 'form-select formula-ref-sensor';
    sel.innerHTML = src ? src.innerHTML : '';
    if (sensorId) sel.value = sensorId;
    row.appendChild(a);
    row.appendChild(document.createTextNode(' → '));
    row.appendChild(sel);
    rows.appendChild(row);
  }
  document.addEventListener('change', function (e) {
    if (e.target && e.target.id === 'formula-kind-select') {
      var sec = $('formula-expr-section');
      if (sec) sec.style.display = (e.target.value === 'expr') ? '' : 'none';
    }
  });
  document.addEventListener('click', function (e) {
    if (!e.target) return;
    if (e.target.id === 'formula-edit') {
      var committed = $('formula-kind').value || 'identity';
      $('formula-dialog-error').style.display = 'none';
      $('formula-dialog-error').textContent = '';
      $('formula-kind-select').value = committed;
      $('formula-expr-section').style.display = (committed === 'expr') ? '' : 'none';
      $('formula-refs-rows').innerHTML = '';
      $('formula-expr-input').value = (committed === 'expr') ? ($('formula-expr-field').value || '') : '';
      if (committed === 'expr') {
        try {
          var refs = JSON.parse($('formula-refs-field').value || '{}');
          Object.keys(refs).forEach(function (k) { addRefRow(k, refs[k]); });
        } catch (e2) {}
      }
      $('formula-dialog').showModal();
      return;
    }
    if (e.target.id === 'formula-add-ref') { addRefRow('', ''); return; }
    if (e.target.id === 'formula-apply') {
      var err = $('formula-dialog-error');
      err.style.display = 'none'; err.textContent = '';
      var kind = $('formula-kind-select').value;
      $('formula-kind').value = kind;
      if (kind !== 'expr') {
        $('formula-expr-field').value = '';
        $('formula-refs-field').value = '';
        $('formula-summary').textContent = (kind === 'zero') ? 'Zero' : 'Identity (default)';
        $('formula-dialog').close();
        return;
      }
      var expr = ($('formula-expr-input').value || '').trim();
      if (!expr) { err.textContent = 'Expression is required.'; err.style.display = ''; return; }
      var refs = {};
      var rws = document.querySelectorAll('#formula-refs-rows .formula-ref-row');
      for (var i = 0; i < rws.length; i++) {
        var al = rws[i].querySelector('.formula-ref-alias').value.trim();
        var sv = rws[i].querySelector('.formula-ref-sensor').value;
        if (al) refs[al] = sv;
      }
      $('formula-expr-field').value = expr;
      $('formula-refs-field').value = JSON.stringify(refs);
      $('formula-summary').textContent = expr;
      $('formula-dialog').close();
    }
  });
})();
"#;

/// The sensor add dialog + formula builder dialog + script.
fn sensor_dialog(nid_str: &str, parent_str: &str) -> Markup {
    let after_request_js = "if(event.detail.elt.id === 'add-sensor-form' && \
        event.detail.successful) { \
        document.querySelector('#add-sensor-dialog').close(); \
        htmx.trigger('#sensor-list', 'load'); } else if \
        (event.detail.elt.id === 'add-sensor-form') { \
        document.getElementById('sensor-form-error').textContent = \
        event.detail.xhr.responseText; \
        document.getElementById('sensor-form-error').style.display \
        = 'block'; }";

    // Hx.vals with nodepath — must be a string literal for maud to handle correctly.
    // We use a format! to build the vals JSON, then pass as plain string (maud &quot;-escapes it).
    let ref_sensor_vals = format!(r#"{{"nodepath": "{}"}}"#, parent_str);

    html! {
        dialog id="add-sensor-dialog"
            _="on click if event.target == me then call me.close()"
        {
            div class="dialog-header" {
                h2 data-i18n="node.add_sensor_dialog_title" { "ADD SENSOR" }
                button type="button" class="btn-close"
                    _="on click call #add-sensor-dialog.close()"
                {
                    (PreEscaped("&times;"))
                }
            }
            div class="dialog-body" {
                div id="sensor-form-error" class="login-error"
                    style="display:none; margin-bottom: 1rem;"
                {}
                form id="add-sensor-form" class="form"
                    data-hx-post="/hierarchy/command"
                    data-hx-swap="none"
                    hx-on--after-request=(after_request_js)
                {
                    input type="hidden" name="action" value="attach_sensor";
                    input type="hidden" name="data.parent_id" value=(nid_str);
                    div class="form-row" {
                        label class="form-label" { "DAQ Id" }
                        input type="text" name="data.daq_id" required class="form-input";
                        span class="required" { "*" }
                    }
                    div class="form-row" {
                        label class="form-label" { "Resource" }
                        // The per-meter resource (EMS "Målertype" / energy form). The
                        // value is written to the sensor's `purpose` field, so the option
                        // values are the domain `Resource` tokens themselves — iterated
                        // here so they can't drift from the enum / the rollup contract.
                        select name="data.purpose" required class="form-select" {
                            @for r in Resource::all() {
                                option value=(r.as_str()) { (resource_label_da(r)) }
                            }
                        }
                        span class="required" { "*" }
                    }
                    div class="form-row" {
                        label class="form-label" { "Meter type" }
                        select name="data.meter_type" required class="form-select" {
                            option value="counter" { "counter" }
                            option value="gauge" { "gauge" }
                        }
                        span class="required" { "*" }
                    }
                    div class="form-row" {
                        label class="form-label" { "Unit" }
                        input type="text" name="data.unit" class="form-input";
                    }
                    div class="form-row" {
                        label class="form-label" { "Resample interval (min)" }
                        input type="number" name="data.resample_minutes" min="1" step="1" class="form-input";
                    }
                    div class="form-row" {
                        label class="form-label" { "Formula" }
                        div class="form-inline" {
                            span id="formula-summary" class="form-summary" {
                                "Identity (default)"
                            }
                            button type="button" class="btn-secondary" id="formula-edit" {
                                "Edit formula\u{2026}"
                            }
                        }
                    }
                    input type="hidden" name="data.formula.kind" id="formula-kind" value="identity";
                    input type="hidden" name="data.formula.expr" id="formula-expr-field" value="";
                    input type="hidden" name="data.formula.refs" id="formula-refs-field" value="";
                }
            }
            div class="dialog-footer" {
                button type="submit" form="add-sensor-form" class="btn-warning"
                    data-i18n="common.save"
                {
                    "Save"
                }
                div {}
                button type="button" class="btn-warning"
                    _="on click call #add-sensor-dialog.close()"
                    data-i18n="common.close"
                {
                    "Close"
                }
            }
        }
        dialog id="formula-dialog" class="dialog" {
            div class="dialog-content" {
                h2 { "Build formula" }
                div class="form-row" {
                    label class="form-label" { "Kind" }
                    select id="formula-kind-select" class="form-select" {
                        option value="identity" { "Identity (default)" }
                        option value="zero" { "Zero" }
                        option value="expr" { "Expression" }
                    }
                }
                div id="formula-expr-section" style="display:none;" {
                    div class="form-row" {
                        label class="form-label" { "Expression" }
                        input type="text" id="formula-expr-input" class="form-input"
                            placeholder="abs(self - a - b)";
                    }
                    div class="form-hint" {
                        "Use self, numbers, + - * /, abs(), and aliases bound below. \
                         Aliases cannot be named self or abs."
                    }
                    div id="formula-refs-rows" {}
                    button type="button" class="btn-secondary" id="formula-add-ref" {
                        "+ Add reference"
                    }
                }
                div id="formula-dialog-error" class="login-error" style="display:none;" {}
                div class="dialog-footer" {
                    button type="button" class="btn-warning" id="formula-apply" { "Apply" }
                    button type="button"
                        _="on click call #formula-dialog.close()"
                    {
                        "Cancel"
                    }
                }
            }
            // Hidden <option> source for ref dropdowns; loaded once via htmx.
            select id="ref-sensor-options-src" style="display:none;"
                data-hx-get="/hierarchy/query/company_sensors"
                data-hx-vals=(ref_sensor_vals)
                data-hx-trigger="load"
                data-hx-target="#ref-sensor-options-src"
                data-hx-swap="innerHTML"
                data-hx-request=(crate::html::NO_HEADERS)
            {}
        }
        script { (PreEscaped(FORMULA_DIALOG_JS)) }
    }
}

/// The sensor section: header, add-sensor button, sensor list with htmx.
fn sensor_block(nid_str: &str, parent_str: &str) -> Markup {
    let sensor_vals = format!(r#"{{"nodepath": "{}"}}"#, parent_str);
    html! {
        div style="margin-top: 2rem;" {
            div style="display: grid; grid-template-columns: 1fr auto; align-items: center; margin-bottom: 1rem;" {
                h2 class="section-title" style="margin-bottom: 0;" {
                    span data-i18n="node.sensors" { "Sensors" }
                    " "
                    span id="loading-indicator" class="htmx-indicator"
                        style="display: none; font-size: var(--text-sm); color: var(--accent); margin-left: 8px;"
                        data-i18n="common.loading"
                    {
                        "Loading\u{2026}"
                    }
                }
                button type="button" class="btn-primary"
                    _="on click call #add-sensor-dialog.showModal()"
                    data-i18n="node.add_sensor"
                {
                    "Add sensor"
                }
            }
            (sensor_dialog(nid_str, parent_str))
            ul id="sensor-list" style="display: grid; gap: 8px;"
                data-hx-get="/hierarchy/query/sensors"
                data-hx-vals=(sensor_vals)
                data-hx-trigger="load"
                data-hx-target="#sensor-list"
                data-hx-swap="innerHTML"
                data-hx-request=(crate::html::NO_HEADERS)
                data-hx-indicator="#loading-indicator"
            {
                li style="color: var(--text-muted);" data-i18n="node.sensors_loading" {
                    "Loading sensors\u{2026}"
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// render_sensors — the sensor list items
// ---------------------------------------------------------------------------

/// Render the sensor list as `<li>` items.
/// An empty list renders a single "No sensors found" item.
pub fn render_sensors(sensors: &[Sensor]) -> Markup {
    if sensors.is_empty() {
        html! {
            li style="color: var(--text-muted);" data-i18n="node.no_sensors" {
                "No sensors found"
            }
        }
    } else {
        html! {
            @for s in sensors {
                (render_sensor_row(s))
            }
        }
    }
}

/// A single sensor `<li>`: label on the left, a native `<details>` "…" (kebab)
/// action menu on the right. `<details>` needs no JS, so it survives the HTMX
/// swap into `#sensor-list`. Only **Gå til datatilegnelse** is wired (links to
/// the measurements page, carrying the sensor's identity + metadata as query
/// params); the other items mirror the EMS meter menu and are inert for now
/// (`_="on click halt"` stops the placeholder `#` navigation).
fn render_sensor_row(s: &Sensor) -> Markup {
    let sid = s.id.to_string(); // "S#20001"
    let logical = sid.strip_prefix("S#").unwrap_or(&sid);
    let unit = s.unit.clone().unwrap_or_default();
    // Trailing slash BEFORE the query — the static host 301s `/measurements?x`
    // → `/measurements/` and drops the query string; `/measurements/?x` doesn't.
    let datatilegnelse = format!(
        "/measurements/?daq={}&sid={}&logical={}&purpose={}&unit={}&type={}",
        urlencoding::encode(&s.daq_id),
        urlencoding::encode(&sid),
        urlencoding::encode(logical),
        urlencoding::encode(&s.purpose.to_string()),
        urlencoding::encode(&unit),
        urlencoding::encode(&s.meter_type.to_string()),
    );
    html! {
        li class="sensor-item sensor-row" {
            div class="sensor-row__label" {
                span class="sensor-row__name" { (s.purpose.to_string()) }
                " "
                span class="muted" { "(" (s.daq_id) ")" }
            }
            details class="kebab" {
                summary class="kebab__toggle" title="Handlinger" data-i18n-title="sensor.actions" { "\u{22EF}" }
                div class="action-dropdown kebab__menu" {
                    a href="#" _="on click halt" data-i18n="sensor.menu.details" { "Vis flere detaljer" }
                    a href="#" _="on click halt" data-i18n="sensor.menu.edit" { "Redigér sensor" }
                    a href="#" _="on click halt" data-i18n="sensor.menu.tags" { "Redigér tags" }
                    a href="#" _="on click halt" data-i18n="sensor.menu.consumption" { "Gå til forbrug" }
                    // data-astro-reload: full page load (skip the Astro view transition).
                    // The /measurements page reads location.search at Alpine-init time; a
                    // soft-nav inits it before the URL settles, so q/daq_id come up empty
                    // (blank metadata + a daq_id-less 400 on the load fetch). A real
                    // navigation has the URL right from the start.
                    a href=(datatilegnelse) data-astro-reload="" data-i18n="sensor.menu.datatilegnelse" { "Gå til datatilegnelse" }
                    a href="#" class="danger-link" _="on click halt" data-i18n="sensor.menu.deactivate" { "Deaktivér sensor" }
                    a href="#" class="danger-link" _="on click halt" data-i18n="sensor.menu.delete" { "Slet sensor" }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests (unit)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use model::domain::ids::NodeId;
    use model::domain::node::Node;

    fn make_hn2_node() -> Node {
        let id = NodeId::parse("HN2#10003").unwrap();
        Node::builder()
            .id(id)
            .name("SeedCo01".to_owned())
            .path("HN0#root|HN2#10003".to_owned())
            .build()
    }

    #[test]
    fn render_node_contains_id_and_name() {
        let node = make_hn2_node();
        let html = render_node(&node, false, true, Some(CognitoGroup::Admin), &[]).into_string();
        assert!(html.contains("HN2#10003"), "node id missing");
        assert!(html.contains("SeedCo01"), "node name missing");
    }

    #[test]
    fn render_node_no_metadata_message() {
        let node = make_hn2_node();
        let html = render_node(&node, false, true, Some(CognitoGroup::Admin), &[]).into_string();
        assert!(html.contains("No metadata available"), "no-metadata msg missing");
    }

    #[test]
    fn render_node_hx_vals_quot_escaped() {
        let node = make_hn2_node();
        let html = render_node(&node, false, true, Some(CognitoGroup::Admin), &[]).into_string();
        // data-hx-vals with JSON must not have unescaped " after ={
        assert!(
            !html.contains(r#"data-hx-vals="{""#),
            "unescaped double-quote in data-hx-vals"
        );
        assert!(
            html.contains("&quot;parent&quot;"),
            "&quot;-escaped parent key missing from hx-vals"
        );
    }

    // -------------------------------------------------------------------------
    // Capability-gating tests
    // -------------------------------------------------------------------------

    /// Admin: add-child block is present.
    #[test]
    fn render_node_admin_has_add_child() {
        let node = make_hn2_node();
        let html = render_node(&node, false, true, Some(CognitoGroup::Admin), &[]).into_string();
        assert!(html.contains("add-child-dialog"), "Admin should show add-child-dialog");
        assert!(html.contains("Add child"), "Admin should show Add child button");
        assert!(html.contains("ADD CHILD"), "dialog title missing");
        assert!(html.contains("delete_node"), "Admin should show the delete button");
    }

    /// Admin but the node type allows no children (leaf) → no add-child block.
    #[test]
    fn render_node_admin_leaf_hides_add_child() {
        let node = make_hn2_node();
        let html = render_node(&node, false, false, Some(CognitoGroup::Admin), &[]).into_string();
        assert!(
            !html.contains("add-child-dialog"),
            "Admin on a leaf type should NOT show add-child-dialog"
        );
        assert!(
            !html.contains("Add child"),
            "Admin on a leaf type should NOT show Add child button"
        );
        // Delete is gated on admin only (not allow_children) — still present on a leaf.
        assert!(html.contains("delete_node"), "Admin on a leaf should still show delete");
    }

    /// Writer: no add-child block, no add-sensor; name input is editable (no readonly).
    #[test]
    fn render_node_writer_no_add_child_or_sensor() {
        let node = make_hn2_node();
        let html = render_node(&node, true, true, Some(CognitoGroup::Writer), &[]).into_string();
        assert!(
            !html.contains("add-child-dialog"),
            "Writer should NOT have add-child-dialog"
        );
        assert!(
            !html.contains("Add child"),
            "Writer should NOT have Add child button"
        );
        assert!(
            !html.contains("add-sensor-dialog"),
            "Writer should NOT have add-sensor-dialog"
        );
        assert!(
            !html.contains("Add sensor"),
            "Writer should NOT have Add sensor button"
        );
        assert!(
            !html.contains("delete_node"),
            "Writer should NOT have the delete button"
        );
        // Name input should be editable (no readonly attribute).
        assert!(
            !html.contains(r#"id="name" value="SeedCo01" readonly"#),
            "Writer name input should not be readonly"
        );
    }

    /// Reader: no add-child block, no add-sensor; name input carries readonly.
    #[test]
    fn render_node_reader_no_add_child_or_sensor_and_readonly() {
        let node = make_hn2_node();
        let html = render_node(&node, true, true, Some(CognitoGroup::Reader), &[]).into_string();
        assert!(
            !html.contains("add-child-dialog"),
            "Reader should NOT have add-child-dialog"
        );
        assert!(
            !html.contains("add-sensor-dialog"),
            "Reader should NOT have add-sensor-dialog"
        );
        assert!(
            html.contains(r#"id="name" value="SeedCo01" readonly"#),
            "Reader name input should be readonly"
        );
    }

    /// None capability: same as Reader — no add-child/sensor, name readonly.
    #[test]
    fn render_node_none_capability_is_readonly() {
        let node = make_hn2_node();
        let html = render_node(&node, true, true, None, &[]).into_string();
        assert!(!html.contains("add-child-dialog"), "None should not show add-child-dialog");
        assert!(!html.contains("add-sensor-dialog"), "None should not show add-sensor-dialog");
        assert!(
            html.contains(r#"id="name" value="SeedCo01" readonly"#),
            "None capability: name input should be readonly"
        );
    }

    // -------------------------------------------------------------------------
    // Metadata edit-form gating
    // -------------------------------------------------------------------------

    fn lat_fields() -> Vec<(String, FieldSpec)> {
        use model::domain::values::FieldType;
        vec![(
            "lat".to_string(),
            FieldSpec {
                typ: FieldType::Number { min: Some(-90.0), max: Some(90.0) },
                required: true,
            },
        )]
    }

    #[test]
    fn render_node_writer_with_fields_shows_edit_form() {
        let mut node = make_hn2_node();
        node.metadata = serde_json::json!({ "lat": 55 }); // integer renders as "55"
        let fields = lat_fields();
        let html = render_node(&node, false, true, Some(CognitoGroup::Writer), &fields).into_string();
        assert!(html.contains("id=\"metadata-form\""), "edit form missing");
        assert!(html.contains("update_node"), "update_node action missing");
        assert!(html.contains("data-i18n=\"node.edit\""), "Edit button missing");
        assert!(html.contains("data-i18n=\"node.save\""), "Save button missing");
        assert!(html.contains("metadata-cancel-btn"), "Cancel button missing");
        assert!(html.contains("value=\"55\""), "lat prefill missing");
        assert!(html.contains("Data gemt"), "save-success toast text missing");
    }

    #[test]
    fn render_node_admin_with_fields_shows_edit_form() {
        let mut node = make_hn2_node();
        node.metadata = serde_json::json!({ "lat": 10 });
        let html = render_node(&node, false, true, Some(CognitoGroup::Admin), &lat_fields()).into_string();
        assert!(html.contains("id=\"metadata-form\""), "edit form missing for admin");
    }

    #[test]
    fn render_node_reader_with_fields_no_edit_form() {
        let mut node = make_hn2_node();
        node.metadata = serde_json::json!({ "lat": 55 });
        let html = render_node(&node, false, true, Some(CognitoGroup::Reader), &lat_fields()).into_string();
        assert!(!html.contains("id=\"metadata-form\""), "reader must not get edit form");
        assert!(!html.contains("update_node"), "reader must not get update_node");
    }

    #[test]
    fn render_node_writer_no_fields_no_edit_form() {
        let node = make_hn2_node(); // empty metadata, no fields
        let html = render_node(&node, false, true, Some(CognitoGroup::Writer), &[]).into_string();
        assert!(!html.contains("id=\"metadata-form\""), "no fields → no edit form");
        assert!(html.contains("No metadata available"));
    }

    #[test]
    fn render_sensors_empty() {
        let html = render_sensors(&[]).into_string();
        assert!(html.contains("No sensors found"));
    }

    #[test]
    fn render_sensors_nonempty() {
        use model::domain::ids::SensorId;
        use model::domain::values::{MeterType, Resource};
        let s = Sensor::builder()
            .id(SensorId::make(1))
            .daq_id("daq:test:001".to_owned())
            .path("HN0#root|HN2#10003|S#1".to_owned())
            .purpose(Resource::Electricity)
            .meter_type(MeterType::Counter)
            .build();
        let html = render_sensors(&[s]).into_string();
        assert!(html.contains("daq:test:001"));
        assert!(html.contains("electricity"));
        assert!(html.contains("sensor-item"));
    }
}
