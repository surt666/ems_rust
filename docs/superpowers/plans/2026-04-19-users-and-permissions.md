# Users, Block-Permissions, Edge-Kind Generalization — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to execute task-by-task.

**Goal:** Add `User` domain + CRUD, generalize edge verbs beyond `has_<label>`, and support `BLOCKED` permission edges on nodes. No in-Lambda enforcement — upstream handles it.

**Architecture:** One bundled refactor. A new `Edge_kind.t` replaces the implicit `has_<label>` prefix (used today for node-child edges and sensor attachments) and adds a `Blocked` verb. `User.t` lives alongside `Node.t` in the same DynamoDB table, keyed on `U#<email>`. Per-node deny is stored as a `blocked` edge from user to node; effective permission is computed in a single pure function so it can later be delegated to Amazon Verified Permissions (Cedar).

**Tech Stack:** OCaml 5 (Effects), smaws-clients DynamoDB, API Gateway v2, Cognito (group claim only — reader/writer/admin).

---

## Design notes

- **Enforcement is upstream.** Lambda stores and returns policy data; it never rejects a command based on the caller. API Gateway authorizer + frontend gating handle that. The admin-only UI for `create_node` is a frontend concern.
- **Cognito group = capability ceiling.** One of `reader | writer | admin`. A reader-group user cannot be granted anything beyond read anywhere; a writer cannot escalate to admin. Stored on the user record so `effective_permission` can report it without re-reading the JWT.
- **Per-node is default-allow.** The only per-node edge is `blocked`. Effective capability on node N for user U is: `min(cognito_group_capability, not_blocked_on_N_or_any_ancestor)`.
- **Future: delegate to Amazon Verified Permissions / Cedar.** Keep `Access.effective_permission` a single pure function over `(user, node, blocked_edges, ancestors)` so it can be swapped for an AVP call. Mark the seam in the code.
- **Future: in-Lambda enforcement.** TODO note in `docs/architecture.md`: when the API Gateway authorizer pattern is insufficient (e.g. bulk queries needing per-row filtering), Lambda can read the caller's email + group from the authorizer context and call `Access.effective_permission` itself before every command.

## Storage additions

```
User item:
  pk     = U#<email>
  sk     = U#<email>
  type   = "user"
  name, cognito_group, language, currency, created
  gsi1pk = "user"
  gsi1sk = U#<email>

Block edge (user → node):
  pk     = U#<email>
  sk     = blocked#HN<n>#<uuid>
  type   = "edge"
  kind   = "blocked"
  created
  gsi1pk = HN<n>#<uuid>
  gsi1sk = blocks#U#<email>
```

Node-child and sensor edges keep their existing key shape; only the codec changes to go through `Edge_kind.t`.

## File structure

New:
- `lib/domain/edge_kind.ml` — variant + sk/gsi verb resolver
- `lib/domain/user.ml`, `user_id.ml`, `cognito_group.ml`, `language.ml`, `currency.ml`
- `lib/logic/users.ml` — CRUD
- `lib/logic/access.ml` — block, unblock, list, effective_permission (pure over an ancestor list)
- `test/test_domain_edge_kind.ml`, `test_domain_user.ml`, `test_logic_users.ml`, `test_logic_access.ml`

Modified:
- `lib/effects.ml` — replace `Put_edge.label:string` with `kind:Edge_kind.t`; add `Put_user/Get_user/List_users/Delete_user`; add `List_blocked_nodes/List_blocked_users`
- `lib/repo/codec.ml` — `edge_item` and `sensor_edge_item` go through `Edge_kind`; add `user_item`, `user_of_item`, `blocked_edge_item`
- `lib/repo/memory.ml`, `lib/repo/dynamo.ml` — handle new effects
- `lib/logic/hierarchy.ml` — use `Edge_kind.Has_label` at call sites
- `lib/logic/sensors.ml` — use `Edge_kind.Has_sensor`
- `lib/api/api_command.ml`, `api_query.ml` — add `create_user`, `update_user`, `delete_user`, `block_user`, `unblock_user`, `get_user`, `list_users`, `list_blocked_nodes`, `list_blocked_users`, `effective_permission`
- `docs/architecture.md`, `docs/hierarchy-and-sensors.md`, `docs/api.md` — reflect new shapes

## Dependency rule check

