use maud::{html, Markup};
use model::domain::schema::FieldSpec;
use model::domain::values::{Currency, FieldType, Language, Permission, Profile, Timezone};
use strum::IntoEnumIterator;

// ---------------------------------------------------------------------------
// Option-list renderers
// ---------------------------------------------------------------------------

/// `<option>` elements for all Profile variants (in `Profile::all` order).
pub fn render_profiles() -> Markup {
    html! {
        @for p in Profile::all() {
            @let s = p.to_string();
            option value=(s) { (s) }
        }
    }
}

/// `<option>` elements for all supported languages (lowercase).
pub fn render_languages() -> Markup {
    html! {
        @for l in Language::iter() {
            @let s = l.to_string();
            option value=(s) { (s) }
        }
    }
}

/// `<option>` elements for all supported currencies.
pub fn render_currencies() -> Markup {
    html! {
        @for c in Currency::iter() {
            @let s = c.to_string();
            option value=(s) { (s) }
        }
    }
}

/// `<option>` elements for all permission levels.
pub fn render_permissions() -> Markup {
    html! {
        @for p in Permission::iter() {
            @let s = p.to_string();
            option value=(s) { (s) }
        }
    }
}

/// `<option>` elements for all supported timezones.
pub fn render_timezones() -> Markup {
    html! {
        @for tz in Timezone::iter() {
            @let s = tz.to_string();
            option value=(s) { (s) }
        }
    }
}

// ---------------------------------------------------------------------------
// Add-child form (the dialog body)
// ---------------------------------------------------------------------------

