# Hierarchy Lambda — Rust/Graviton Rewrite — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Re-implement the OCaml `services/hierarchy` lambda + its onion model in Rust on Graviton (arm64), byte-compatible with the live DynamoDB data, folding Cognito provisioning back in-lambda.

**Architecture:** Cargo workspace `crates/model` (domain/logic/repository) + `crates/services/hierarchy` (the lambda), mirroring `../EMS`. Entities are `serde` + `TypedBuilder` structs; closed sets are enums (ADTs); logic functions are generic over async `FnOnce` closures (no repository traits); one codec module owns the `HN<n>#<id>` item layout. `anyhow` + `thiserror`; functional/immutable, minimal `mut`; `maud` for HTML.

**Tech Stack:** Rust 2021, tokio, lambda_http + aws_lambda_events, aws-sdk-dynamodb, aws-sdk-cognitoidentityprovider, aws-config, serde + serde_dynamo + serde_json, typed_builder, strum, maud, chrono, anyhow, thiserror; dev: rstest, proptest, serial_test; build: cargo-lambda.

**Reference sources (port from these):** OCaml in `services/hierarchy/lib/` (domain/, logic/, repo/, api/) + tests in `services/hierarchy/test/`; EMS shape in `../EMS/crates/model` + `../EMS/crates/services/hierarchy`. The live item format is documented in `services/hierarchy/lib/repo/codec.ml`.

**Parity rule:** every OCaml Alcotest case is ported 1:1 (same inputs/expected). Capture real items first (Phase 2) and assert codec round-trips against them.

---

## Phase 0 — Workspace scaffold

**Files:**
- Create: `Cargo.toml` (workspace), `crates/model/Cargo.toml`, `crates/model/src/lib.rs`, `crates/services/hierarchy/Cargo.toml`, `crates/services/hierarchy/src/main.rs`, `rust-toolchain.toml`, `.gitignore` additions.

### Task 0.1: workspace + crates compile

- [ ] **Step 1:** Create `Cargo.toml`:
```toml
[workspace]
resolver = "2"
members = ["crates/model", "crates/services/hierarchy"]

[workspace.package]
edition = "2021"

[workspace.dependencies]
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_dynamo = { version = "4", features = ["aws-sdk-dynamodb+1"] }
aws-config = { version = "1", features = ["behavior-version-latest"] }
aws-sdk-dynamodb = "1"
aws-sdk-cognitoidentityprovider = "1"
typed_builder = "0.20"
strum = { version = "0.26", features = ["derive"] }
maud = "0.26"
chrono = { version = "0.4", features = ["serde"] }
anyhow = "1"
thiserror = "2"
lambda_http = "0.13"
aws_lambda_events = "0.15"
rstest = "0.23"
proptest = "1"
serial_test = "3"
```
- [ ] **Step 2:** `crates/model/Cargo.toml` — package `model`, pull workspace deps actually used (serde, serde_dynamo, serde_json, aws-config, aws-sdk-dynamodb, aws-sdk-cognitoidentityprovider, typed_builder, strum, chrono, anyhow, thiserror; dev: rstest, proptest, serial_test). `crates/model/src/lib.rs`:
```rust
pub mod domain;
pub mod errors;
pub mod logic;
pub mod repository;
```
  Create empty `domain/mod.rs`, `logic/mod.rs`, `repository/mod.rs`, and `errors.rs` with `// placeholder`.
- [ ] **Step 3:** `crates/services/hierarchy/Cargo.toml` — package `hierarchy`, deps lambda_http, aws_lambda_events, tokio, anyhow, serde, serde_json, maud, strum, chrono, `model = { path = "../../model" }`. `main.rs`:
```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> { Ok(()) }
```
- [ ] **Step 4:** `rust-toolchain.toml` pinning `channel = "stable"`. Add to `.gitignore`: `/target`, `crates/**/target`.
- [ ] **Step 5: Run** `cargo build` → Expected: workspace builds clean.
- [ ] **Step 6: Commit** `git add Cargo.toml crates rust-toolchain.toml .gitignore && git commit -m "chore(rust): workspace scaffold (model + hierarchy crates)"`.

