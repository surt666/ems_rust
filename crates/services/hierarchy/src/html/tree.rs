use maud::{html, Markup, PreEscaped};
use model::domain::ids::{Level, NodeId};

use super::{level_visual, pct};

/// Build the URL query string for the nodes endpoint.
///
/// Params are percent-encoded, and `&` between them is left as a plain `&`
/// in the string — maud will HTML-escape it to `&amp;` when it appears in an
/// attribute value (matching OCaml `api_html.ml` where `string_attr` only
/// HTML-escapes, and the browser DOM parser decodes `&amp;` → `&` when reading
/// the attribute).
fn nodes_url(id: &str, user: &str, path: &str) -> String {
    format!(
        "/hierarchy/query/nodes?id={}&user={}&path={}",
        pct(id),
        pct(user),
        pct(path)
    )
}

fn node_url(id: &str, user: &str) -> String {
    format!("/hierarchy/query/node?id={}&user={}", pct(id), pct(user))
}

/// The toggle `<svg>` arrow (or a spacer span when leaf).
fn tree_toggle(is_leaf: bool, bar_class: &str) -> Markup {
    if is_leaf {
        html! { span class="tree-toggle-spacer" {} }
    } else {
        let cls = format!("tree-toggle tree-toggle-{}", bar_class);
        html! {
            svg class=(cls) width="20" height="20" viewBox="0 0 16 16" fill="currentColor" {
                path d="M3 1l12 7-12 7z";
            }
        }
    }
}

/// The `<svg>` icon (or nothing for Hn0 which has no icon).
fn tree_icon(icon_href: &str) -> Markup {
    if icon_href.is_empty() {
        html! {}
    } else {
        html! {
            svg aria-hidden="true" focusable="false" class="tree-icon" width="16" height="16" {
                use href=(icon_href);
            }
        }
    }
}

/// Emit one tree `<li>` or one permission-row + child-rows `<div>` pair,
/// matching `api_html.ml :: list_item`.
///
/// * `id`              – the node id (`HN1#10001`, etc.)
/// * `name`            – display name
/// * `user`            – email address of the requesting user (not `U#` prefixed)
/// * `parent_path`     – parent path string (e.g. `"H#root"`) or empty string
///   for top-level nodes where the path is computed from scratch
/// * `with_permissions` – emit the permission-grid variant
/// * `is_leaf`         – suppress the expand toggle
pub fn list_item(
    id: &NodeId,
    name: &str,
    user: &str,
    parent_path: &str,
    with_permissions: bool,
    is_leaf: bool,
) -> Markup {
    let id_str = id.to_string();
    let level = id.level();

    // Build current_path: "H#root#HN1#10001" when parent is "H#root".
    // Mirrors OCaml: parent_path = Some p → sprintf "%s#%s" p id_str
    //                            = None   → sprintf "H#%s" id_str
    // In our API parent_path is always provided (non-empty = has parent,
    // empty = no parent → "H#<id_str>"). The caller passes "H#root" for
    // top-level nodes.
    let current_path = if parent_path.is_empty() {
        format!("H#{}", id_str)
    } else {
        format!("{}#{}", parent_path, id_str)
    };

    let display_name = if name == "root" { "" } else { name };
    let (icon_href, bar_class) = level_visual(level);
    let toggle = tree_toggle(is_leaf, bar_class);
    let icon = tree_icon(icon_href);

    if with_permissions {
        // Permission-grid variant:
        //   <div class="permission-row" ...> … </div>
        //   <div class="child-rows" ...></div>
        let perm_url = format!("{}&permissions=true", nodes_url(&id_str, user, &current_path));
        html! {
            div class="permission-row"
                data-id=(id_str)
                data-path=(current_path)
                style="display: grid; grid-template-columns: auto auto auto auto auto auto 1fr; gap: 0.5rem; align-items: center;"
            {
                (toggle)
                (icon)
                input type="checkbox" name="data.allowed" value=(id_str) class="allowed-checkbox";
                svg width="16" height="16" style="color: var(--success);" fill="none" stroke="currentColor" viewBox="0 0 24 24" {
                    path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 13l4 4L19 7";
                }
                input type="checkbox" name="data.blocked" value=(id_str) class="blocked-checkbox";
                svg width="16" height="16" style="color: var(--danger);" fill="none" stroke="currentColor" viewBox="0 0 24 24" {
                    path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12";
                }
                span style="font-weight: 600;" { (display_name) }
            }
            div class="child-rows"
                style="display:none;"
                data-hx-get=(perm_url)
                data-hx-request=(PreEscaped(r#"{"noHeaders": true}"#))
                data-hx-target="this"
                data-hx-swap="innerHTML"
                data-hx-trigger="loadChildren once"
            {}
        }
    } else {
        // Standard tree <li> variant.
        let load_url = nodes_url(&id_str, user, &current_path);
        let detail_url = node_url(&id_str, user);
        // node_path for <a data-node-path> is parent_path (not current_path)
        // — mirrors OCaml: `let node_path = Option.value parent_path ~default:""`
        // When parent_path is Some p, it's p; when None (top-level), it's "".
        let node_path = parent_path;
        let hyperscript = "on click remove .selected from .node-name-link in body \
            then add .selected to me \
            then set sessionStorage.selectedNodeId to my @data-node-id \
            then set sessionStorage.selectedNodePath to my @data-node-path";
        html! {
            li data-id=(id_str) data-path=(current_path) {
                div class="icon-wrapper"
                    data-hx-get=(load_url)
                    data-hx-request=(PreEscaped(r#"{"noHeaders": true}"#))
                    data-hx-target="next .nested-list"
                    data-hx-trigger="loadChildren"
                {
                    (toggle)
                }
                (icon)
                a href="#"
                    class="node-name-link"
                    data-node-id=(id_str)
                    data-node-path=(node_path)
                    data-hx-get=(detail_url)
                    data-hx-request=(PreEscaped(r#"{"noHeaders": true}"#))
                    data-hx-target=".main-area"
                    data-hx-swap="innerHTML"
                    _=(hyperscript)
                    style="cursor: pointer; text-decoration: none; color: inherit;"
                {
                    (display_name)
                }
                ul class="nested-list" {}
            }
        }
    }
}

/// Render a sequence of node refs as a series of `list_item`s (no wrapper
/// element — matches OCaml `render_nodes` which returns `null items`, i.e.
/// a bare sequence that HTMX swaps into the target).
pub fn render_nodes(
    refs: &[(NodeId, String)],
    user: &str,
    parent_path: &str,
    with_permissions: bool,
) -> Markup {
    html! {
        @for (id, name) in refs {
            // Leaf detection: OCaml treats Hn4 as deepest displayable level.
            @let is_leaf = id.level() == Level::Hn4;
            (list_item(id, name, user, parent_path, with_permissions, is_leaf))
        }
    }
}
