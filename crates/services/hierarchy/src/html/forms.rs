use maud::{html, Markup};
use model::domain::values::Profile;

// ---------------------------------------------------------------------------
// Option-list renderers
// ---------------------------------------------------------------------------

/// `<option>` elements for all Profile variants (in OCaml `Profile.all` order).
/// Mirrors OCaml `api_html.ml :: render_profiles`.
pub fn render_profiles() -> Markup {
    html! {
        @for p in Profile::all() {
            @let s = p.to_string();
            option value=(s) { (s) }
        }
    }
}

/// `<option>` elements for all supported languages (lowercase).
/// Mirrors OCaml `api_html.ml :: render_languages`.
pub fn render_languages() -> Markup {
    let langs = ["danish", "swedish", "norwegian", "english", "german"];
    html! {
        @for l in langs {
            option value=(l) { (l) }
        }
    }
}

/// `<option>` elements for all supported currencies.
/// Mirrors OCaml `api_html.ml :: render_currencies`.
pub fn render_currencies() -> Markup {
    let currencies = ["DKK", "SEK", "NOK", "USD", "EUR"];
    html! {
        @for c in currencies {
            option value=(c) { (c) }
        }
    }
}

/// `<option>` elements for all permission levels.
/// Mirrors OCaml `api_html.ml :: render_permissions`.
pub fn render_permissions() -> Markup {
    let perms = ["view", "edit", "admin"];
    html! {
        @for p in perms {
            option value=(p) { (p) }
        }
    }
}

/// `<option>` elements for all supported timezones.
/// Mirrors OCaml `api_html.ml :: render_timezones`.
pub fn render_timezones() -> Markup {
    let tzs = [
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
    ];
    html! {
        @for tz in tzs {
            option value=(tz) { (tz) }
        }
    }
}

// ---------------------------------------------------------------------------
// Add-child form (the dialog body)
// ---------------------------------------------------------------------------

/// Render the add-child form body.
///
/// This is the fragment loaded into `#add-child-body` via HTMX.
/// Mirrors OCaml `api_html.ml :: render_add_child_form` (the inner HTML).
///
/// Parameters:
/// - `parent_id_str`: the parent node id (e.g. `"HN2#10003"`)
/// - `chosen_level`: display string for the level (e.g. `"HN3"`)
/// - `level_options`: pre-rendered `<option>` elements for the level selector
/// - `label_options`: pre-rendered `<option>` elements for the label selector
///   (empty slice → no label row; single item → disabled selector + hidden input)
/// - `metadata_inputs`: pre-rendered form rows for metadata fields
/// - `is_level_choice`: when false the level selector is disabled (only one allowed)
pub fn render_add_child_form(
    parent_id_str: &str,
    chosen_level: &str,
    level_options: &[(&str, bool)],  // (value, is_selected)
    label_options: &[(&str, bool)],  // (value, is_selected)
    metadata_inputs: Markup,
    is_multi_level: bool,
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
                            @if *selected {
                                option value=(val) selected { (val) }
                            } @else {
                                option value=(val) { (val) }
                            }
                        }
                    }
                } @else {
                    select class="form-select" disabled {
                        @for (val, selected) in level_options {
                            @if *selected {
                                option value=(val) selected { (val) }
                            } @else {
                                option value=(val) { (val) }
                            }
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
                                @if *selected {
                                    option value=(val) selected { (val) }
                                } @else {
                                    option value=(val) { (val) }
                                }
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
            // Schema-driven metadata fields
            (metadata_inputs)
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
        );
        let html = markup.into_string();
        assert!(html.contains(r#"name="action""#), "action input missing");
        assert!(html.contains(r#"value="add_node""#), "add_node value missing");
        assert!(html.contains(r#"name="data.parent_id""#), "parent_id input missing");
        assert!(html.contains(r#"name="data.name""#), "name input missing");
        assert!(html.contains(r#"name="data.label""#), "label input missing");
    }
}
