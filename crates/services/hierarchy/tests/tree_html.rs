use model::domain::ids::{Level, NodeId};
use hierarchy::html::tree::{list_item, render_nodes};

// ---------------------------------------------------------------------------
// Whitespace normalizer
// ---------------------------------------------------------------------------

/// Normalise HTML for structural comparison:
///
/// 1. Collapse all whitespace runs between `>` and `<` to nothing
///    (removes indentation / newlines between tags).
/// 2. Normalise self-closing void syntax: ` />` → `>` (XHTML → HTML),
///    so `<path ... />` and `<path ...>` compare equal.
/// 3. Normalise attribute quoting: `attr='val'` → `attr="val"`.
/// 4. Decode `&quot;` → `"` so that maud-escaped and single-quoted attrs
///    compare equal (both end up with literal `"` inside double-quoted attrs).
/// 5. Trim the whole string.
pub fn normalize(html: &str) -> String {
    // Step 1: collapse whitespace between tags.
    let mut out = String::with_capacity(html.len());
    let bytes = html.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'>' {
            out.push('>');
            i += 1;
            // Skip whitespace between > and <
            while i < bytes.len()
                && (bytes[i] == b' '
                    || bytes[i] == b'\n'
                    || bytes[i] == b'\r'
                    || bytes[i] == b'\t')
            {
                i += 1;
            }
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }

    // Step 2: normalise self-closing void syntax ` />` → `>`.
    let out = out.replace(" />", ">").replace("/>", ">");

    // Step 3: replace single-quoted attribute values with double-quoted.
    // `='...'` → `="..."` (pass inner content through unchanged).
    let mut result = String::with_capacity(out.len());
    let chars: Vec<char> = out.chars().collect();
    let n = chars.len();
    let mut j = 0;
    while j < n {
        if j + 1 < n && chars[j] == '=' && chars[j + 1] == '\'' {
            result.push('=');
            result.push('"');
            j += 2; // skip ='
            while j < n && chars[j] != '\'' {
                result.push(chars[j]);
                j += 1;
            }
            if j < n {
                result.push('"'); // closing "
                j += 1; // skip closing '
            }
        } else {
            result.push(chars[j]);
            j += 1;
        }
    }

    // Step 4: decode &quot; → " (normalises maud-escaped vs pre-escaped attrs).
    let result = result.replace("&quot;", "\"");

    result.trim().to_owned()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn golden(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read golden file {:?}: {}", path, e))
}

// ---------------------------------------------------------------------------
// Golden comparison tests
// ---------------------------------------------------------------------------

/// Partner node (HN1#10001, "Acme Group", user "steen666@gmail.com",
/// parent "H#root") — must match tree_toplevel.html.
#[test]
fn tree_toplevel_matches_golden() {
    let id = NodeId::parse("HN1#10001").unwrap();
    let markup = list_item(&id, "Acme Group", "steen666@gmail.com", "H#root", false, false);
    let rendered = markup.into_string();
    let expected = golden("tree_toplevel.html");

    let got = normalize(&rendered);
    let want = normalize(&expected);

    if got != want {
        eprintln!("=== RENDERED (normalised) ===\n{}\n", got);
        eprintln!("=== EXPECTED (normalised) ===\n{}\n", want);
        // Show first diff position
        let g: Vec<char> = got.chars().collect();
        let w: Vec<char> = want.chars().collect();
        for (pos, (a, b)) in g.iter().zip(w.iter()).enumerate() {
            if a != b {
                eprintln!("First diff at char {}: got {:?}, want {:?}", pos, a, b);
                eprintln!("Context got:  {:?}", &got[pos.saturating_sub(20)..std::cmp::min(pos+40, got.len())]);
                eprintln!("Context want: {:?}", &want[pos.saturating_sub(20)..std::cmp::min(pos+40, want.len())]);
                break;
            }
        }
        if g.len() != w.len() {
            eprintln!("Length diff: got {} chars, want {} chars", g.len(), w.len());
        }
    }

    assert_eq!(got, want, "tree_toplevel.html golden mismatch");
}

/// Permission row for the same node — must match tree_permissions.html.
#[test]
fn tree_permissions_matches_golden() {
    let id = NodeId::parse("HN1#10001").unwrap();
    let markup = list_item(&id, "Acme Group", "steen666@gmail.com", "H#root", true, false);
    let rendered = markup.into_string();
    let expected = golden("tree_permissions.html");

    let got = normalize(&rendered);
    let want = normalize(&expected);

    if got != want {
        eprintln!("=== RENDERED (normalised) ===\n{}\n", got);
        eprintln!("=== EXPECTED (normalised) ===\n{}\n", want);
        let g: Vec<char> = got.chars().collect();
        let w: Vec<char> = want.chars().collect();
        for (pos, (a, b)) in g.iter().zip(w.iter()).enumerate() {
            if a != b {
                eprintln!("First diff at char {}: got {:?}, want {:?}", pos, a, b);
                eprintln!("Context got:  {:?}", &got[pos.saturating_sub(20)..std::cmp::min(pos+40, got.len())]);
                eprintln!("Context want: {:?}", &want[pos.saturating_sub(20)..std::cmp::min(pos+40, want.len())]);
                break;
            }
        }
        if g.len() != w.len() {
            eprintln!("Length diff: got {} chars, want {} chars", g.len(), w.len());
        }
    }

    assert_eq!(got, want, "tree_permissions.html golden mismatch");
}

// ---------------------------------------------------------------------------
// Critical substring tests
// ---------------------------------------------------------------------------

#[test]
fn tree_toplevel_contains_hyperscript() {
    let id = NodeId::parse("HN1#10001").unwrap();
    let markup = list_item(&id, "Acme Group", "steen666@gmail.com", "H#root", false, false);
    let html = markup.into_string();
    assert!(
        html.contains("on click remove .selected from .node-name-link in body"),
        "hyperscript missing"
    );
    assert!(
        html.contains("sessionStorage.selectedNodeId"),
        "sessionStorage.selectedNodeId missing"
    );
    assert!(
        html.contains("sessionStorage.selectedNodePath"),
        "sessionStorage.selectedNodePath missing"
    );
}

#[test]
fn tree_toplevel_has_loadchildren_trigger() {
    let id = NodeId::parse("HN1#10001").unwrap();
    let markup = list_item(&id, "Acme Group", "steen666@gmail.com", "H#root", false, false);
    let html = markup.into_string();
    assert!(html.contains("loadChildren"), "data-hx-trigger=loadChildren missing");
}

#[test]
fn tree_toplevel_has_pct_encoding() {
    let id = NodeId::parse("HN1#10001").unwrap();
    let markup = list_item(&id, "Acme Group", "steen666@gmail.com", "H#root", false, false);
    let html = markup.into_string();
    assert!(html.contains("%23"), "# not percent-encoded to %23");
    assert!(html.contains("%40"), "@ not percent-encoded to %40");
    assert!(html.contains("&amp;"), "& not HTML-escaped to &amp;");
}

#[test]
fn tree_toplevel_has_partner_classes() {
    let id = NodeId::parse("HN1#10001").unwrap();
    let markup = list_item(&id, "Acme Group", "steen666@gmail.com", "H#root", false, false);
    let html = markup.into_string();
    assert!(html.contains("tree-toggle-partner"), "tree-toggle-partner class missing");
    assert!(html.contains("#icon-partner"), "#icon-partner href missing");
}

#[test]
fn permissions_has_checkbox_names() {
    let id = NodeId::parse("HN1#10001").unwrap();
    let markup = list_item(&id, "Acme Group", "steen666@gmail.com", "H#root", true, false);
    let html = markup.into_string();
    assert!(html.contains("name=\"data.allowed\""), "data.allowed checkbox missing");
    assert!(html.contains("name=\"data.blocked\""), "data.blocked checkbox missing");
}

#[test]
fn permissions_has_loadchildren_once() {
    let id = NodeId::parse("HN1#10001").unwrap();
    let markup = list_item(&id, "Acme Group", "steen666@gmail.com", "H#root", true, false);
    let html = markup.into_string();
    assert!(html.contains("loadChildren once"), "loadChildren once trigger missing");
}

// ---------------------------------------------------------------------------
// render_nodes smoke test
// ---------------------------------------------------------------------------

#[test]
fn render_nodes_emits_two_items() {
    let refs = vec![
        (NodeId::parse("HN1#10001").unwrap(), "Acme Group".to_owned()),
        (NodeId::parse("HN1#10002").unwrap(), "Beta Corp".to_owned()),
    ];
    let markup = render_nodes(&refs, "steen666@gmail.com", "H#root", false);
    let html = markup.into_string();
    assert!(html.contains("HN1#10001"), "first node missing");
    assert!(html.contains("Acme Group"), "first name missing");
    assert!(html.contains("HN1#10002"), "second node missing");
    assert!(html.contains("Beta Corp"), "second name missing");
}

// ---------------------------------------------------------------------------
// Level-to-icon mapping test
// ---------------------------------------------------------------------------

#[test]
fn level_visual_mapping() {
    use hierarchy::html::level_visual;
    assert_eq!(level_visual(Level::Hn1), ("#icon-partner", "partner"));
    assert_eq!(level_visual(Level::Hn2), ("#icon-company", "company"));
    assert_eq!(level_visual(Level::Hn3), ("#icon-property", "property"));
    assert_eq!(level_visual(Level::Hn4), ("#icon-building", "building"));
    assert_eq!(level_visual(Level::Hn5), ("#icon-area", "area"));
    assert_eq!(level_visual(Level::Hn6), ("#icon-group", "group"));
    assert_eq!(level_visual(Level::Hn7), ("#icon-area", "area"));
    assert_eq!(level_visual(Level::Hn8), ("#icon-area", "area"));
    assert_eq!(level_visual(Level::Hn9), ("#icon-area", "area"));
    assert_eq!(level_visual(Level::Hn0), ("", ""));
}

// ---------------------------------------------------------------------------
// pct helper test
// ---------------------------------------------------------------------------

#[test]
fn pct_encodes_special_chars() {
    use hierarchy::html::pct;
    assert_eq!(pct("HN1#10001"), "HN1%2310001");
    assert_eq!(pct("steen666@gmail.com"), "steen666%40gmail.com");
    assert_eq!(pct("H#root#HN1#10001"), "H%23root%23HN1%2310001");
    // Unreserved chars pass through
    assert_eq!(pct("hello-world.test~ok_123"), "hello-world.test~ok_123");
}