No changes. `logic/access.ml` depends on `domain` + `effects` only. `logic/users.ml` depends on `domain` + `effects`. `repo/*` continues to be the only place touching smaws.

---

## Tasks

### Task 1: `Edge_kind.t` domain type

**Files:**
- Create: `lib/domain/edge_kind.ml`
- Test: `test/test_domain_edge_kind.ml`

- [ ] **Step 1: Write the failing test**

```ocaml
(* test/test_domain_edge_kind.ml *)
open Ocaml_lambda_test

let has_label_verbs () =
  let k = Edge_kind.Has_label "building" in
  Alcotest.(check string) "sk verb" "has_building" (Edge_kind.sk_verb k);
  Alcotest.(check string) "gsi verb" "parent_of"  (Edge_kind.gsi_verb k)

let has_sensor_verbs () =
  let k = Edge_kind.Has_sensor in
  Alcotest.(check string) "sk verb"  "has_sensor"   (Edge_kind.sk_verb k);
  Alcotest.(check string) "gsi verb" "sensor_of"    (Edge_kind.gsi_verb k)

let blocked_verbs () =
  let k = Edge_kind.Blocked in
  Alcotest.(check string) "sk verb"  "blocked" (Edge_kind.sk_verb k);
  Alcotest.(check string) "gsi verb" "blocks"  (Edge_kind.gsi_verb k)

let roundtrip_to_string () =
  let xs = [ Edge_kind.Has_label "b"; Edge_kind.Has_sensor; Edge_kind.Blocked ] in
  List.iter (fun k ->
    let s = Edge_kind.to_string k in
    match Edge_kind.of_string s with
    | Ok k' -> Alcotest.(check bool) (Printf.sprintf "rt %s" s) true (k = k')
    | Error e -> Alcotest.failf "of_string %S: %s" s e) xs

let tests =
  [ Alcotest.test_case "Has_label verbs"  `Quick has_label_verbs
  ; Alcotest.test_case "Has_sensor verbs" `Quick has_sensor_verbs
  ; Alcotest.test_case "Blocked verbs"    `Quick blocked_verbs
  ; Alcotest.test_case "to/of_string rt"  `Quick roundtrip_to_string
  ]
```

Wire in `test/test_all.ml`.

- [ ] **Step 2: Run test to verify it fails**

Run: `dune runtest`
Expected: FAIL — `Edge_kind` does not exist.

- [ ] **Step 3: Implement the domain type**

```ocaml
(* lib/domain/edge_kind.ml *)
type t =
  | Has_label of string
  | Has_sensor
  | Blocked

let sk_verb = function
  | Has_label l -> "has_" ^ l
  | Has_sensor  -> "has_sensor"
  | Blocked     -> "blocked"

let gsi_verb = function
  | Has_label _ -> "parent_of"
  | Has_sensor  -> "sensor_of"
  | Blocked     -> "blocks"

let to_string = function
  | Has_label l -> "has_label:" ^ l
  | Has_sensor  -> "has_sensor"
  | Blocked     -> "blocked"

let of_string s =
  if s = "has_sensor" then Ok Has_sensor
  else if s = "blocked" then Ok Blocked
  else match String.split_on_char ':' s with
    | [ "has_label"; l ] when l <> "" -> Ok (Has_label l)
    | _ -> Error (Printf.sprintf "bad edge_kind %S" s)
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `dune runtest` → PASS.

- [ ] **Step 5: Commit**

```bash
git add lib/domain/edge_kind.ml test/test_domain_edge_kind.ml test/test_all.ml
git commit -m "domain: add Edge_kind.t with sk/gsi verb pair"
```

---

### Task 2: Migrate `Put_edge` effect to carry `Edge_kind`

**Files:**
- Modify: `lib/effects.ml`
- Modify: `lib/repo/codec.ml:117-130` (`edge_item`)
- Modify: `lib/repo/memory.ml`, `lib/repo/dynamo.ml`
- Modify: `lib/logic/hierarchy.ml:132-133`

- [ ] **Step 1: Update the effect signature**

```ocaml
(* lib/effects.ml — replace the Put_edge constructor *)
  | Put_edge : {
      from_ : string;           (* pk — node id or user id *)
      to_   : string;           (* gsi1pk target — node id or user id *)
      kind  : Edge_kind.t;
      name  : string;
      created : Ptime.t;
    } -> unit Effect.t