---

## Phase 1 — Domain (ids, values, entities)

Port `services/hierarchy/lib/domain/*.ml` to `crates/model/src/domain/`. ADTs for closed sets; `TypedBuilder` + `serde` for entities. Each module ports its OCaml test 1:1 into `#[cfg(test)]`.

**Files:** `domain/ids.rs`, `domain/values.rs`, `domain/node.rs`, `domain/sensor.rs`, `domain/user.rs`, `domain/mod.rs`.

### Task 1.1: `Level` + `NodeId` (port `level.ml`, `node_id.ml`)
- [ ] **Step 1 (test first):** in `domain/ids.rs`, port `test_domain_node_id.ml` + `test_domain_level.ml`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn node_id_render_parse() {
        assert_eq!(NodeId::make(Level::Hn2, 2).to_string(), "HN2#2");
        assert_eq!(NodeId::parse("HN2#2").unwrap(), NodeId::make(Level::Hn2, 2));
        assert_eq!(NodeId::Root.to_string(), "HN0#root");
        assert_eq!(NodeId::parse("HN0#root").unwrap(), NodeId::Root);
    }
    #[test] fn node_id_rejects_bad() { assert!(NodeId::parse("X#2").is_err()); }
    #[test] fn level_depth_roundtrip() {
        for d in 0..=9 { assert_eq!(Level::of_depth(d).unwrap().depth(), d); }
    }
}
```
- [ ] **Step 2: Run** `cargo test -p model ids` → FAIL (undefined).
- [ ] **Step 3:** implement `Level` (enum `Hn0..Hn9`, `depth()/of_depth()/to_string()="hn<n>"/parse`) and `NodeId` (`enum { Root, Node { level: Level, id: u32 } }`, `make/level/id/to_string="HN<n>#<id>"/parse/is_root`, `Display`, `PartialEq`). `#[derive(strum::EnumIter)]` on Level.
- [ ] **Step 4: Run** `cargo test -p model ids` → PASS.
- [ ] **Step 5: Commit** `feat(model): Level + NodeId with parse/render parity`.

### Task 1.2: value enums (port `edge_kind.ml`, `cognito_group.ml`, `profile.ml`, `currency.ml`, `language.ml`, `metadata.ml` field-types)
- [ ] **Step 1 (tests):** in `domain/values.rs` port `test_domain_edge_kind.ml`, the cognito_group/profile cases from `test_domain_user.ml` + `test_domain_profile.ml`:
```rust
#[test] fn profile_to_group() {
    use CognitoGroup::*;
    for (p, g) in [("SysAdm",Admin),("Developer",Writer),("Standard",Writer),("Technician",Reader),("Reader",Reader)] {
        assert_eq!(Profile::parse(p).unwrap().to_cognito_group(), g);
    }
}
#[test] fn cognito_group_capitalised_caseinsensitive() {
    assert_eq!(CognitoGroup::Admin.to_string(), "Admin");
    assert_eq!(CognitoGroup::parse("admin").unwrap(), CognitoGroup::Admin);
    assert_eq!(CognitoGroup::parse("Admin").unwrap(), CognitoGroup::Admin);
}
#[test] fn edge_kind_sk_verb() {
    assert_eq!(EdgeKind::HasLabel("building".into()).sk_verb(), "has_building");
    assert_eq!(EdgeKind::HasSensor.sk_verb(), "has_sensor");
    assert_eq!(EdgeKind::Administrates.sk_verb(), "administrates");
    assert_eq!(EdgeKind::Blocked.sk_verb(), "blocked");
}
```
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3:** implement enums with `parse`/`Display`/`to_string` matching the OCaml `of_string`/`to_string` exactly:
  - `EdgeKind { HasLabel(String), HasSensor, Blocked, Administrates }` — `sk_verb`, `kind_string` (`"has_label:<l>"` etc.), `parse`.
  - `CognitoGroup { Reader, Writer, Admin }` — `to_string` capitalised; `parse` lowercases then matches.
  - `Profile { Sysadm, Developer, Standard, Technician, Reader }` — `all()` (strum EnumIter), `to_string`, `parse`, `to_cognito_group`.
  - `Currency`, `Language` — variants + parse/to_string from `currency.ml`/`language.ml`.
  - `MeterType { Counter, Gauge }`; `FieldType` enum (String{min_len,max_len}, Number{min,max}, Integer{min,max}, Boolean, Timestamp, Enum{one_of:Vec<String>}) from `metadata.ml`.
