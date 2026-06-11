//! Command ADT + form→JSON normaliser.

use serde::Deserialize;
use serde_json::{Value, Map};

// ---------------------------------------------------------------------------
// Command ADT
// ---------------------------------------------------------------------------

/// Every command accepted by `POST /hierarchy/command`.
///
/// Serde-tagged on `"action"` (snake_case).
#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Command {
    /// `add_node` — add a child node under `parent_id`.
    AddNode {
        parent_id: String,
        name: String,
        #[serde(default)]
        label: Option<String>,
        /// `"hn1"` / `"hn2"` / … string; parsed downstream.
        #[serde(default)]
        level: Option<String>,
        /// Arbitrary JSON metadata object.
        #[serde(default)]
        metadata: Option<Value>,
        /// Schema JSON blob (validated downstream).
        #[serde(default)]
        schema: Option<Value>,
    },

    /// `delete_node` — delete a node by its `id`.
    DeleteNode {
        id: String,
    },

    /// `attach_sensor` — attach a DAQ sensor to a node.
    AttachSensor {
        parent_id: String,
        daq_id: String,
        purpose: String,
        meter_type: String,
        #[serde(default)]
        unit: Option<String>,
        /// May be a JSON integer (`15`) or a form-encoded string (`"15"` or `""`).
        /// Validated/coerced downstream.
        #[serde(default)]
        resample_minutes: Option<Value>,
        /// Formula JSON blob.
        #[serde(default)]
        formula: Option<Value>,
    },

    /// `replace_sensor_device` — swap the DAQ device on an existing sensor.
    ReplaceSensorDevice {
        sensor_id: String,
        daq_id: String,
    },

    /// `create_user` — create a Cognito user and apply hierarchy access.
    CreateUser {
        email: String,
        name: String,
        /// Profile string (`"SysAdm"`, `"Developer"`, …); mapped to Cognito group downstream.
        profile: String,
        #[serde(default)]
        language: Option<String>,
        #[serde(default)]
        currency: Option<String>,
        /// Node IDs granted `Administrates` access.
        #[serde(default)]
        allowed: Vec<String>,
        /// Node IDs that are `Blocked`.
        #[serde(default)]
        blocked: Vec<String>,
    },

    /// `update_user` — update mutable fields of an existing user.
    UpdateUser {
        id: String,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        cognito_group: Option<String>,
        #[serde(default)]
        language: Option<String>,
        #[serde(default)]
        currency: Option<String>,
    },

    /// `delete_user` — the GUI sends `email`; also accept legacy `"id"` form.
    DeleteUser {
        /// Preferred identifier (the GUI sends this).
        #[serde(default)]
        email: Option<String>,
        /// Legacy fallback: a `"U#..."` user-id string.
        #[serde(default)]
        id: Option<String>,
    },

    /// `block_user` — block a user from a hierarchy node.
    BlockUser {
        user_id: String,
        node_id: String,
    },

    /// `unblock_user` — lift a block.
    UnblockUser {
        user_id: String,
        node_id: String,
    },

    /// `grant_administrates` — grant a user administrates access to a node.
    GrantAdministrates {
        user_id: String,
        node_id: String,
    },
}