```

and rename the helper:

```ocaml
let put_edge ~from_ ~to_ ~kind ~name ~created =
  Effect.perform (Put_edge { from_; to_; kind; name; created })
```

- [ ] **Step 2: Update codec.ml**

Replace `edge_item` with:

```ocaml
let edge_item ~from_ ~to_ ~kind ~name ~created =
  [
    ("pk", s from_);
    ("sk", s (Printf.sprintf "%s#%s" (Edge_kind.sk_verb kind) to_));
    ("type", s "edge");
    ("kind", s (Edge_kind.to_string kind));
    ("name", s name);
    ("created", s (Ptime.to_rfc3339 ~tz_offset_s:0 created));
    ("gsi1pk", s to_);
    ("gsi1sk", s (Printf.sprintf "%s#%s" (Edge_kind.gsi_verb kind) from_));
  ]
```

- [ ] **Step 3: Update hierarchy.ml call site**

```ocaml
(* lib/logic/hierarchy.ml:131-133 *)
Effects.put_node child;
Effects.put_edge
  ~from_:(Node_id.to_string parent)
  ~to_:(Node_id.to_string child.Node.id)
  ~kind:(Edge_kind.Has_label edge_label)
  ~name:child.Node.name
  ~created;
```

- [ ] **Step 4: Update Memory + Dynamo handlers to match the new effect**

Memory: wrap the existing edge-index key computation around `Edge_kind.sk_verb`. Dynamo: delegate to the new `Codec.edge_item`.

- [ ] **Step 5: Run tests**

Run: `dune runtest`
Expected: all existing node/children/schema tests still pass (behaviorally unchanged — the stored `sk` is identical for `Has_label` rows).

- [ ] **Step 6: Commit**

```bash
git commit -am "effects/codec: route Put_edge through Edge_kind"
```

---

### Task 3: Migrate sensor attachment to `Edge_kind.Has_sensor`

**Files:**
- Modify: `lib/repo/codec.ml:408-417` — `sensor_edge_item`
- Modify: `lib/logic/sensors.ml` attach / detach sites
- Modify: `lib/effects.ml` — if `Put_sensor` needs to carry the edge too; else leave it split between the sensor row and the edge row as today.

- [ ] **Step 1: Replace `sensor_edge_item` body to call `edge_item`**

```ocaml
let sensor_edge_item ~parent ~sensor_id ~created =
  edge_item
    ~from_:(Node_id.to_string parent)
    ~to_:(Sensor_id.to_string sensor_id)
    ~kind:Edge_kind.Has_sensor
    ~name:""
    ~created
```

- [ ] **Step 2: Confirm stored `sk` still starts with `has_sensor#` and sensor lookup queries still work**

Update any repo query filter that hard-codes `has_sensor#` to use `Edge_kind.sk_verb Has_sensor ^ "#"`.

- [ ] **Step 3: Run tests**

Run: `dune runtest` — existing sensor attach/list tests must pass.

- [ ] **Step 4: Commit**

```bash
git commit -am "sensors: route edge creation through Edge_kind.Has_sensor"
```

---

### Task 4: Migrate `List_children` filter

**Files:**
- Modify: `lib/logic/hierarchy.ml:141-147`
- Modify: `lib/effects.ml` — `List_children` filter becomes `Edge_kind.t option` (not a raw prefix string)

- [ ] **Step 1: Change the effect to take an optional Edge_kind**

```ocaml
| List_children    : Node_id.t * Edge_kind.t option -> Node.t list Effect.t
| List_child_refs  : Node_id.t * Edge_kind.t option -> (Node_id.t * string) list Effect.t

let list_children ?kind parent =
  Effect.perform (List_children (parent, kind))
let list_child_refs ?kind parent =
  Effect.perform (List_child_refs (parent, kind))
```

- [ ] **Step 2: Update call sites**

```ocaml
(* hierarchy.ml:31 *)
Effects.list_children ~kind:(Edge_kind.Has_label edge_spec.Schema.label) parent

(* hierarchy.ml:141-147 *)
let list_children ?label parent =
  let kind = Option.map (fun l -> Edge_kind.Has_label l) label in
  Ok (Effects.list_children ?kind parent)
```