- [ ] **Step 4: Run** → PASS. **Step 5: Commit** `feat(model): value ADTs (EdgeKind, CognitoGroup, Profile, Currency, Language, FieldType)`.

### Task 1.3: `Schema` + `Metadata` validation (port `schema.ml`, `metadata.ml`)
- [ ] **Step 1 (tests):** port `test_domain_schema.ml` + `test_domain_metadata.ml` validation cases (edge depth ordering, duplicate labels, min>max, field-spec validation).
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** `Schema { version: u32, edges: Vec<(Level, Vec<(Level, Vec<EdgeSpec>)>)>, metadata: Vec<(Level, Vec<(String, FieldSpec)>)>, sensors: Vec<Level> }` with `#[derive(TypedBuilder, Serialize, Deserialize, Clone)]`; `EdgeSpec { label, min: Option<i32>, max: Option<i32> }`; `FieldSpec { typ: FieldType, required: bool }`. Port `validate`, `allowed_children`, `metadata_for`, `allows_sensors`, and metadata `validate_one`/`validate`.
- [ ] **Step 4/5:** PASS, commit `feat(model): Schema + Metadata validation`.

### Task 1.4: `Formula` + parser (port `formula.ml`, `formula_parser.ml`)
- [ ] **Step 1 (tests):** port `test_domain_formula.ml` (parse + eval cases).
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** `enum Formula { Zero, Identity, Expr { refs: Vec<(String, SensorId)>, expr: Expr } }`; `enum Expr { Num(f64), Self_, Ref(String), Abs(Box<Expr>), Add(Box<Expr>,Box<Expr>), Sub(..), Mul(..), Div(..) }`. Port the recursive parser + `eval(self_reading, resolve)`.
- [ ] **Step 4/5:** PASS, commit `feat(model): Formula ADT + parser/eval`.

### Task 1.5: entities `Node`, `Sensor`, `SensorSk`, `SensorId`, `User`, `UserId` (port `node.ml`, `sensor.ml`, `sensor_sk.ml`, `sensor_id.ml`, `user.ml`, `user_id.ml`)
- [ ] **Step 1 (tests):** port `test_domain_sensor*.ml`, `test_domain_user.ml` (user_id render/parse, etc.).
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:**
  - `SensorId(u32)` newtype — `to_string="S#<n>"`, `parse`.
  - `enum SensorSk { Active(DateTime<Utc>), History(DateTime<Utc>) }` — `to_string` (`"active#<rfc3339>"` / `<rfc3339>`), `parse`.
  - `UserId(String email)` — `of_email`, `to_string="U#<email>"`, `parse`, `email`.
  - `#[derive(TypedBuilder, Serialize, Deserialize, Clone)] Node { id: NodeId, name, parent: Option<NodeId>, path: String, created: DateTime<Utc>, metadata: serde_json::Value, schema: Option<Schema> }` + `make(level,id,name,parent,parent_path,created,metadata,schema)`, `make_root`, `child_path`, `level`, `segment_at_level`.
  - `Sensor { id: SensorId, created, daq_id, path, purpose, meter_type: MeterType, unit: Option<String>, formula: Formula, resample_minutes: Option<i32> }` + `child_path`, `parent_id`.
  - `User { id: UserId, name, cognito_group: CognitoGroup, language: Language, currency: Currency, created }` (TypedBuilder; defaults for language/currency).
