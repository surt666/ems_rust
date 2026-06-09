# Hierarchy Lambda — Rust / Graviton Rewrite — Design

**Goal:** Re-implement the OCaml `services/hierarchy` lambda and its onion model in Rust on
Graviton (arm64), preserving every current behaviour and the exact on-the-wire DynamoDB format,
while the AWS SDK is mature (cognito-idp included). Functional, immutable-first, ADT-heavy, with
databases/effects injected as higher-order functions rather than traits.

**Why:** the OCaml AWS SDK (smaws, preview) is too immature — no cognito-idp, stale-connection
hangs, log-dump bugs we've had to patch. Rust's `aws-sdk-*` is production-grade and lets us fold
Cognito provisioning back into the request path.

**Scope:** the hierarchy lambda + its model only. NOT the aggregations lambda (already Go), the
Flink/Glue pipeline, or the frontend. The frontend (Astro/htmx) is unchanged — the Rust lambda
must emit byte-identical HTML fragments and JSON.

---

## 1. Layout — a Cargo workspace mirroring `../EMS`

```
crates/
  model/                         # onion model
    src/
      lib.rs                     # re-exports + lazy OnceCell AWS clients (DynamoDB, Cognito)
      errors.rs                  # RepositoryError, DomainError (thiserror)
      domain/
        mod.rs
        ids.rs                   # NodeId, UserId, SensorId
        values.rs                # Level, EdgeKind, CognitoGroup, Profile, Currency,
                                 #   Language, MeterType, Permission   (all enums)
        node.rs                  # Node, Schema, EdgeSpec, Metadata field-types
        sensor.rs                # Sensor, SensorSk, Formula (recursive enum)
        user.rs                  # User
      logic/
        mod.rs
        hierarchy.rs             # node tree / child refs
        access.rs                # administrates/blocked, has_admin_access, start nodes
        users.rs                 # create/update/delete (+ grants)
        sensors.rs
        schema_check.rs
      repository/
        mod.rs
        dynamodb/{mod.rs,codec.rs,node.rs,edge.rs,user.rs,sensor.rs}
        cognito/{mod.rs,user.rs}
  services/hierarchy/            # the lambda
    src/
      main.rs                    # lambda_http runtime + HTTP routing + form parsing + dispatch
      command.rs                 # Command ADT + handlers (wire repo fns into logic closures)
      query.rs                   # query handlers (JSON + HTML)
      html/{mod.rs,tree.rs,node.rs,forms.rs}   # maud renderers
    Cargo.toml
  Cargo.toml                     # [workspace]
```

The OCaml `services/hierarchy` and the Go cognito-sync lambda stay live until the Rust passes
parity, then both are deleted.

## 2. The "change one place" rules

1. **Entities are `serde` + `TypedBuilder` structs.** Adding a field (a building-metadata field,
   a user field) edits the struct *only*; `serde` + `serde_dynamo` + `serde_json` do (de)serialization.
   No parallel DTO/trait/impl to touch. Construction is `Node::builder().name(..).build()`.
2. **Logic is generic over async closures**, never traits:
   ```rust
   pub async fn create_user<Save, F>(user: User, grants: Vec<Grant>, save: Save) -> Result<User>
   where Save: FnOnce(User, Vec<Grant>) -> F, F: Future<Output = Result<User, RepositoryError>>;
   ```
   The signature *is* the contract; `command.rs`/`query.rs` partially-apply the real repository
   functions. No `RepositoryTrait` objects.
3. **One layout owner.** `repository/dynamodb/codec.rs` is the only module that knows the
   `HN<n>#<id>` keys, `gsi1pk/gsi1sk`, the `schema` map encoding, edge `sk = "has_<label>#<child>"`,
   and sensor `active#<ts>`. Everything above sees plain structs.
4. **ADTs for everything that is a closed set of cases** — `Level`, `EdgeKind`, `CognitoGroup`,
   `Profile`, `Currency`, `Language`, `MeterType`, `Permission`, `Metadata` field-types,
   `Formula` (recursive), the `Command` payload, and the error types. Adding a case is one `match`
   arm the compiler points you to.

## 3. Codec — byte-compatible with today's data

The codec replicates `services/hierarchy/lib/repo/codec.ml` exactly, verified against live items:

- **Node item:** `pk = sk = "HN<n>#<id>"`, `type="node"`, `name`, `created` (RFC3339Z),
  `metadata` (M), `gsi1pk="HN<n>"`, `gsi1sk=<path>` (`|`-joined `HN0#root|HN1#..|..`), and `schema`
  (only on HN2). Schema = `{version, edges, metadata, sensors}` in the nested-M form.
- **Edge item:** `pk=<parent>`, `sk="has_<label>#<child>"`, `type="edge"`, `kind="has_label:<label>"`,
  `name=<child name>`, `gsi1pk="HN<child depth>"`, `gsi1sk=<child path>`. User edges:
  `administrates#<node>`, `blocked#<node>`.