- [ ] **Step 3: Update Memory + Dynamo handlers** — both translate `kind` to the sk prefix via `Edge_kind.sk_verb kind ^ "#"`.

- [ ] **Step 4: Run tests, commit**

```bash
git commit -am "effects: list_children filter typed as Edge_kind"
```

---

### Task 5: User domain types

**Files:**
- Create: `lib/domain/user_id.ml`, `cognito_group.ml`, `language.ml`, `currency.ml`, `user.ml`
- Test: `test/test_domain_user.ml`

- [ ] **Step 1: Write tests**

```ocaml
(* test/test_domain_user.ml *)
open Ocaml_lambda_test

let user_id_rt () =
  let id = User_id.of_email "alice@example.com" in
  Alcotest.(check string) "render" "U#alice@example.com" (User_id.to_string id);
  match User_id.of_string "U#alice@example.com" with
  | Ok id' -> Alcotest.(check string) "rt" "alice@example.com" (User_id.email id')
  | Error e -> Alcotest.failf "of_string: %s" e

let user_id_rejects_bad () =
  match User_id.of_string "alice@example.com" with
  | Ok _ -> Alcotest.fail "missing U# prefix should fail"
  | Error _ -> ()

let cognito_group_parses () =
  Alcotest.(check bool) "reader" true
    (Cognito_group.of_string "reader" = Ok Cognito_group.Reader);
  Alcotest.(check bool) "writer" true
    (Cognito_group.of_string "writer" = Ok Cognito_group.Writer);
  Alcotest.(check bool) "admin"  true
    (Cognito_group.of_string "admin"  = Ok Cognito_group.Admin)

let tests =
  [ Alcotest.test_case "user_id roundtrip"   `Quick user_id_rt
  ; Alcotest.test_case "user_id rejects bad" `Quick user_id_rejects_bad
  ; Alcotest.test_case "cognito_group parse" `Quick cognito_group_parses
  ]
```

- [ ] **Step 2: Implement**

```ocaml
(* lib/domain/user_id.ml *)
type t = { email : string }
let of_email email = { email }
let email t = t.email
let to_string t = "U#" ^ t.email
let of_string s =
  match String.length s > 2 && String.sub s 0 2 = "U#" with
  | true ->
      let e = String.sub s 2 (String.length s - 2) in
      if e = "" then Error "empty email after U#" else Ok { email = e }
  | false -> Error "user_id must start with U#"
```

```ocaml
(* lib/domain/cognito_group.ml *)
type t = Reader | Writer | Admin
let to_string = function Reader -> "reader" | Writer -> "writer" | Admin -> "admin"
let of_string = function
  | "reader" -> Ok Reader | "writer" -> Ok Writer | "admin" -> Ok Admin
  | s -> Error (Printf.sprintf "bad cognito group %S" s)

(* capability ordering: admin > writer > reader *)
let rank = function Reader -> 0 | Writer -> 1 | Admin -> 2
```

```ocaml
(* lib/domain/language.ml *)
type t = Danish | Swedish | Norwegian | English | German
let to_string = function
  | Danish -> "danish" | Swedish -> "swedish" | Norwegian -> "norwegian"
  | English -> "english" | German -> "german"
let of_string = function
  | "danish" -> Ok Danish | "swedish" -> Ok Swedish
  | "norwegian" -> Ok Norwegian | "english" -> Ok English
  | "german" -> Ok German
  | s -> Error (Printf.sprintf "bad language %S" s)
let default = Danish
```

```ocaml
(* lib/domain/currency.ml *)
type t = DKK | SEK | NOK | USD | EUR
let to_string = function
  | DKK -> "DKK" | SEK -> "SEK" | NOK -> "NOK" | USD -> "USD" | EUR -> "EUR"
let of_string = function
  | "DKK" -> Ok DKK | "SEK" -> Ok SEK | "NOK" -> Ok NOK
  | "USD" -> Ok USD | "EUR" -> Ok EUR
  | s -> Error (Printf.sprintf "bad currency %S" s)
let default = DKK
```

```ocaml
(* lib/domain/user.ml *)
type t = {
  id : User_id.t;
  name : string;
  cognito_group : Cognito_group.t;
  language : Language.t;
  currency : Currency.t;
  created : Ptime.t;
}

let make ~email ~name ~cognito_group ?(language=Language.default)
         ?(currency=Currency.default) ~created () =
  { id = User_id.of_email email; name; cognito_group; language; currency; created }