- [ ] **Step 4/5:** PASS, commit `feat(model): Node/Sensor/User entities (TypedBuilder + serde)`.

---

## Phase 2 — Codec (the single layout owner)

**Files:** `repository/dynamodb/codec.rs`, test fixtures `crates/model/tests/fixtures/*.json`.

### Task 2.1: capture live items as fixtures
- [ ] **Step 1:** with `AWS_PROFILE=stel-sb`, dump representative items to `crates/model/tests/fixtures/`: a company node `HN2#10003` (with schema), an `HN3`/`HN4` node, a `has_*` edge, a sensor `S#10010` + its `has_sensor` edge, a user `U#steen666@gmail.com`, an `administrates#` edge. Command per item:
```bash
aws dynamodb get-item --table-name hierarchy_new --key '{"pk":{"S":"HN2#10003"},"sk":{"S":"HN2#10003"}}' --output json > crates/model/tests/fixtures/node_hn2.json
```
- [ ] **Step 2: Commit** `test(model): capture live DynamoDB item fixtures`.

### Task 2.2: node/edge/sensor/user encode↔decode
- [ ] **Step 1 (tests):** in `codec.rs`, decode each fixture and assert the struct fields; then re-encode and assert the produced `HashMap<String, AttributeValue>` equals the fixture's `Item` (ignoring attribute order). Add a `proptest` round-trip: `encode(decode(item)) == item` for generated nodes/users.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** implement, porting `repo/codec.ml`:
  - `node_to_item(&Node) -> HashMap<String, AttributeValue>` and `node_of_item(&HashMap..) -> Result<Node>` (`pk=sk=id`, `type`, `name`, `created`, `metadata` via serde_dynamo, `gsi1pk="HN<n>"`, `gsi1sk=path`, `schema` when present via `schema_to_av`/`schema_of_av`).
  - `edge_item(...)`, `edge_with_anchor(...)`, `sensor_edge_item(...)` — `sk="has_<label>#<child>"`, `kind`, `gsi1pk`, `gsi1sk`.
  - `sensor_to_item(&Sensor, active: bool)` / `sensor_of_item`.
  - `user_to_item(&User)` / `user_of_item` (cognito_group capitalised; parse case-insensitive).
  - `schema_to_av`/`schema_of_av` — the nested-M `{version,edges,metadata,sensors}` encoding (`field_type_attr`/`decode_field_spec` parity, including `integer`/`boolean`).
  - `json_to_attr`/`attr_to_json` for `metadata`.
- [ ] **Step 4: Run** `cargo test -p model codec` → PASS (fixtures + proptest). **Step 5: Commit** `feat(model): DynamoDB codec, byte-compatible with live items`.

---

## Phase 3 — Logic (generic over closures)

Port `services/hierarchy/lib/logic/*.ml`. Each fn is generic over async `FnOnce` closures; tests inject an in-memory `HashMap`-backed fake (port of `repo/memory.ml`).

**Files:** `logic/hierarchy.rs`, `logic/access.rs`, `logic/users.rs`, `logic/sensors.rs`, `logic/schema_check.rs`, plus test helper `crates/model/src/repository/memory.rs` (test-only fake closures).

### Task 3.1: in-memory fake (port `repo/memory.ml`)
- [ ] **Step 1:** `memory.rs` — a `Store` (`HashMap<(String,String), Item>`) with helper closures matching the repository signatures (`get_node`, `put_node`, `list_child_refs`, `put_edge`, `delete_edge`, `list_administrated_nodes`, `list_blocked_nodes`, `get_user`, `put_user`, `delete_user`, `list_users`, `now`). Returns boxed async closures for tests. **Commit** `test(model): in-memory store + closures`.