/// Render the add-child form body.
///
/// This is the fragment loaded into `#add-child-body` via HTMX.
///
/// Parameters:
/// - `parent_id_str`: the parent node id (e.g. `"HN2#10003"`)
/// - `chosen_level`: display string for the level (e.g. `"HN3"`)
/// - `level_options`: pre-rendered `<option>` elements for the level selector
/// - `label_options`: pre-rendered `<option>` elements for the label selector
///   (empty slice → no label row; single item → disabled selector + hidden input)
/// - `metadata_inputs`: pre-rendered form rows for metadata fields
/// - `is_level_choice`: when false the level selector is disabled (only one allowed)
/// - `needs_schema`: when true (creating a company), render the "Design skema"
///   button + hidden `schema_json` field that the schema designer populates
pub fn render_add_child_form(
    parent_id_str: &str,
    chosen_level: &str,
    level_options: &[(&str, bool)],  // (value, is_selected)
    label_options: &[(&str, bool)],  // (value, is_selected)
    metadata_inputs: Markup,
    is_multi_level: bool,
    needs_schema: bool,
) -> Markup {
    let after_request_js = "if(event.detail.elt.id === \
        'add-child-form' && \
        event.detail.successful) { \
        document.querySelector('#add-child-dialog').close(); \
        location.reload(); } else if \
        (event.detail.elt.id === \
        'add-child-form') { \
        document.getElementById('add-child-error').textContent \
        = event.detail.xhr.responseText; \
        document.getElementById('add-child-error').style.display \
        = 'block'; }";

    let level_vals = format!(r#"{{"parent": "{}"}}"#, parent_id_str);

    html! {
        div id="add-child-error" class="login-error"
            style="display:none; margin-bottom: 1rem;"
        {}
        form id="add-child-form" class="form"
            data-hx-post="/hierarchy/command"
            data-hx-swap="none"
            hx-on--after-request=(after_request_js)
        {
            input type="hidden" name="action" value="add_node";
            input type="hidden" name="data.parent_id" value=(parent_id_str);
            // Level selector
            div class="form-row" {
                label class="form-label" data-i18n="common.type" { "Type" }
                @if is_multi_level {
                    select name="data.level" class="form-select"
                        data-hx-get="/hierarchy/query/add_child_form"
                        data-hx-trigger="change"
                        data-hx-vals=(level_vals)
                        data-hx-include="this"
                        data-hx-target="#add-child-body"
                        data-hx-swap="innerHTML"
                    {
                        @for (val, selected) in level_options {
                            option value=(val) selected[*selected] { (val) }
                        }
                    }
                } @else {
                    select class="form-select" disabled {
                        @for (val, selected) in level_options {
                            option value=(val) selected[*selected] { (val) }
                        }
                    }
                    input type="hidden" name="data.level" value=(chosen_level);
                }
            }
            // Label selector (only emitted if labels are provided)
            @if !label_options.is_empty() {
                @let single = label_options.len() == 1;
                div class="form-row" {
                    label class="form-label" data-i18n="common.label" { "Label" }
                    @if single {
                        select class="form-select" disabled {
                            @for (val, _) in label_options {
                                option value=(val) selected { (val) }
                            }
                        }
                        @for (val, _) in label_options {
                            input type="hidden" name="data.label" value=(val);
                        }
                    } @else {
                        select name="data.label" class="form-select" {
                            @for (val, selected) in label_options {
                                option value=(val) selected[*selected] { (val) }
                            }
                        }
                    }
                }
            }
            // Name field (always present)
            div class="form-row" {
                label class="form-label" data-i18n="common.name" { "Name" }
                input type="text" name="data.name" required class="form-input";
                span class="required" { "*" }
            }
            // Schema designer (company only): button opens the shared designer
            // dialog; its serialised schema is written into this hidden field by
            // the Layout's schema-updated listener.
            @if needs_schema {
                div class="form-row" {
                    label class="form-label" data-i18n="node.schema" { "Hierarki-skema" }
                    button type="button" class="btn-secondary"
                        // Opens the designer in SCHEMA mode. The same dialog also
                        // edits node formulas; going through this entry point keeps
                        // the two from appearing together.
                        onclick="window.emsOpenSchemaDesigner && window.emsOpenSchemaDesigner()"
                    {
                        "Design skema" span class="required" { "*" }
                    }
                    span id="add-child-schema-status" style="margin-left:.5rem;color:var(--text-muted);" { "(ikke defineret)" }
                }
                input type="hidden" name="schema_json" id="add-child-schema-json";
            }
            // Schema-driven metadata fields
            (metadata_inputs)
        }
    }
}

// ---------------------------------------------------------------------------
// metadata_inputs — schema-typed metadata form inputs (shared)
// ---------------------------------------------------------------------------

/// Render schema-typed metadata form inputs.
///
/// - `prefill`: current values keyed by field name (for the edit form); `None`
///   leaves inputs empty (add-child form).
/// - `disabled`: when true, inputs/selects start disabled and carry the
///   `md-input` class (used by the node-edit form's Edit toggle); add-child
///   passes `false` for byte-identical output to the previous builder.
pub fn metadata_inputs(
    fields: &[(String, FieldSpec)],
    prefill: Option<&serde_json::Map<String, serde_json::Value>>,
    disabled: bool,
) -> Markup {
    let cur = |name: &str| -> Option<String> {
        prefill.and_then(|m| m.get(name)).map(crate::html::scalar_to_string)
    };
    // For <input type=date>, prefill wants YYYY-MM-DD.
    let date_part = |s: &str| s.get(0..10).unwrap_or(s).to_string();

    let input_class = if disabled { "form-input md-input" } else { "form-input" };
    let select_class = if disabled { "form-select md-input" } else { "form-select" };

    html! {
        @for (fname, spec) in fields {
            @let req_attr = spec.required;
            @let value = cur(fname);
            div class="form-row" {
                label class="form-label" { (fname) }
                @match &spec.typ {
                    FieldType::String { .. } => {
                        input type="text"
                            name=(format!("data.metadata.{}", fname))
                            class=(input_class)
                            value=[value.clone()]
                            disabled[disabled]
                            required[req_attr];
                    }
                    FieldType::Number { .. } => {
                        input type="number" step="any"
                            name=(format!("data.metadata.{}", fname))
                            class=(input_class)
                            value=[value.clone()]
                            disabled[disabled]
                            required[req_attr];
                    }
                    FieldType::Integer { .. } => {
                        input type="number" step="1"
                            name=(format!("data.metadata.{}", fname))
                            class=(input_class)
                            value=[value.clone()]
                            disabled[disabled]
                            required[req_attr];
                    }
                    FieldType::Boolean => {
                        @let v = value.clone().unwrap_or_default();
                        select
                            name=(format!("data.metadata.{}", fname))
                            class=(select_class)
                            disabled[disabled]
                            required[req_attr]
                        {
                            option value="true" selected[v == "true"] { "true" }
                            option value="false" selected[v == "false"] { "false" }
                        }
                    }
                    FieldType::Timestamp => {
                        input type="date"
                            name=(format!("data.metadata.{}", fname))
                            class=(input_class)
                            value=[value.clone().map(|s| date_part(&s))]
                            disabled[disabled]
                            required[req_attr];
                    }
                    FieldType::Enum { one_of } => {
                        @let v = value.clone().unwrap_or_default();
                        select
                            name=(format!("data.metadata.{}", fname))
                            class=(select_class)
                            disabled[disabled]
                            required[req_attr]
                        {
                            @for opt in one_of {
                                option value=(opt) selected[*opt == v] { (opt) }
                            }
                        }
                    }
                }
                @if spec.required {
                    span class="required" { "*" }
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

    #[test]
    fn render_profiles_all_variants() {
        let html = render_profiles().into_string();
        assert!(html.contains("Developer"), "Developer missing");
        assert!(html.contains("Standard"), "Standard missing");
        assert!(html.contains("Technician"), "Technician missing");
        assert!(html.contains("Reader"), "Reader missing");
        assert!(html.contains("SysAdm"), "SysAdm missing");
    }

    #[test]
    fn render_languages_all_variants() {
        let html = render_languages().into_string();
        for lang in ["danish", "swedish", "norwegian", "english", "german"] {
            assert!(html.contains(lang), "{lang} missing");
        }
    }

    #[test]
    fn render_currencies_all_variants() {
        let html = render_currencies().into_string();
        for c in ["DKK", "SEK", "NOK", "USD", "EUR"] {
            assert!(html.contains(c), "{c} missing");
        }
    }

    #[test]
    fn render_permissions_all_variants() {
        let html = render_permissions().into_string();
        for p in ["view", "edit", "admin"] {
            assert!(html.contains(p), "{p} missing");
        }
    }

    #[test]
    fn render_timezones_all_variants() {
        let html = render_timezones().into_string();
        for tz in [
            "Europe/Copenhagen",
            "Europe/Stockholm",
            "Europe/Oslo",
            "Europe/Berlin",
            "Europe/London",
            "Europe/Paris",
            "Europe/Madrid",
            "Europe/Rome",
            "Europe/Amsterdam",
            "UTC",
        ] {
            assert!(html.contains(tz), "{tz} missing");
        }
    }

    #[test]
    fn render_add_child_form_key_fields() {
        let markup = render_add_child_form(
            "HN2#10003",
            "HN3",
            &[("HN3", true)],
            &[("property", true)],
            html! {},
            false,
            false,
        );
        let html = markup.into_string();
        assert!(html.contains(r#"name="action""#), "action input missing");
        assert!(html.contains(r#"value="add_node""#), "add_node value missing");
        assert!(html.contains(r#"name="data.parent_id""#), "parent_id input missing");
        assert!(html.contains(r#"name="data.name""#), "name input missing");
        assert!(html.contains(r#"name="data.label""#), "label input missing");
        // No schema designer for a non-company child.
        assert!(!html.contains(r#"name="schema_json""#), "schema field should be absent");
    }

    #[test]
    fn render_add_child_form_company_has_schema_designer() {
        let markup = render_add_child_form(
            "HN1#10001",
            "HN2",
            &[("HN2", true)],
            &[("company", true)],
            html! {},
            false,
            true, // needs_schema
        );
        let html = markup.into_string();
        assert!(html.contains(r#"name="schema_json""#), "hidden schema_json field missing");
        // Goes through emsOpenSchemaDesigner rather than showModal() directly: the
        // dialog also edits node formulas, and that entry point is what keeps the
        // hierarchy editor from appearing alongside them.
        assert!(html.contains("emsOpenSchemaDesigner"), "designer open hook missing");
        assert!(html.contains("Design skema"), "design-schema button missing");
    }
}