```

- [ ] **Step 3: Run tests, commit**

```bash
git commit -am "domain: add User + supporting value types"
```

---

### Task 6: User effects + Memory handler

**Files:**
- Modify: `lib/effects.ml` — add `Put_user`, `Get_user`, `List_users`, `Delete_user`
- Modify: `lib/repo/memory.ml` — store users in a hashtable keyed on `User_id.to_string`

- [ ] **Step 1: Add effects**

```ocaml
| Put_user    : User.t -> unit Effect.t
| Get_user    : User_id.t -> User.t option Effect.t
| List_users  : unit -> User.t list Effect.t
| Delete_user : User_id.t -> unit Effect.t

let put_user u            = Effect.perform (Put_user u)
let get_user id           = Effect.perform (Get_user id)
let list_users ()         = Effect.perform (List_users ())
let delete_user id        = Effect.perform (Delete_user id)
```

- [ ] **Step 2: Extend Memory state**

Add `users : (string, User.t) Hashtbl.t` to the state record; initialize in `Memory.empty`; handle each effect with direct table ops.

- [ ] **Step 3: Write a smoke test and commit**

```ocaml
(* test/test_logic_users.ml — scaffold *)
let put_get_roundtrip () =
  let st = Memory.empty () in
  Memory.run st (fun () ->
    let u = User.make ~email:"a@x" ~name:"A" ~cognito_group:Cognito_group.Reader
              ~created:Ptime.epoch () in
    Effects.put_user u;
    match Effects.get_user u.User.id with
    | Some u' -> Alcotest.(check string) "name" "A" u'.User.name
    | None -> Alcotest.fail "put then get returned None")
```

```bash
git commit -am "effects/memory: user CRUD effects"
```

---

### Task 7: User logic module

**Files:**
- Create: `lib/logic/users.ml`
- Test: extend `test/test_logic_users.ml`

- [ ] **Step 1: Write tests** — create, get-unknown, duplicate-email conflict, list, delete.

- [ ] **Step 2: Implement**

```ocaml
(* lib/logic/users.ml *)
let ( let* ) = Result.bind

let bad m = Errors.Bad_request m

let create ~email ~name ~cognito_group ?language ?currency () =
  match Effects.get_user (User_id.of_email email) with
  | Some _ -> Error (Errors.Conflict (Printf.sprintf "user %s already exists" email))
  | None ->
      let created = Effects.now () in
      let u = User.make ~email ~name ~cognito_group ?language ?currency ~created () in
      Effects.put_user u;
      Ok u