### Task 3.2: access (port `access.ml`)
- [ ] **Step 1 (tests):** port `test_logic_access.ml` 1:1, including the **capitalised** `Writer` assertions and `delete_cascades_*`, and the **start-nodes** behaviour (a user administrating an HN2 shows it; root grant → children).
- [ ] **Step 2:** FAIL. **Step 3:** implement `ancestors_of`, `effective_permission`, `block`/`unblock`, `grant_administrates`, `has_admin_access`, `list_administrated_nodes`, `start_refs(uid, list_admin, list_child_refs, get_node)` — all generic over the relevant closures. **Step 4/5:** PASS, commit `feat(model): access logic (grants, has_admin_access, start nodes)`.

### Task 3.3: users (port `users.ml`)
- [ ] **Step 1 (tests):** port `test_logic_users.ml` 1:1 (create/duplicate/get/update/delete; **delete cascades Administrates AND Blocked**).
- [ ] **Step 2:** FAIL. **Step 3:** `create<Save,F>(user, grants, save)`, `update`, `delete<List,Del,F..>(id, list_admin, list_blocked, delete_edge, delete_user)`, `list`. **Step 4/5:** PASS, commit `feat(model): users logic (+ grant + cascade delete)`.

### Task 3.4: hierarchy, sensors, schema_check (port `hierarchy.ml`, `sensors.ml`, `schema_check.ml`)
- [ ] **Step 1 (tests):** port `test_logic_hierarchy.ml`, `test_logic_sensors.ml`, `test_logic_schema_check.ml`, `test_logic_properties.ml` 1:1.
- [ ] **Step 2:** FAIL. **Step 3:** implement `add_node`, `list_child_refs`/`list_children`, `get_node`; sensor `attach`/`list_active`/`get_active`/`replace_device`/`evaluate`/`list_under_company`; `schema_check::find_for`. **Step 4/5:** PASS, commit `feat(model): hierarchy + sensors + schema_check logic`.

---

## Phase 4 — Repository (concrete async fns)

**Files:** `repository/dynamodb/{node.rs,edge.rs,user.rs,sensor.rs}`, `repository/cognito/user.rs`, `errors.rs`.

### Task 4.1: errors
- [ ] **Step 1:** `errors.rs` — `#[derive(thiserror::Error)] RepositoryError { NotFound, NotFoundUser, Conflict(String), Validation(String), Aws(String), Codec(String) }` and `DomainError`. `From<aws_sdk_dynamodb::Error>` etc. **Commit** `feat(model): error enums`.

### Task 4.2: dynamodb fns (port `repo/dynamo.ml`)
- [ ] **Step 1 (integration tests, `#[ignore]` by default / feature `it`):** round-trip a node/user/edge against a real table (uses fixtures table name from env).
- [ ] **Step 2/3:** plain `pub async fn get_node(client:&Client, table:&str, id:&NodeId) -> Result<Option<Node>, RepositoryError>` etc. (query_child_edges/refs with `begins_with`, `query_administrated_nodes`, `query_active_sensor` with `active#`, `delete_subtree`, `allocate_and_put_node`/`allocate_and_put_sensor` id allocation). Use `codec` for mapping; never re-encode outside `codec`.
- [ ] **Step 4/5:** `cargo test -p model --features it` (when a table is available); commit `feat(model): dynamodb repository fns`.

### Task 4.3: cognito fns + lazy clients (port EMS `repository/cognito/user.rs`, `lib.rs`)
- [ ] **Step 1/2/3:** `cognito/user.rs` — `create_cognito_user(client, pool, email, name)`, `add_user_to_group(client, pool, email, group)`, `delete_cognito_user(client, pool, email)` (ignore `UsernameExists`/`UserNotFound`). `lib.rs` — `OnceCell` `get_dynamodb_client()`, `get_cognito_client()`, `get_table()`, `get_user_pool_id()` (env). **Step 4/5:** commit `feat(model): cognito repository + lazy clients`.

---

## Phase 5 — HTML (maud)

**Files:** `crates/services/hierarchy/src/html/{mod.rs,tree.rs,node.rs,forms.rs}`.

