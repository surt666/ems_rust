use model::domain::ids::NodeId;
use model::domain::node::Node;
use model::domain::values::CognitoGroup;
use hierarchy::html::node::render_node;
use hierarchy::html::forms::{
    render_currencies, render_languages, render_permissions, render_profiles, render_timezones,
};

// ---------------------------------------------------------------------------
// Whitespace normalizer (re-used from tree_html.rs)
// ---------------------------------------------------------------------------

/// Normalise HTML for structural comparison:
///
/// 1. Collapse all whitespace runs between `>` and `<` to nothing.
/// 2. Normalise self-closing void syntax: ` />` → `>`.
/// 3. Normalise attribute quoting: `attr='val'` → `attr="val"`.
/// 4. Decode `&quot;` → `"` so maud-escaped and single-quoted attrs compare equal.
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
    let mut result = String::with_capacity(out.len());
    let chars: Vec<char> = out.chars().collect();
    let n = chars.len();
    let mut j = 0;
    while j < n {
        if j + 1 < n && chars[j] == '=' && chars[j + 1] == '\'' {
            result.push('=');
            result.push('"');
            j += 2;
            while j < n && chars[j] != '\'' {
                result.push(chars[j]);
                j += 1;
            }
            if j < n {
                result.push('"');
                j += 1;
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

/// Print a diff-friendly mismatch report and return (got, want).
fn assert_golden(rendered: &str, golden_html: &str, label: &str) {
    let got = normalize(rendered);
    let want = normalize(golden_html);

    if got != want {
        eprintln!("=== {} RENDERED (normalised) ===\n{}\n", label, got);
        eprintln!("=== {} EXPECTED (normalised) ===\n{}\n", label, want);
        let g: Vec<char> = got.chars().collect();
        let w: Vec<char> = want.chars().collect();
        for (pos, (a, b)) in g.iter().zip(w.iter()).enumerate() {
            if a != b {
                eprintln!(
                    "First diff at char {}: got {:?}, want {:?}",
                    pos, a, b
                );
                eprintln!(
                    "Context got:  {:?}",
                    &got[pos.saturating_sub(30)..std::cmp::min(pos + 60, got.len())]
                );
                eprintln!(
                    "Context want: {:?}",
                    &want[pos.saturating_sub(30)..std::cmp::min(pos + 60, want.len())]
                );
                break;
            }
        }
        if g.len() != w.len() {
            eprintln!(
                "Length diff: got {} chars, want {} chars",
                g.len(),
                w.len()
            );
        }
    }

    assert_eq!(got, want, "{} golden mismatch", label);
}

// ---------------------------------------------------------------------------
// Build the HN2#10003 node for the node-detail test
// ---------------------------------------------------------------------------

fn make_hn2_10003_node() -> Node {
    let id = NodeId::parse("HN2#10003").unwrap();
    Node {
        id,
        name: "SeedCo01".to_owned(),
        parent: None, // no parent → parent_str = nid_str (not used since no sensors)
        path: "HN0#root|HN2#10003".to_owned(),
        created: chrono::Utc::now(),
        metadata: serde_json::json!({}),
        schema: None,
        label: String::new(),
    }
}

// ---------------------------------------------------------------------------
// Golden comparison tests
// ---------------------------------------------------------------------------

#[test]
fn node_detail_matches_golden() {
    let node = make_hn2_10003_node();
    // Admin case: show_sensors=false; golden was captured from full-permissions output.
    let rendered = render_node(&node, false, true, Some(CognitoGroup::Admin), &[]).into_string();
    let expected = golden("node_detail.html");
    assert_golden(&rendered, &expected, "node_detail.html");
}

#[test]
fn profiles_matches_golden() {
    let rendered = render_profiles().into_string();
    let expected = golden("profiles.html");
    assert_golden(&rendered, &expected, "profiles.html");
}

#[test]
fn languages_matches_golden() {
    let rendered = render_languages().into_string();
    let expected = golden("languages.html");
    assert_golden(&rendered, &expected, "languages.html");
}

#[test]
fn currencies_matches_golden() {
    let rendered = render_currencies().into_string();
    let expected = golden("currencies.html");
    assert_golden(&rendered, &expected, "currencies.html");
}

#[test]
fn permissions_matches_golden() {
    let rendered = render_permissions().into_string();
    let expected = golden("permissions.html");
    assert_golden(&rendered, &expected, "permissions.html");
}

// ---------------------------------------------------------------------------
// Validity assertions — quote-bearing attributes must be &quot;-escaped
// ---------------------------------------------------------------------------

#[test]
fn node_detail_hx_vals_quot_escaped() {
    let node = make_hn2_10003_node();
    let html = render_node(&node, false, true, Some(CognitoGroup::Admin), &[]).into_string();
    // data-hx-vals must NOT have a raw `{"` after the `=`
    assert!(
        !html.contains(r#"data-hx-vals="{""#),
        "invalid HTML: unescaped double-quote in data-hx-vals"
    );
    // Must contain &quot;-escaped keys
    assert!(
        html.contains("&quot;parent&quot;"),
        "data-hx-vals JSON not &quot;-escaped"
    );
}

#[test]
fn node_detail_hx_request_quot_escaped() {
    let node = make_hn2_10003_node();
    let html = render_node(&node, false, true, Some(CognitoGroup::Admin), &[]).into_string();
    assert!(
        !html.contains(r#"data-hx-request="{""#),
        "invalid HTML: unescaped double-quote in data-hx-request"
    );
    assert!(
        html.contains("&quot;noHeaders&quot;"),
        "data-hx-request JSON not &quot;-escaped"
    );
}

// ---------------------------------------------------------------------------
// Dialog/form structural assertions
// ---------------------------------------------------------------------------

#[test]
fn add_child_form_action_and_fields() {
    let node = make_hn2_10003_node();
    let html = render_node(&node, false, true, Some(CognitoGroup::Admin), &[]).into_string();
    assert!(html.contains("add-child-dialog"), "add-child-dialog id missing");
    assert!(html.contains("add-child-body"), "add-child-body id missing");
    assert!(html.contains("/hierarchy/query/add_child_form"), "form URL missing");
    assert!(html.contains("ADD CHILD"), "dialog title missing");
    assert!(html.contains("Add child"), "Add child button label missing");
}

#[test]
fn timezones_structural() {
    let html = render_timezones().into_string();
    assert!(html.contains("Europe/Copenhagen"), "Europe/Copenhagen missing");
    assert!(html.contains("UTC"), "UTC missing");
    // Count option elements
    let count = html.matches("<option").count();
    assert_eq!(count, 10, "expected 10 timezone options, got {}", count);
}

#[test]
fn profiles_order_is_stable() {
    let html = render_profiles().into_string();
    // Expected order: Developer, Standard, Technician, Reader, SysAdm
    let dev_pos = html.find("Developer").unwrap();
    let std_pos = html.find("Standard").unwrap();
    let tech_pos = html.find("Technician").unwrap();
    let reader_pos = html.find("Reader").unwrap();
    let sysadm_pos = html.find("SysAdm").unwrap();
    assert!(dev_pos < std_pos, "Developer must come before Standard");
    assert!(std_pos < tech_pos, "Standard must come before Technician");
    assert!(tech_pos < reader_pos, "Technician must come before Reader");
    assert!(reader_pos < sysadm_pos, "Reader must come before SysAdm");
}

// ---------------------------------------------------------------------------
// Node formulas (the Formler section)
// ---------------------------------------------------------------------------

use hierarchy::html::node::render_node_formulas;
use model::domain::ids::{Level, SensorId};
use model::domain::node_formula::{NodeFormula, Reference, Term};
use model::domain::sensor::Sensor;
use model::domain::values::{EnergyType, Purpose, ReadingKind};

const CO: &str = "HN0#root|HN2#997";

fn fx_node(level: Level, id: u32, parent: Option<(Level, u32)>, path: &str, name: &str) -> Node {
    Node::builder()
        .id(NodeId::make(level, id))
        .name(name.to_string())
        .parent(parent.map(|(l, i)| NodeId::make(l, i)))
        .path(path.to_string())
        .build()
}

fn fx_sensor(id: u32, node_path: &str, daq: &str) -> Sensor {
    Sensor::builder()
        .id(SensorId::make(id))
        .daq_id(daq.to_string())
        .path(format!("{node_path}|S#{id}"))
        .energy_type(EnergyType::DistrictHeating)
        .reading_kind(ReadingKind::Counter)
        .build()
}

/// Building A1 with a bimåler formula, one own sensor, one sensor in a sibling
/// branch (Building C2), a direct child and a grandchild.
fn fixture() -> String {
    let a1 = format!("{CO}|HN4#1");
    let c2 = format!("{CO}|HN4#2");
    let node = fx_node(Level::Hn4, 1, Some((Level::Hn2, 997)), &a1, "Building A1");
    let formulas = vec![
        NodeFormula {
            node: NodeId::make(Level::Hn4, 1),
            energy_type: EnergyType::DistrictHeating,
            purpose: Purpose::Dhw,
            terms: vec![Term {
                reference: Reference::Sensor(SensorId::make(2)),
                coefficient: 1.0,
            }],
            note: None,
        },
        NodeFormula {
            node: NodeId::make(Level::Hn4, 1),
            energy_type: EnergyType::DistrictHeating,
            purpose: Purpose::SpaceHeating,
            terms: vec![
                Term { reference: Reference::Sensor(SensorId::make(1)), coefficient: 1.0 },
                Term { reference: Reference::Sensor(SensorId::make(2)), coefficient: -1.0 },
            ],
            note: Some("bimåler".to_string()),
        },
    ];
    let company_sensors = vec![
        fx_sensor(1, &a1, "daq:main"),
        fx_sensor(2, &a1, "daq:dhw"),
        fx_sensor(77, &c2, "daq:elsewhere"),
    ];
    let children = vec![fx_node(Level::Hn5, 5, Some((Level::Hn4, 1)), &format!("{a1}|HN5#5"), "Area")];
    render_node_formulas(&node, &formulas, &company_sensors, &children).into_string()
}

#[test]
fn node_formulas_render_one_card_per_formula() {
    let html = fixture();
    assert!(html.contains("district_heating"), "energy type in the heading");
    assert!(html.contains("space_heating"), "purpose in the heading");
    assert!(html.contains("bimåler"), "the note is shown");
    assert!(html.contains("-1"), "the subtraction coefficient is shown");
}

/// Both commands must be postable from the card.
#[test]
fn node_formulas_post_set_and_delete() {
    let html = fixture();
    assert!(html.contains("set_node_formula"), "save posts set_node_formula");
    assert!(html.contains("delete_node_formula"), "delete posts delete_node_formula");
    assert!(html.contains("/hierarchy/command"), "posts to the command endpoint");
}

/// The picker offers EVERY sensor in the company — that is what makes the
/// main-in-one-building / sub-in-another case reachable — and names the node
/// each one hangs off, so a sideways reference is an informed choice.
#[test]
fn reference_picker_offers_company_wide_sensors() {
    let html = fixture();
    assert!(html.contains("S#1") && html.contains("S#2"), "own sensors offered");
    assert!(html.contains("S#77"), "a sensor in another branch is offered too");
}

/// Node references are direct children only — an upward or deeper one would
/// double count.
#[test]
fn reference_picker_offers_only_direct_children_as_nodes() {
    let html = fixture();
    assert!(html.contains("HN5#5"), "the direct child is offered");
    assert!(!html.contains("HN2#997"), "the parent must not be offered");
}

/// `unallocated` is derived by the roll-up and must not be declarable.
#[test]
fn new_formula_card_does_not_offer_reserved_purposes() {
    let html = fixture();
    assert!(!html.contains("\"unallocated\""), "unallocated must not be selectable");
    assert!(html.contains("\"total\""), "but total is - it IS the node's own formula");
}

/// The old per-sensor formula dialog is gone, and the sensor form asks for
/// nothing but what the sensor measures.
#[test]
fn add_sensor_form_has_no_classification_controls() {
    let node = fx_node(Level::Hn4, 1, Some((Level::Hn2, 997)), &format!("{CO}|HN4#1"), "B");
    let html = render_node(&node, true, true, Some(CognitoGroup::Admin), &[]).into_string();
    assert!(!html.contains("formula-dialog"), "the dialog is gone");
    assert!(!html.contains("data.formula"), "and its hidden inputs");
    assert!(html.contains("data.energy_type"), "energy_type is what a sensor declares");
}