impl Command {
    /// Resolve the effective user identifier for `DeleteUser`:
    /// prefer `email`, fall back to `id`.
    #[cfg(test)]
    pub fn delete_user_id(&self) -> Option<&str> {
        match self {
            Command::DeleteUser { email, id } => email.as_deref().or(id.as_deref()),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Form → JSON normaliser
// ---------------------------------------------------------------------------

/// The form keys that should be collected into JSON arrays rather than scalars.
const ARRAY_KEYS: &[&str] = &["allowed", "blocked"];

/// Parse an `application/x-www-form-urlencoded` body into `(key, value)` pairs,
/// URL-decoding both key and value.  `+` is treated as a space (standard form
/// encoding).
fn parse_form(body: &str) -> Vec<(String, String)> {
    body.split('&')
        .filter_map(|pair| {
            if pair.is_empty() {
                return None;
            }
            match pair.find('=') {
                None => {
                    let k = url_decode(pair);
                    Some((k, String::new()))
                }
                Some(i) => {
                    let k = url_decode(&pair[..i]);
                    let v = url_decode(&pair[i + 1..]);
                    Some((k, v))
                }
            }
        })
        .collect()
}

/// URL-decode a percent-encoded string, treating `+` as space.
fn url_decode(s: &str) -> String {
    // urlencoding::decode handles %XX; we pre-replace '+' → ' ' first.
    let with_spaces = s.replace('+', " ");
    urlencoding::decode(&with_spaces)
        .map(|cow| cow.into_owned())
        .unwrap_or_else(|_| with_spaces)
}

/// Strip the `"data."` prefix from a form key if present.
fn strip_data_prefix(k: &str) -> &str {
    k.strip_prefix("data.").unwrap_or(k)
}

/// Build a `serde_json::Value` (always `Value::Object`) from flat form fields:
///
/// 1. Strip the `"data."` prefix from every key.
/// 2. Collect repeated `allowed` / `blocked` keys into JSON arrays.
/// 3. All other fields become strings at their (possibly dotted-path) location.
pub fn form_to_command_json(form_body: &str) -> Value {
    let fields = parse_form(form_body);
    form_fields_to_json(&fields)
}

/// Internal: convert already-parsed fields into a JSON Value.
fn form_fields_to_json(fields: &[(String, String)]) -> Value {
    // Partition into array-keys and scalars.
    let (arrays, scalars): (Vec<_>, Vec<_>) =
        fields.iter().partition(|(k, _)| ARRAY_KEYS.contains(&strip_data_prefix(k)));

    // Build the base object from scalar fields using dotted-path nesting.
    let mut map = Map::new();
    for (k, v) in &scalars {
        let normed = strip_data_prefix(k);
        if normed.is_empty() {
            continue;
        }
        let parts: Vec<&str> = normed.split('.').collect();
        set_path(&mut map, &parts, v);
    }

    // Append array fields — collect all values for each array key.
    for &array_key in ARRAY_KEYS {
        let vals: Vec<Value> = arrays
            .iter()
            .filter(|(k, _)| strip_data_prefix(k) == array_key)
            .map(|(_, v)| Value::String(v.clone()))
            .collect();
        if !vals.is_empty() {
            map.insert(array_key.to_string(), Value::Array(vals));
        }
    }

    Value::Object(map)
}

/// Recursively set a dotted-path key inside a JSON object map.
/// Scalars only; no metadata coercion at this layer — the `Command` ADT uses
/// `String` fields everywhere.
fn set_path(map: &mut Map<String, Value>, path: &[&str], value: &str) {
    match path {
        [] => {}
        [key] => {
            map.insert((*key).to_string(), Value::String(value.to_string()));
        }
        [key, rest @ ..] => {
            let entry = map
                .entry((*key).to_string())
                .or_insert_with(|| Value::Object(Map::new()));
            if let Value::Object(nested) = entry {
                set_path(nested, rest, value);
            } else {
                // Key already exists as a scalar — replace with nested object.
                let mut nested = Map::new();
                set_path(&mut nested, rest, value);
                map.insert((*key).to_string(), Value::Object(nested));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// parse_command
// ---------------------------------------------------------------------------

/// Detect whether a body string looks like JSON (starts with `{` or `[`,
/// ignoring leading whitespace).
fn looks_like_json(s: &str) -> bool {
    s.trim_start()
        .chars()
        .next()
        .map(|c| c == '{' || c == '[')
        .unwrap_or(false)
}

/// Parse a raw HTTP body into a `Command`.
///
/// If the content-type is `application/x-www-form-urlencoded` *or* the body
/// doesn't look like JSON (HTMX default `text/plain` workaround), normalise
/// the body via `form_to_command_json` first; otherwise parse directly as JSON.
pub fn parse_command(
    content_type: Option<&str>,
    body: &str,
) -> Result<Command, serde_json::Error> {
    let is_form_ct = content_type
        .map(|ct| {
            ct.to_lowercase()
                .starts_with("application/x-www-form-urlencoded")
        })
        .unwrap_or(false);

    let is_form = is_form_ct || !looks_like_json(body);

    if is_form {
        let json = form_to_command_json(body);
        serde_json::from_value(json)
    } else {
        serde_json::from_str(body)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ---- form_to_command_json -----------------------------------------------

    /// `form_to_command_json` behaviour:
    /// repeated `data.allowed`/`data.blocked` become arrays; `data.` prefix stripped;
    /// percent-encoding decoded.
    #[test]
    fn form_array_create_user() {
        let form = "action=create_user\
            &data.email=a%40b\
            &data.name=X\
            &data.profile=SysAdm\
            &data.allowed=HN2%231\
            &data.allowed=HN2%232\
            &data.blocked=HN3%235";
        let v = form_to_command_json(form);
        assert_eq!(v["action"], json!("create_user"));
        assert_eq!(v["email"], json!("a@b"));
        assert_eq!(v["name"], json!("X"));
        assert_eq!(v["profile"], json!("SysAdm"));
        assert_eq!(v["allowed"], json!(["HN2#1", "HN2#2"]));
        assert_eq!(v["blocked"], json!(["HN3#5"]));
    }

    /// `+` in form body is decoded as space.
    #[test]
    fn form_plus_decoded_as_space() {
        let form = "action=add_node&data.name=Hello+World&data.parent_id=HN1%231";
        let v = form_to_command_json(form);
        assert_eq!(v["name"], json!("Hello World"));
        assert_eq!(v["parent_id"], json!("HN1#1"));
    }

    /// Keys without `data.` prefix pass through as-is.
    #[test]
    fn form_no_data_prefix() {
        let form = "action=delete_node&id=HN3%2342";
        let v = form_to_command_json(form);
        assert_eq!(v["action"], json!("delete_node"));
        assert_eq!(v["id"], json!("HN3#42"));
    }

    /// Empty `allowed`/`blocked` — if no array keys appear, no array key in output.
    #[test]
    fn form_no_array_keys_omitted() {
        let form = "action=delete_node&data.id=HN3%2310";
        let v = form_to_command_json(form);
        assert!(v.get("allowed").is_none());
        assert!(v.get("blocked").is_none());
    }

    /// Dotted sub-keys (e.g. `data.formula.kind`) become nested objects.
    #[test]
    fn form_nested_dotted_key() {
        let form = "action=attach_sensor\
            &data.parent_id=HN3%231\
            &data.daq_id=daq%3A1\
            &data.purpose=Electricity\
            &data.meter_type=counter\
            &data.formula.kind=expr\
            &data.formula.expr=abs%28self%29";
        let v = form_to_command_json(form);
        assert_eq!(v["formula"]["kind"], json!("expr"));
        assert_eq!(v["formula"]["expr"], json!("abs(self)"));
    }

    // ---- Command JSON deserialization ---------------------------------------

    /// `create_user` with profile + arrays parses into the right variant.
    #[test]
    fn parse_create_user_json() {
        let json = json!({
            "action": "create_user",
            "email": "alice@ex",
            "name": "Alice",
            "profile": "Developer",
            "language": "english",
            "currency": "EUR",
            "allowed": ["HN2#1", "HN2#2"],
            "blocked": ["HN3#5"]
        });
        let cmd: Command = serde_json::from_value(json).unwrap();
        assert_eq!(
            cmd,
            Command::CreateUser {
                email: "alice@ex".into(),
                name: "Alice".into(),
                profile: "Developer".into(),
                language: Some("english".into()),
                currency: Some("EUR".into()),
                allowed: vec!["HN2#1".into(), "HN2#2".into()],
                blocked: vec!["HN3#5".into()],
            }
        );
    }

    /// `create_user` without optional fields defaults correctly.
    #[test]
    fn parse_create_user_minimal() {
        let json = json!({
            "action": "create_user",
            "email": "bob@ex",
            "name": "Bob",
            "profile": "Reader",
        });
        let cmd: Command = serde_json::from_value(json).unwrap();
        assert_eq!(
            cmd,
            Command::CreateUser {
                email: "bob@ex".into(),
                name: "Bob".into(),
                profile: "Reader".into(),
                language: None,
                currency: None,
                allowed: vec![],
                blocked: vec![],
            }
        );
    }

    /// `delete_user` with `email` field.
    #[test]
    fn parse_delete_user_by_email() {
        let json = json!({ "action": "delete_user", "email": "bob@ex" });
        let cmd: Command = serde_json::from_value(json).unwrap();
        assert!(matches!(&cmd, Command::DeleteUser { email: Some(e), .. } if e == "bob@ex"));
        assert_eq!(cmd.delete_user_id(), Some("bob@ex"));
    }

    /// `delete_user` with `id` fallback (the `"U#..."` form).
    #[test]
    fn parse_delete_user_by_id() {
        let json = json!({ "action": "delete_user", "id": "U#bob@ex" });
        let cmd: Command = serde_json::from_value(json).unwrap();
        assert!(matches!(&cmd, Command::DeleteUser { email: None, id: Some(i) } if i == "U#bob@ex"));
        assert_eq!(cmd.delete_user_id(), Some("U#bob@ex"));
    }

    /// `email` takes precedence over `id` in `delete_user_id()`.
    #[test]
    fn delete_user_email_preferred_over_id() {
        let cmd = Command::DeleteUser {
            email: Some("alice@ex".into()),
            id: Some("U#alice@ex".into()),
        };
        assert_eq!(cmd.delete_user_id(), Some("alice@ex"));
    }

    /// `add_node` happy path.
    #[test]
    fn parse_add_node() {
        let json = json!({
            "action": "add_node",
            "parent_id": "HN2#10002",
            "name": "P",
            "level": "hn3",
            "metadata": {}
        });
        let cmd: Command = serde_json::from_value(json).unwrap();
        assert!(
            matches!(&cmd, Command::AddNode { parent_id, name, level, .. }
                if parent_id == "HN2#10002" && name == "P" && level.as_deref() == Some("hn3"))
        );
    }

    /// `delete_node` parses `id`.
    #[test]
    fn parse_delete_node() {
        let json = json!({ "action": "delete_node", "id": "HN3#42" });
        let cmd: Command = serde_json::from_value(json).unwrap();
        assert!(matches!(&cmd, Command::DeleteNode { id } if id == "HN3#42"));
    }

    /// `attach_sensor` with integer `resample_minutes`.
    #[test]
    fn parse_attach_sensor_with_int_resample() {
        let json = json!({
            "action": "attach_sensor",
            "parent_id": "HN3#1",
            "daq_id": "daq:1",
            "purpose": "Electricity",
            "meter_type": "counter",
            "unit": "kWh",
            "resample_minutes": 15
        });
        let cmd: Command = serde_json::from_value(json).unwrap();
        assert!(
            matches!(&cmd, Command::AttachSensor { resample_minutes: Some(Value::Number(n)), .. }
                if n.as_i64() == Some(15))
        );
    }

    /// `attach_sensor` with string `resample_minutes` (form path).
    #[test]
    fn parse_attach_sensor_with_string_resample() {
        let json = json!({
            "action": "attach_sensor",
            "parent_id": "HN3#1",
            "daq_id": "daq:1",
            "purpose": "Electricity",
            "meter_type": "counter",
            "resample_minutes": "15"
        });
        let cmd: Command = serde_json::from_value(json).unwrap();
        assert!(
            matches!(&cmd, Command::AttachSensor { resample_minutes: Some(Value::String(s)), .. }
                if s == "15")
        );
    }

    /// `replace_sensor_device` parses both required fields.
    #[test]
    fn parse_replace_sensor_device() {
        let json = json!({
            "action": "replace_sensor_device",
            "sensor_id": "S#old",
            "daq_id": "daq:new"
        });
        let cmd: Command = serde_json::from_value(json).unwrap();
        assert!(
            matches!(&cmd, Command::ReplaceSensorDevice { sensor_id, daq_id }
                if sensor_id == "S#old" && daq_id == "daq:new")
        );
    }

    /// `update_user` with optional fields absent defaults to `None`.
    #[test]
    fn parse_update_user_minimal() {
        let json = json!({ "action": "update_user", "id": "U#alice@ex" });
        let cmd: Command = serde_json::from_value(json).unwrap();
        assert!(
            matches!(&cmd, Command::UpdateUser { id, name: None, cognito_group: None, .. }
                if id == "U#alice@ex")
        );
    }

    /// `block_user` and `unblock_user` parse `user_id` + `node_id`.
    #[test]
    fn parse_block_unblock_user() {
        let block = json!({ "action": "block_user", "user_id": "U#dave@ex", "node_id": "HN2#10002" });
        let cmd: Command = serde_json::from_value(block).unwrap();
        assert!(
            matches!(&cmd, Command::BlockUser { user_id, node_id }
                if user_id == "U#dave@ex" && node_id == "HN2#10002")
        );

        let unblock = json!({ "action": "unblock_user", "user_id": "U#eve@ex", "node_id": "HN2#10002" });
        let cmd: Command = serde_json::from_value(unblock).unwrap();
        assert!(
            matches!(&cmd, Command::UnblockUser { user_id, node_id }
                if user_id == "U#eve@ex" && node_id == "HN2#10002")
        );
    }

    /// `grant_administrates` parses `user_id` + `node_id`.
    #[test]
    fn parse_grant_administrates() {
        let json = json!({ "action": "grant_administrates", "user_id": "U#alice@ex", "node_id": "HN2#1" });
        let cmd: Command = serde_json::from_value(json).unwrap();
        assert!(
            matches!(&cmd, Command::GrantAdministrates { user_id, node_id }
                if user_id == "U#alice@ex" && node_id == "HN2#1")
        );
    }

    // ---- parse_command (form vs JSON routing) --------------------------------

    /// JSON body routed directly.
    #[test]
    fn parse_command_json_body() {
        let body = r#"{"action":"delete_node","id":"HN3#42"}"#;
        let cmd = parse_command(None, body).unwrap();
        assert!(matches!(&cmd, Command::DeleteNode { id } if id == "HN3#42"));
    }

    /// Form content-type triggers form normalisation.
    #[test]
    fn parse_command_form_content_type() {
        let body = "action=delete_node&data.id=HN3%2342";
        let cmd = parse_command(Some("application/x-www-form-urlencoded"), body).unwrap();
        assert!(matches!(&cmd, Command::DeleteNode { id } if id == "HN3#42"));
    }

    /// Body that doesn't start with `{` is treated as form even without header.
    #[test]
    fn parse_command_sniff_form_no_content_type() {
        let body = "action=delete_node&data.id=HN3%2342";
        let cmd = parse_command(None, body).unwrap();
        assert!(matches!(&cmd, Command::DeleteNode { id } if id == "HN3#42"));
    }

    /// Full form round-trip: form body → `create_user` command with arrays.
    #[test]
    fn parse_command_form_create_user_with_arrays() {
        let body = "action=create_user\
            &data.email=a%40b\
            &data.name=X\
            &data.profile=SysAdm\
            &data.allowed=HN2%231\
            &data.allowed=HN2%232\
            &data.blocked=HN3%235";
        let cmd = parse_command(Some("application/x-www-form-urlencoded"), body).unwrap();
        assert_eq!(
            cmd,
            Command::CreateUser {
                email: "a@b".into(),
                name: "X".into(),
                profile: "SysAdm".into(),
                language: None,
                currency: None,
                allowed: vec!["HN2#1".into(), "HN2#2".into()],
                blocked: vec!["HN3#5".into()],
            }
        );
    }
}