### Task 5.1: tree + permission rows (port `api_html.ml` `list_item`/`render_nodes`/`tree_toggle`)
- [ ] **Step 1 (snapshot tests):** assert the rendered `<li>` contains the exact attributes the frontend needs: `data-id`, `data-node-id`, `data-node-path`, the hyperscript `_="on click … @data-node-path"`, `class="permission-row"` + `name="data.allowed"`/`data.blocked` checkboxes for `with_permissions`, `class="child-rows"` with the `loadChildren` htmx attrs.
- [ ] **Step 2:** FAIL. **Step 3:** `tree.rs` with `list_item(...) -> Markup` and `render_nodes_li(refs, ...) -> Markup`, porting the OCaml structure exactly. **Step 4/5:** commit `feat(hierarchy): maud tree + permission rows`.

### Task 5.2: node detail, sensor list, option lists, dialogs (port the rest of `api_html.ml`)
- [ ] **Step 1 (snapshot tests):** node detail (`id`/`name`/metadata table/`add_child_block`/`sensor_block`), `render_sensors`, `render_profiles` = `Profile::all` names, `render_languages`/`currencies`/`timezones`/`permissions`, the add-sensor/add-child dialogs.
- [ ] **Step 2/3/4:** implement in `node.rs`/`forms.rs`. **Step 5:** commit `feat(hierarchy): maud node detail + forms + option lists`.

---

## Phase 6 — Service (main.rs + dispatch)

**Files:** `crates/services/hierarchy/src/main.rs`, `command.rs`, `query.rs`.

### Task 6.1: form/command parsing (port `handler.ml` + `api_command.ml`)
- [ ] **Step 1 (tests):** port the form-array test (`data.allowed`/`data.blocked` → Vec) and the `Command` parse cases.
- [ ] **Step 2:** FAIL. **Step 3:** `command.rs` — `#[derive(Deserialize)] #[serde(tag="action", rename_all="snake_case")] enum Command { AddNode{..}, DeleteNode{..}, CreateUser{email,name,profile,language,currency,#[serde(default)] allowed:Vec<String>, #[serde(default)] blocked:Vec<String>}, UpdateUser{..}, DeleteUser{email}, AttachSensor{..}, ReplaceSensorDevice{..}, BlockUser{..}, UnblockUser{..}, GrantAdministrates{..} }`; a form→JSON normaliser (strip `data.`, collect repeated `allowed`/`blocked` into arrays — port `form_to_command_json`). **Step 4/5:** commit `feat(hierarchy): Command ADT + form parsing`.

### Task 6.2: command dispatch wiring (port `api_command.ml` run_* + create_user_full)
- [ ] **Step 1 (tests):** port `test_api_command.ml` 1:1 against the in-memory fake (create_user with profile → group + grants; delete_user by email + cascade).
- [ ] **Step 2:** FAIL. **Step 3:** for each `Command`, call the matching `logic::*` fn with closures wrapping `repository::dynamodb::*` and (for users) `repository::cognito::*` with **rollback** on Cognito failure (port EMS `create_user_full`/`delete_user`). **Step 4/5:** commit `feat(hierarchy): command dispatch (+ sync Cognito + rollback)`.

### Task 6.3: query dispatch + routing (port `api_query.ml`, `api_html.ml` dispatch, `handler.ml` routing)
- [ ] **Step 1 (tests):** port `test_api_query.ml` 1:1, incl. **top-level shows administrated HN2** and `effective_permission` cases.
- [ ] **Step 2:** FAIL. **Step 3:** `query.rs` — JSON queries (`get_node`/`list_children`/`list_users`/`list_sensors`/`list_blocked_*`/`effective_permission`) + HTML queries (`nodes`/`node`/`sensors`/`profiles`/…). `main.rs` — `lambda_http::run(service_fn(handler))`; `handler` routes on `(method, path)`, parses query/body, dispatches to `command`/`query`, returns the response with CORS headers (port `create_response`). **Step 4/5:** commit `feat(hierarchy): query dispatch + lambda routing`.