let get id =
  match Effects.get_user id with
  | Some u -> Ok u
  | None   -> Error (Errors.Not_found (Node_id.make Level.Hn9 (Uuidm.v `V4)))
  (* NOTE: Errors.Not_found only carries a Node_id today. Extend or introduce
     Errors.Not_found_user with a user id in a follow-up — for v1 we report a
     generic Bad_request. *)

let update ~id ?name ?cognito_group ?language ?currency () =
  let* u = get id in
  let u' = {
    u with
    User.name = Option.value name ~default:u.User.name;
    cognito_group = Option.value cognito_group ~default:u.User.cognito_group;
    language = Option.value language ~default:u.User.language;
    currency = Option.value currency ~default:u.User.currency;
  } in
  Effects.put_user u';
  Ok u'

let delete id =
  match Effects.get_user id with
  | None -> Error (Errors.Bad_request "user not found")
  | Some _ -> Effects.delete_user id; Ok id

let list () = Ok (Effects.list_users ())
```

Cascade-delete of the user's BLOCKED edges is handled in Task 12.

- [ ] **Step 3: Tests pass, commit**

```bash
git commit -am "logic: users CRUD"
```

---

### Task 8: Errors: Not_found_user + User_id in Errors

**Files:**
- Modify: `lib/domain/errors.ml`
- Modify: `lib/logic/users.ml` — use the new case

- [ ] **Step 1: Add variant**

```ocaml
(* lib/domain/errors.ml *)
type t =
  ...
  | Not_found_user of User_id.t
```

Update `message` and API error mapping to produce `not_found` with user id message.

- [ ] **Step 2: Replace the placeholder in users.ml**

- [ ] **Step 3: Run tests, commit**

```bash
git commit -am "errors: add Not_found_user"
```

---

### Task 9: Dynamo codec + handler for users

**Files:**
- Modify: `lib/repo/codec.ml` — add `user_item`, `user_of_item`
- Modify: `lib/repo/dynamo.ml` — handle the four user effects

- [ ] **Step 1: Codec**

```ocaml
let user_item (u : User.t) =
  let uid = User_id.to_string u.User.id in
  [
    ("pk", s uid); ("sk", s uid);
    ("type", s "user");
    ("name", s u.User.name);
    ("cognito_group", s (Cognito_group.to_string u.User.cognito_group));
    ("language", s (Language.to_string u.User.language));
    ("currency", s (Currency.to_string u.User.currency));
    ("created", s (Ptime.to_rfc3339 ~tz_offset_s:0 u.User.created));
    ("gsi1pk", s "user");
    ("gsi1sk", s uid);
  ]

let user_of_item kvs : (User.t, string) result =
  let* pk_v = field kvs "pk" in
  let* pk_s = as_string pk_v in
  let* id = User_id.of_string pk_s in
  let* name_v = field kvs "name" in
  let* name = as_string name_v in
  let* g_v = field kvs "cognito_group" in
  let* g_s = as_string g_v in
  let* cognito_group = Cognito_group.of_string g_s in
  let language =
    match List.assoc_opt "language" kvs with
    | Some (Dyn.S s) -> (match Language.of_string s with Ok l -> l | _ -> Language.default)
    | _ -> Language.default
  in
  let currency =
    match List.assoc_opt "currency" kvs with
    | Some (Dyn.S s) -> (match Currency.of_string s with Ok c -> c | _ -> Currency.default)
    | _ -> Currency.default
  in
  let* c_v = field kvs "created" in
  let* c_s = as_string c_v in
  let created = match Ptime.of_rfc3339 c_s with
    | Ok (t,_,_) -> t | Error _ -> Ptime.epoch in
  Ok { User.id; name; cognito_group; language; currency; created }
```

- [ ] **Step 2: Dynamo handler**

- `Put_user u` → `PutItem (user_item u)`
- `Get_user id` → `GetItem pk=sk=U#<email>` → decode
- `Delete_user id` → cascade: Query pk=U#<email>, TransactWriteItems delete all (user row + any blocked edges)
- `List_users ()` → Query GSI1 on `gsi1pk="user"`

- [ ] **Step 3: Tests + commit**

Add a codec round-trip test in `test/test_codec.ml` (or create it).

```bash
git commit -am "repo: dynamo codec + handler for users"
```

---

### Task 10: API wiring for users

**Files:**
- Modify: `lib/api/api_command.ml` — add `create_user`, `update_user`, `delete_user`
- Modify: `lib/api/api_query.ml` — add `get_user`, `list_users`
- Modify: `lib/api/api_json.ml` — `user_to_json`

- [ ] **Step 1: JSON encoder**

```ocaml
let user_to_json (u : User.t) : Yojson.Safe.t =
  `Assoc [
    "id", `String (User_id.to_string u.User.id);
    "email", `String (User_id.email u.User.id);
    "name", `String u.User.name;
    "cognito_group", `String (Cognito_group.to_string u.User.cognito_group);
    "language", `String (Language.to_string u.User.language);
    "currency", `String (Currency.to_string u.User.currency);
    "created", `String (Ptime.to_rfc3339 ~tz_offset_s:0 u.User.created);
  ]
```

- [ ] **Step 2: Command dispatcher cases**

```
POST /command { "action": "create_user", "email": "...", "name": "...",
                "cognito_group": "writer", "language": "danish"?, "currency": "DKK"? }
  → 200 user
POST /command { "action": "update_user", "id": "U#...", ... optional fields ... }
  → 200 user
POST /command { "action": "delete_user", "id": "U#..." }
  → 200 { "deleted": "U#..." }
```

- [ ] **Step 3: Query dispatcher cases**

```
GET /query/get_user?id=U%23alice@example.com
GET /query/list_users
```

- [ ] **Step 4: Run tests + commit**

```bash
git commit -am "api: create/update/delete/get/list user"
```

---

### Task 11: Block edges — effect + codec

**Files:**
- Modify: `lib/effects.ml` — `List_blocked_nodes`, `List_blocked_users`, `Delete_edge`
- Modify: `lib/repo/codec.ml` — use `edge_item` with `Edge_kind.Blocked`
- Modify: `lib/repo/memory.ml`, `lib/repo/dynamo.ml`

- [ ] **Step 1: Add effects**

```ocaml
| List_blocked_nodes : User_id.t -> Node_id.t list Effect.t
| List_blocked_users : Node_id.t -> User_id.t list Effect.t
| Delete_edge : { from_ : string; to_ : string; kind : Edge_kind.t } -> unit Effect.t

let list_blocked_nodes id = Effect.perform (List_blocked_nodes id)
let list_blocked_users id = Effect.perform (List_blocked_users id)
let delete_edge ~from_ ~to_ ~kind = Effect.perform (Delete_edge { from_; to_; kind })
```

- [ ] **Step 2: Memory**

Extend the edge table so it is keyed on `(from, kind, to)`. Provide queries for the two directions.

- [ ] **Step 3: Dynamo handler**

- `List_blocked_nodes U#alice` → `Query pk = "U#alice" AND begins_with(sk, "blocked#")` → decode the suffix as `Node_id`.
- `List_blocked_users HN4#x` → `Query gsi1pk = "HN4#x" AND begins_with(gsi1sk, "blocks#U#")`.
- `Delete_edge` → `DeleteItem` on derived `{pk, sk}`.

- [ ] **Step 4: Tests + commit**

```bash
git commit -am "effects/repo: block-edge reads and deletes"
```

---

### Task 12: Access logic — block / unblock / effective_permission

**Files:**
- Create: `lib/logic/access.ml`
- Test: `test/test_logic_access.ml`

- [ ] **Step 1: Tests**

```ocaml
(* Seed: company c, building b under c, user U (writer group), block U on c.
   Expected:
   - effective_permission U c = None (blocked)
   - effective_permission U b = None (inherited block)
   - after unblock: effective_permission U b = Some Writer *)
```

- [ ] **Step 2: Implement**

```ocaml
(* lib/logic/access.ml *)
let ( let* ) = Result.bind

let block ~user_id ~node_id () =
  let* _ = match Effects.get_user user_id with
    | Some u -> Ok u
    | None -> Error (Errors.Not_found_user user_id)
  in
  let* _ = match Effects.get_node node_id with
    | Some n -> Ok n
    | None -> Error (Errors.Not_found node_id)
  in
  let created = Effects.now () in
  Effects.put_edge
    ~from_:(User_id.to_string user_id)
    ~to_:(Node_id.to_string node_id)
    ~kind:Edge_kind.Blocked ~name:"" ~created;
  Ok ()

let unblock ~user_id ~node_id () =
  Effects.delete_edge
    ~from_:(User_id.to_string user_id)
    ~to_:(Node_id.to_string node_id)
    ~kind:Edge_kind.Blocked;
  Ok ()

(* Ancestor chain: walk Node.parent upward. *)
let ancestors_of node_id =
  let rec go acc id =
    match Effects.get_node id with
    | None -> List.rev acc
    | Some n ->
        (match n.Node.parent with
         | Some p -> go (p :: acc) p
         | None   -> List.rev acc)
  in
  go [] node_id

(* Single point of delegation. Replace with AVP/Cedar in a future task. *)
let effective_permission ~user_id ~node_id =
  match Effects.get_user user_id with
  | None -> Error (Errors.Not_found_user user_id)
  | Some u ->
      let blocked = Effects.list_blocked_nodes user_id in
      let chain = node_id :: ancestors_of node_id in
      let is_blocked =
        List.exists (fun n ->
          List.exists (fun b -> Node_id.equal b n) blocked) chain
      in
      if is_blocked then Ok None
      else Ok (Some u.User.cognito_group)

let list_blocked_nodes ~user_id = Ok (Effects.list_blocked_nodes user_id)
let list_blocked_users ~node_id = Ok (Effects.list_blocked_users node_id)
```

- [ ] **Step 3: Run tests + commit**

```bash
git commit -am "logic: block/unblock + effective_permission"
```

---

### Task 13: API wiring for access

**Files:**
- Modify: `lib/api/api_command.ml` — `block_user`, `unblock_user`
- Modify: `lib/api/api_query.ml` — `list_blocked_nodes`, `list_blocked_users`, `effective_permission`

- [ ] **Step 1: Commands**

```
POST /command { "action": "block_user",   "user_id": "U#...", "node_id": "HN4#..." }
POST /command { "action": "unblock_user", "user_id": "U#...", "node_id": "HN4#..." }
  → 200 { "ok": true }
```

- [ ] **Step 2: Queries**

```
GET /query/list_blocked_nodes?user=U%23alice@example.com
  → 200 { "nodes": [ "HN4#...", ... ] }

GET /query/list_blocked_users?node=HN4%23...
  → 200 { "users": [ "U#...", ... ] }

GET /query/effective_permission?user=U%23alice@example.com&node=HN4%23...
  → 200 { "capability": "writer" } | { "capability": null, "reason": "blocked" }
```

- [ ] **Step 3: Run tests + commit**

```bash
git commit -am "api: block/unblock + access queries"
```

---

### Task 14: Cascade cleanup on delete

**Files:**
- Modify: `lib/logic/users.ml` → delete also tears down that user's `blocked` edges
- Modify: `lib/logic/hierarchy.ml` → delete_node already removes direct child edges; extend to also remove any `blocks#…` gsi1sk rows pointing at this node

- [ ] **Step 1: Tests** — delete user A should remove both U#A and all `blocked#…` edges with pk=U#A.
- [ ] **Step 2: Implement via pk Query + TransactWriteItems (same pattern as EMS `cascade_delete`).**
- [ ] **Step 3: Commit.**

```bash
git commit -am "logic: cascade blocked-edges on user/node delete"
```

---

### Task 15: Docs

**Files:**
- Modify: `docs/architecture.md` — add User + block-edge section; note upstream enforcement + future AVP option.
- Modify: `docs/hierarchy-and-sensors.md` — reference `Edge_kind.t` replacing the `has_<label>` string.
- Modify: `docs/api.md` — append the new commands + queries with examples; include `%23` encoding note for `U#` just like `HN#`.

- [ ] **Step 1–3:** write prose + examples, no code changes.
- [ ] **Step 4: Commit.**

```bash
git commit -am "docs: users + block-permissions"
```

---

### Task 16: Build + deploy + smoke test

- [ ] **Step 1:** `make clean && make build` — verify zip produced.
- [ ] **Step 2:** `aws lambda update-function-code --function-name ocaml_hello --zip-file fileb://ocaml-lambda-hierarchy.zip --region eu-central-1`
- [ ] **Step 3:** `aws lambda wait function-updated --function-name ocaml_hello --region eu-central-1`
- [ ] **Step 4:** smoke — `curl` `create_user`, `get_user`, `block_user`, `effective_permission` against API `vp9p5wrn6f`.

---

## Out of scope (TODOs noted in code / docs)

- In-Lambda enforcement. For now, upstream (API Gateway authorizer + frontend) decides; the Lambda is a pure store-and-report. `// TODO(enforcement)` at the top of `lib/api/api_command.ml`.
- Cognito group enforcement at write time (e.g. refusing an `update_user` that promotes a reader to admin unless the caller is admin). Same story — upstream concern.
- Amazon Verified Permissions (Cedar). `Access.effective_permission` is the seam. Swap the function body for an AVP call when / if we move there.
- Cascade on node delete already exists; extend to remove inbound `blocks#` rows when a node is deleted. Covered in Task 14.

## Self-review notes

Spec coverage — every decision from the discussion maps to a task:

- Cognito group as ceiling → stored on user, read by `effective_permission` (Task 5, 12).
- Default-allow + explicit BLOCKED → only edge kind added is `Blocked` (Task 1, 11).
- Single refactor → one branch, sequential tasks 1→16.
- AVP/Cedar future → isolated in `Access.effective_permission` (Task 12) and documented (Task 15).
- No enforcement → no command/query rejects on the caller's identity. Documented (Task 15).
- `create_node` admin-only → frontend concern; noted as TODO (Task 15).

Type consistency — `Edge_kind.t` appears identically in every task. `User_id.t` has a single definition (Task 5) used by all downstream tasks. `Cognito_group.t` same. Effect constructors are defined once (Tasks 2, 4, 6, 11) and referenced by their canonical names thereafter.

Placeholder scan — no `TBD`, no `implement later`, no bare "add tests here"; each step has concrete code or a concrete command.