- **Sensor item:** `pk="S#<id>"`, `sk="active#<ts>"`, `daq_id`, `purpose`, `meter_type`, `formula`,
  `resample_minutes`, `unit`, `gsi1pk="S"`, `gsi1sk=<path>`; sensor edge `has_sensor#S#<id>`.
- **User item:** `pk=sk="U#<email>"`, `type="user"`, `name`, `cognito_group` (**capitalised**
  `Admin`/`Writer`/`Reader`), `language`, `currency`, `created`, `gsi1pk="user"`.

Where pure `serde_dynamo` derive can't express a convention (composite keys, the path, the schema
blob), the codec maps by hand — but only here.

## 4. Behaviours preserved (parity checklist)

Commands (`POST /hierarchy/command`): `add_node`, `delete_node`, `attach_sensor`,
`replace_sensor_device`, `create_user`, `update_user`, `delete_user`, `block_user`,
`unblock_user`, `grant_administrates`.

Queries (`GET /hierarchy/query/*`): `nodes` (tree, incl. `permissions=true`), `node` (detail),
`sensors`, `users`, `profiles`, `languages`, `currencies`, `permissions`, `timezones`,
`add_child_form`; JSON: `get_node`, `get_sensor`, `get_user`, `list_children`, `list_users`,
`list_sensors`, `list_blocked_nodes`, `list_blocked_users`, `effective_permission`.

Recent additions kept: 5-profile→3-group mapping (`SysAdm→Admin`, `Developer/Standard→Writer`,
`Technician/Reader→Reader`); capitalised `CognitoGroup` (case-insensitive parse); access grants on
create (`allowed→Administrates`, `blocked→Blocked`); delete cascades **all** user edges;
top-level tree = the user's administrated nodes as start nodes (root grant → partners).

## 5. Cognito — synchronous, in-lambda

`create_user`: write user + grant edges → `AdminCreateUser` (invite email) +
`AdminAddUserToGroup`; if Cognito fails, **roll back** the DynamoDB user (EMS `create_user_full`
pattern). `delete_user`: cascade edges + `AdminDeleteUser` (ignore `UserNotFound`).
The Go cognito-sync lambda, its DynamoDB-stream event source, and DLQ are removed.

## 6. Errors & style

`Result` everywhere. `thiserror` enums for `RepositoryError` and `DomainError`; `anyhow::Result`
at the lambda boundary with `?`. Functional/immutable: iterators/`map`/`fold` over loops,
return-new over mutate; `mut` only where idiomatic (building a `Vec`, SDK builders, the maud
buffer). No shared mutable state beyond the `OnceCell` clients.

## 7. HTML — maud

`html/` builds every fragment with the `maud` macro — compile-time-checked, one fragment per
function, no template files or context structs. Fragments must match the OCaml output the frontend
relies on: the tree `<li>`/permission rows (with the `data-id`/`data-node-path`/hyperscript
attributes), node detail, sensor list, `<option>` lists, dialogs.

## 8. Testing

- **Identical to OCaml.** Every Alcotest case is ported 1:1 (same inputs, same expected values):
  `node_id`/`level`/`edge_kind` round-trips, schema/metadata validation, sensor_sk, formula
  parse/eval, `profile→cognito_group`, `cognito_group` case-insensitive parse + capitalised render,
  access (grant/block, has_admin_access, start nodes), users (create/update/delete + cascade),
  command/query dispatch, **codec round-trips against real captured items**.
- **HOF payoff:** logic tests inject in-memory closures (a `HashMap`-backed fake) — no DynamoDB,
  same coverage as OCaml's memory repo.
- **Property-based (`proptest`):** codec encode∘decode = id; path build/parse round-trip;
  `profile→group` total; `bucket_label`/id parsing invariants.
- maud fragments: snapshot assertions on key attributes.

## 9. Build & deploy

`cargo lambda build --release --arm64` → `bootstrap` (provided.al2023, arm64). In the **existing**
`OcamlHierarchyStack` (Go CDK): point the API lambda's `Code` at the Rust zip, set
`Runtime=provided.al2023`, `Architecture=arm64`, add `cognito-idp` IAM (scoped to
`userpool/eu-central-1_gADB2vK24`) + `USER_POOL_ID` env. **Table + stream untouched.**

**Cutover:** deploy the Rust lambda as a *second* function (own Function URL/API) first; diff its
responses against the OCaml lambda for every endpoint until identical; then swap the stack's lambda
Code/Runtime/Arch and delete the OCaml crate + the Go cognito-sync stack resources.

## 10. Out of scope / risks

- Out: aggregations lambda, Flink/Glue, frontend changes, the `meter-identity` bridge (it consumes
  *sensor* rows — unaffected by the user-row change; it stays).
- Risk: HTML/JSON parity — mitigated by response-diffing during the parallel phase.
- Risk: codec drift — mitigated by round-trip tests against captured live items.