### Task 6.4: full test parity audit
- [ ] **Step 1:** enumerate every `Alcotest.test_case` in `services/hierarchy/test/*.ml`; confirm each has a ported Rust test. List + close gaps.
- [ ] **Step 2: Run** `cargo test` (workspace) → all green; count ≈ the OCaml 156. **Step 3: Commit** `test: full parity audit vs OCaml suite`.

---

## Phase 7 — Build, deploy parallel, response-diff

**Files:** `crates/services/hierarchy/Cargo.toml` (release profile), `infra/hierarchy/app.go` (a *second* lambda + URL), a diff script `scripts/parity_diff.sh`.

### Task 7.1: arm64 build
- [ ] **Step 1:** install/confirm `cargo lambda` (`cargo lambda --version`). Add release profile to root `Cargo.toml`:
```toml
[profile.release]
opt-level = "z"
lto = true
strip = true
codegen-units = 1
```
- [ ] **Step 2: Run** `cargo lambda build --release --arm64 -p hierarchy` → produces `target/lambda/hierarchy/bootstrap`. **Step 3: Commit** `chore(hierarchy): arm64 cargo-lambda build`.

### Task 7.2: deploy as a second function + diff
- [ ] **Step 1:** in `infra/hierarchy/app.go`, add `OcamlHierarchyFunctionRust` (Runtime `provided.al2023`, Architecture `arm64`, Code = the Rust zip, role = DynamoDB read/write on the table + `cognito-idp` on the pool + `USER_POOL_ID` env) behind its own Function URL output; **do not** touch the existing lambda yet.
- [ ] **Step 2: Deploy** (`AWS_PROFILE=stel-sb`, `unset GOROOT`, `cdk diff` → only `[+]` new fn/role/url, `cdk deploy OcamlHierarchyStack`).
- [ ] **Step 3:** `scripts/parity_diff.sh` — for every endpoint (`query/nodes`, `node`, `users`, `profiles`, …, and read-only command echoes), `curl` both the OCaml API and the Rust Function URL with the same inputs and `diff` the bodies. Iterate until identical. **Step 4: Commit** `chore: parallel Rust lambda + parity-diff script`.

---

## Phase 8 — Cutover

### Task 8.1: swap + remove
- [ ] **Step 1:** in `app.go`, point `OcamlHierarchyFunction` at the Rust zip + `provided.al2023` + `arm64` + cognito IAM/env; delete the temporary second function; **remove** the Go cognito-sync function, its DynamoDB-stream event source, and DLQ. `cdk diff` → Lambda `[~]` (code/runtime/arch) + cognito-sync resources `[-]`, **table untouched**.
- [ ] **Step 2: Deploy**; verify via the live API (`/hierarchy/query/users`, create/delete a throwaway user → Cognito mirrored synchronously). **Step 3:** `git rm -r services/hierarchy` (OCaml) + `infra/hierarchy/lambda/cognito-sync`; update `CLAUDE.md` Stack 1 (now Rust/arm64, no OCaml/smaws notes, no Go cognito-sync). **Step 4: Commit** `feat: cut hierarchy over to Rust/Graviton; remove OCaml + Go cognito-sync`.

---

## Self-review notes
- **Spec coverage:** layout (P0), single-place serde/TypedBuilder/ADT (P1), HOF logic (P3), codec single owner (P2), byte-compat (P2 fixtures), Cognito sync+rollback (P4/P6.2), maud (P5), errors anyhow+thiserror (P4.1/P6), identical tests + proptest (every phase + P6.4), arm64 + reuse stack (P7/P8), cutover (P8) — all covered.
- **Independently testable:** P1–P6 are `cargo test -p model` / `-p hierarchy` green on their own; P7 adds a non-destructive parallel function; P8 is the only destructive step and is last.
- **Type consistency:** `NodeId`, `CognitoGroup`, `Profile`, `EdgeKind`, `Command` names are used consistently across phases; closures' signatures are defined where each logic fn is introduced.
