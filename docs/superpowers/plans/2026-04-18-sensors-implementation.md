# Sensor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend the hierarchy table to support sensors as first-class entities with stable logical identity, versioned physical-device assignment history, and pure formula-based value computation, per `docs/superpowers/specs/2026-04-17-hierarchy-and-sensors-design.md` §6.

**Architecture:** Sensors live in their own DynamoDB partition `S#<uuid>`. The currently-active physical device is represented by a row whose sk is `active#<iso8601>`; history rows carry the bare timestamp (no `active#` prefix) and record when that device *was* active. Attachment is modeled as a `has_sensor#S#<uuid>` edge in the parent node's partition. Formulas form a pure AST (`Identity | Expr`); `Self` is the sensor's own raw reading while `Ref "alias"` resolves to another sensor's *computed* value (S'), giving the layer its absolute-value semantics via explicit `Abs`. Device replacement is an atomic `TransactWriteItems` of (delete old active, put old-as-history, put new active).

**Tech Stack:** OCaml 5 effects, Alcotest, smaws (DynamoDB), Ptime, Uuidm, Yojson.

---

## Phase 1 — Pure domain types and formula evaluator

### Task 1: Formula AST types

**Files:**
- Create: `lib/domain/formula.ml`
- Create: `test/test_domain_formula.ml`
- Modify: `test/test_ocaml_lambda_test.ml`

- [ ] **Step 1: Write the failing test**

Create `test/test_domain_formula.ml`:

```ocaml
open Ocaml_lambda_test

let uuid_a = Uuidm.of_string "00000000-0000-4000-8000-000000000001" |> Option.get
let uuid_b = Uuidm.of_string "00000000-0000-4000-8000-000000000002" |> Option.get

let identity_constructs () =
  let f = Formula.Identity in
  Alcotest.(check bool) "is identity"
    true
    (match f with Formula.Identity -> true | _ -> false)

let expr_has_refs () =
  let ast = Formula.Abs (Formula.Sub (Formula.Self, Formula.Ref "S1")) in
  let f = Formula.Expr { ast; refs = [ ("S1", uuid_a) ] } in
  match f with
  | Formula.Expr { refs; _ } ->
      Alcotest.(check int) "one ref" 1 (List.length refs);
      let alias, u = List.hd refs in
      Alcotest.(check string) "alias" "S1" alias;
      Alcotest.(check bool) "uuid equal" true (Uuidm.equal u uuid_a)
  | _ -> Alcotest.fail "expected Expr"

let deeply_nested_expr () =
  let ast =
    Formula.Abs
      (Formula.Sub
         (Formula.Sub (Formula.Self, Formula.Ref "S4"),
          Formula.Ref "S5"))
  in
  let _ = Formula.Expr { ast; refs = [ ("S4", uuid_a); ("S5", uuid_b) ] } in
  Alcotest.(check pass) "compiles" () ()

let tests =
  [
    Alcotest.test_case "Identity constructs"    `Quick identity_constructs;
    Alcotest.test_case "Expr carries refs"      `Quick expr_has_refs;
    Alcotest.test_case "deeply nested Abs/Sub"  `Quick deeply_nested_expr;
  ]
```

Register in `test/test_ocaml_lambda_test.ml` by adding `("domain.formula", Test_domain_formula.tests);` alongside the other domain entries.

- [ ] **Step 2: Run test to verify it fails**

Run: `dune test`
Expected: build failure — `Unbound module Formula`.

- [ ] **Step 3: Write minimal implementation**

Create `lib/domain/formula.ml`:

```ocaml
type expr =
  | Num of float
  | Self
  | Ref of string
  | Abs of expr
  | Add of expr * expr
  | Sub of expr * expr
  | Mul of expr * expr
  | Div of expr * expr

type t =
  | Identity
  | Expr of { ast : expr; refs : (string * Uuidm.t) list }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `dune test`
Expected: PASS — all three Formula tests green.

- [ ] **Step 5: Commit**

```bash
git add lib/domain/formula.ml test/test_domain_formula.ml test/test_ocaml_lambda_test.ml
git commit -m "feat(domain): add Formula AST types"
```

---

### Task 2: Formula evaluator

**Files:**
- Modify: `lib/domain/formula.ml`
- Modify: `test/test_domain_formula.ml`

- [ ] **Step 1: Write the failing test**

Append to `test/test_domain_formula.ml`:

```ocaml
let eval_identity () =
  let v =
    Formula.eval ~self:7.5 ~resolve:(fun _ -> failwith "should not call")
      Formula.Identity
  in
  Alcotest.(check (float 1e-9)) "identity returns self" 7.5 v

let eval_arithmetic () =
  let ast =
    Formula.Sub (Formula.Self, Formula.Add (Formula.Ref "S1", Formula.Ref "S2"))
  in
  let f = Formula.Expr { ast; refs = [ ("S1", uuid_a); ("S2", uuid_b) ] } in
  let resolve alias =
    match alias with
    | "S1" -> 3.0
    | "S2" -> 1.0
    | _ -> failwith "unknown alias"
  in
  let v = Formula.eval ~self:10.0 ~resolve f in
  Alcotest.(check (float 1e-9)) "10 - (3 + 1) = 6" 6.0 v

let eval_abs_flips_negative () =
  let ast = Formula.Abs (Formula.Sub (Formula.Self, Formula.Ref "S1")) in
  let f = Formula.Expr { ast; refs = [ ("S1", uuid_a) ] } in
  let resolve _ = 12.0 in
  let v = Formula.eval ~self:5.0 ~resolve f in
  Alcotest.(check (float 1e-9)) "|5 - 12| = 7" 7.0 v

let eval_multiplier () =
  let ast = Formula.Mul (Formula.Self, Formula.Num 2.5) in
  let f = Formula.Expr { ast; refs = [] } in
  let v = Formula.eval ~self:4.0 ~resolve:(fun _ -> 0.0) f in
  Alcotest.(check (float 1e-9)) "4 * 2.5 = 10" 10.0 v

let eval_div_by_zero_is_infinity () =
  let ast = Formula.Div (Formula.Self, Formula.Num 0.0) in
  let f = Formula.Expr { ast; refs = [] } in
  let v = Formula.eval ~self:1.0 ~resolve:(fun _ -> 0.0) f in
  Alcotest.(check bool) "infinite" true (Float.is_infinite v)

let eval_unknown_ref_raises () =
  let ast = Formula.Ref "missing" in
  let f = Formula.Expr { ast; refs = [] } in
  (try
     let _ = Formula.eval ~self:0.0 ~resolve:(fun _ -> 0.0) f in
     Alcotest.fail "expected exception"
   with Formula.Unknown_ref "missing" -> ())
```

Extend the `tests` list with each new case:

```ocaml
    Alcotest.test_case "eval Identity = self"        `Quick eval_identity;
    Alcotest.test_case "eval arithmetic"             `Quick eval_arithmetic;
    Alcotest.test_case "eval Abs flips negative"     `Quick eval_abs_flips_negative;
    Alcotest.test_case "eval multiplier"             `Quick eval_multiplier;
    Alcotest.test_case "eval div by zero = infinity" `Quick eval_div_by_zero_is_infinity;
    Alcotest.test_case "eval unknown ref raises"     `Quick eval_unknown_ref_raises;
```

- [ ] **Step 2: Run test to verify it fails**

Run: `dune test`
Expected: build failure — `Unbound value eval` / `Unbound constructor Unknown_ref`.

- [ ] **Step 3: Write minimal implementation**

Append to `lib/domain/formula.ml`:

```ocaml
exception Unknown_ref of string

let rec eval_expr ~self ~resolve = function
  | Num n          -> n
  | Self           -> self
  | Ref alias      -> resolve alias
  | Abs e          -> Float.abs (eval_expr ~self ~resolve e)
  | Add (a, b)     -> eval_expr ~self ~resolve a +. eval_expr ~self ~resolve b
  | Sub (a, b)     -> eval_expr ~self ~resolve a -. eval_expr ~self ~resolve b
  | Mul (a, b)     -> eval_expr ~self ~resolve a *. eval_expr ~self ~resolve b
  | Div (a, b)     -> eval_expr ~self ~resolve a /. eval_expr ~self ~resolve b

let eval ~self ~resolve = function
  | Identity -> self
  | Expr { ast; refs } ->
      let lookup alias =
        match List.assoc_opt alias refs with
        | Some _ -> resolve alias
        | None -> raise (Unknown_ref alias)
      in
      eval_expr ~self ~resolve:lookup ast
```

- [ ] **Step 4: Run test to verify it passes**

Run: `dune test`
Expected: PASS — all six new eval cases green.

- [ ] **Step 5: Commit**

```bash
git add lib/domain/formula.ml test/test_domain_formula.ml
git commit -m "feat(domain): pure Formula evaluator with Abs and Self/Ref split"
```

---

### Task 3: Sensor_id (`S#<uuid>` parsing)

**Files:**
- Create: `lib/domain/sensor_id.ml`
- Create: `test/test_domain_sensor_id.ml`
- Modify: `test/test_ocaml_lambda_test.ml`

- [ ] **Step 1: Write the failing test**

Create `test/test_domain_sensor_id.ml`:

```ocaml
open Ocaml_lambda_test

let sample = Uuidm.of_string "11111111-2222-4333-8444-555555555555" |> Option.get

let round_trip () =
  let id = Sensor_id.make sample in
  let s = Sensor_id.to_string id in
  Alcotest.(check string) "prefixed" "S#11111111-2222-4333-8444-555555555555" s;
  match Sensor_id.of_string s with
  | Ok id2 ->
      Alcotest.(check bool) "equal" true (Sensor_id.equal id id2);
      Alcotest.(check bool) "uuid preserved" true
        (Uuidm.equal (Sensor_id.uuid id2) sample)
  | Error e -> Alcotest.failf "parse failed: %s" e

let rejects_missing_prefix () =
  match Sensor_id.of_string "11111111-2222-4333-8444-555555555555" with
  | Ok _ -> Alcotest.fail "should reject missing S#"
  | Error _ -> ()

let rejects_wrong_prefix () =
  match Sensor_id.of_string "HN4#11111111-2222-4333-8444-555555555555" with
  | Ok _ -> Alcotest.fail "should reject HN4# prefix"
  | Error _ -> ()

let rejects_bad_uuid () =
  match Sensor_id.of_string "S#not-a-uuid" with
  | Ok _ -> Alcotest.fail "should reject bad uuid"
  | Error _ -> ()

let tests =
  [
    Alcotest.test_case "round trip"            `Quick round_trip;
    Alcotest.test_case "rejects missing prefix" `Quick rejects_missing_prefix;
    Alcotest.test_case "rejects wrong prefix"   `Quick rejects_wrong_prefix;
    Alcotest.test_case "rejects bad uuid"       `Quick rejects_bad_uuid;
  ]
```

Register `("domain.sensor_id", Test_domain_sensor_id.tests);` in `test/test_ocaml_lambda_test.ml`.

- [ ] **Step 2: Run test to verify it fails**

Run: `dune test`
Expected: build failure — `Unbound module Sensor_id`.

- [ ] **Step 3: Write minimal implementation**

Create `lib/domain/sensor_id.ml`:

```ocaml
type t = Uuidm.t

let make u = u
let uuid t = t
let equal = Uuidm.equal

let to_string t = "S#" ^ Uuidm.to_string t

let of_string s =
  if String.length s < 3 || String.sub s 0 2 <> "S#" then
    Error (Printf.sprintf "missing S# prefix in %S" s)
  else
    let rest = String.sub s 2 (String.length s - 2) in
    match Uuidm.of_string rest with
    | Some u -> Ok u
    | None -> Error (Printf.sprintf "bad uuid in %S" s)
```

- [ ] **Step 4: Run test to verify it passes**

Run: `dune test`
Expected: PASS — all four Sensor_id tests green.

- [ ] **Step 5: Commit**

```bash
git add lib/domain/sensor_id.ml test/test_domain_sensor_id.ml test/test_ocaml_lambda_test.ml
git commit -m "feat(domain): Sensor_id with S#<uuid> encoding"
```

---

### Task 4: Sensor sort keys (`active#<ts>` / `<ts>`)

**Files:**
- Create: `lib/domain/sensor_sk.ml`
- Create: `test/test_domain_sensor_sk.ml`
- Modify: `test/test_ocaml_lambda_test.ml`

- [ ] **Step 1: Write the failing test**

Create `test/test_domain_sensor_sk.ml`:

```ocaml
open Ocaml_lambda_test

let sample_time () =
  Ptime.of_rfc3339 "2026-04-18T10:00:00Z" |> Result.get_ok |> fun (t, _, _) -> t

let active_encodes_with_prefix () =
  let t = sample_time () in
  Alcotest.(check string) "active format"
    "active#2026-04-18T10:00:00Z"
    (Sensor_sk.to_string (Sensor_sk.Active t))

let history_encodes_without_prefix () =
  let t = sample_time () in
  Alcotest.(check string) "history format"
    "2026-04-18T10:00:00Z"
    (Sensor_sk.to_string (Sensor_sk.History t))

let parse_active () =
  match Sensor_sk.of_string "active#2026-04-18T10:00:00Z" with
  | Ok (Sensor_sk.Active t) ->
      Alcotest.(check string) "ts" "2026-04-18T10:00:00Z"
        (Ptime.to_rfc3339 ~tz_offset_s:0 t)
  | _ -> Alcotest.fail "expected Active"

let parse_history () =
  match Sensor_sk.of_string "2026-03-01T00:00:00Z" with
  | Ok (Sensor_sk.History t) ->
      Alcotest.(check string) "ts" "2026-03-01T00:00:00Z"
        (Ptime.to_rfc3339 ~tz_offset_s:0 t)
  | _ -> Alcotest.fail "expected History"

let rejects_garbage () =
  match Sensor_sk.of_string "garbage" with
  | Ok _ -> Alcotest.fail "should reject"
  | Error _ -> ()

let tests =
  [
    Alcotest.test_case "active format"   `Quick active_encodes_with_prefix;
    Alcotest.test_case "history format"  `Quick history_encodes_without_prefix;
    Alcotest.test_case "parse active"    `Quick parse_active;
    Alcotest.test_case "parse history"   `Quick parse_history;
    Alcotest.test_case "rejects garbage" `Quick rejects_garbage;
  ]
```

Register `("domain.sensor_sk", Test_domain_sensor_sk.tests);` in `test/test_ocaml_lambda_test.ml`.

- [ ] **Step 2: Run test to verify it fails**

Run: `dune test`
Expected: build failure — `Unbound module Sensor_sk`.

- [ ] **Step 3: Write minimal implementation**

Create `lib/domain/sensor_sk.ml`:

```ocaml
type t =
  | Active of Ptime.t
  | History of Ptime.t

let active_prefix = "active#"

let ptime_to_s t = Ptime.to_rfc3339 ~tz_offset_s:0 t

let to_string = function
  | Active t  -> active_prefix ^ ptime_to_s t
  | History t -> ptime_to_s t

let parse_ts s =
  match Ptime.of_rfc3339 s with
  | Ok (t, _, _) -> Ok t
  | Error _ -> Error (Printf.sprintf "bad timestamp %S" s)

let of_string s =
  let len = String.length s in
  let plen = String.length active_prefix in
  if len > plen && String.sub s 0 plen = active_prefix then
    let rest = String.sub s plen (len - plen) in
    match parse_ts rest with
    | Ok t -> Ok (Active t)
    | Error e -> Error e
  else
    match parse_ts s with
    | Ok t -> Ok (History t)
    | Error e -> Error e
```

- [ ] **Step 4: Run test to verify it passes**

Run: `dune test`
Expected: PASS — all five Sensor_sk tests green.

- [ ] **Step 5: Commit**

```bash
git add lib/domain/sensor_sk.ml test/test_domain_sensor_sk.ml test/test_ocaml_lambda_test.ml
git commit -m "feat(domain): Sensor_sk active/history encoding"
```

---

### Task 5: Sensor record

**Files:**
- Create: `lib/domain/sensor.ml`
- Create: `test/test_domain_sensor.ml`
- Modify: `test/test_ocaml_lambda_test.ml`

- [ ] **Step 1: Write the failing test**

Create `test/test_domain_sensor.ml`:

```ocaml
open Ocaml_lambda_test

let uuid_of s = Uuidm.of_string s |> Option.get
let ptime_of s = Ptime.of_rfc3339 s |> Result.get_ok |> fun (t, _, _) -> t

let make_sample () : Sensor.t =
  {
    id = Sensor_id.make (uuid_of "11111111-2222-4333-8444-000000000001");
    active_from = ptime_of "2026-04-18T10:00:00Z";
    parent =
      Node_id.make Level.Hn5
        (uuid_of "22222222-2222-4333-8444-000000000002");
    daq_address = "daq:adeunis_pu_v1:123:0018b210000191c7:counter_a";
    hierarchy_path = "P1#C1#PR1#B2#A1";
    purpose = "Electricity";
    meter_type = Sensor.Counter;
    unit = Some "kWh";
    formula = Formula.Identity;
  }

let fields_preserved () =
  let s = make_sample () in
  Alcotest.(check string) "purpose"     "Electricity" s.purpose;
  Alcotest.(check string) "daq_address"
    "daq:adeunis_pu_v1:123:0018b210000191c7:counter_a" s.daq_address;
  Alcotest.(check bool) "counter meter" true
    (match s.meter_type with Sensor.Counter -> true | _ -> false);
  Alcotest.(check (option string)) "unit" (Some "kWh") s.unit

let meter_type_to_string () =
  Alcotest.(check string) "counter" "counter"
    (Sensor.meter_type_to_string Sensor.Counter);
  Alcotest.(check string) "gauge"   "gauge"
    (Sensor.meter_type_to_string Sensor.Gauge)

let meter_type_of_string () =
  Alcotest.(check bool) "counter parse" true
    (Sensor.meter_type_of_string "counter" = Ok Sensor.Counter);
  Alcotest.(check bool) "gauge parse"   true
    (Sensor.meter_type_of_string "gauge"   = Ok Sensor.Gauge);
  match Sensor.meter_type_of_string "wat" with
  | Ok _ -> Alcotest.fail "should reject unknown meter type"
  | Error _ -> ()

let tests =
  [
    Alcotest.test_case "fields preserved"      `Quick fields_preserved;
    Alcotest.test_case "meter_type_to_string"  `Quick meter_type_to_string;
    Alcotest.test_case "meter_type_of_string"  `Quick meter_type_of_string;
  ]
```

Register `("domain.sensor", Test_domain_sensor.tests);` in `test/test_ocaml_lambda_test.ml`.

- [ ] **Step 2: Run test to verify it fails**

Run: `dune test`
Expected: build failure — `Unbound module Sensor`.

- [ ] **Step 3: Write minimal implementation**

Create `lib/domain/sensor.ml`:

```ocaml
type meter_type = Counter | Gauge

type t = {
  id : Sensor_id.t;
  active_from : Ptime.t;
  parent : Node_id.t;
  daq_address : string;
  hierarchy_path : string;
  purpose : string;
  meter_type : meter_type;
  unit : string option;
  formula : Formula.t;
}

let meter_type_to_string = function
  | Counter -> "counter"
  | Gauge   -> "gauge"

let meter_type_of_string = function
  | "counter" -> Ok Counter
  | "gauge"   -> Ok Gauge
  | other     -> Error (Printf.sprintf "unknown meter type %S" other)
```

- [ ] **Step 4: Run test to verify it passes**

Run: `dune test`
Expected: PASS — three Sensor tests green.

- [ ] **Step 5: Commit**

```bash
git add lib/domain/sensor.ml test/test_domain_sensor.ml test/test_ocaml_lambda_test.ml
git commit -m "feat(domain): Sensor record and meter_type variants"
```

---

### Task 6: Sensor_slot type

**Files:**
- Create: `lib/domain/sensor_slot.ml`
- Create: `test/test_domain_sensor_slot.ml`
- Modify: `test/test_ocaml_lambda_test.ml`

- [ ] **Step 1: Write the failing test**

Create `test/test_domain_sensor_slot.ml`:

```ocaml
open Ocaml_lambda_test

let validates_ok () =
  let s : Sensor_slot.t =
    { kind = "electricity"; min = None; max = Some 2;
      meter_type = Sensor_slot.Either; purposes = Some [ "Electricity" ] }
  in
  match Sensor_slot.validate s with
  | Ok () -> ()
  | Error e -> Alcotest.failf "unexpected: %s" e

let rejects_empty_kind () =
  let s : Sensor_slot.t =
    { kind = ""; min = None; max = None;
      meter_type = Sensor_slot.Counter; purposes = None }
  in
  match Sensor_slot.validate s with
  | Ok () -> Alcotest.fail "expected error on empty kind"
  | Error _ -> ()

let rejects_min_greater_than_max () =
  let s : Sensor_slot.t =
    { kind = "heat"; min = Some 3; max = Some 1;
      meter_type = Sensor_slot.Gauge; purposes = None }
  in
  match Sensor_slot.validate s with
  | Ok () -> Alcotest.fail "expected error on min>max"
  | Error _ -> ()

let allows_purpose_check () =
  let s : Sensor_slot.t =
    { kind = "heat"; min = None; max = None;
      meter_type = Sensor_slot.Either; purposes = Some [ "Heat"; "Electricity" ] }
  in
  Alcotest.(check bool) "Heat ok"        true  (Sensor_slot.allows_purpose s "Heat");
  Alcotest.(check bool) "Water rejected" false (Sensor_slot.allows_purpose s "Water")

let allows_purpose_unrestricted () =
  let s : Sensor_slot.t =
    { kind = "anything"; min = None; max = None;
      meter_type = Sensor_slot.Either; purposes = None }
  in
  Alcotest.(check bool) "any purpose ok" true (Sensor_slot.allows_purpose s "Anything")

let tests =
  [
    Alcotest.test_case "validates ok"            `Quick validates_ok;
    Alcotest.test_case "rejects empty kind"      `Quick rejects_empty_kind;
    Alcotest.test_case "rejects min>max"         `Quick rejects_min_greater_than_max;
    Alcotest.test_case "purpose whitelist"       `Quick allows_purpose_check;
    Alcotest.test_case "purpose unrestricted"    `Quick allows_purpose_unrestricted;
  ]
```

Register `("domain.sensor_slot", Test_domain_sensor_slot.tests);` in `test/test_ocaml_lambda_test.ml`.

- [ ] **Step 2: Run test to verify it fails**

Run: `dune test`
Expected: build failure — `Unbound module Sensor_slot`.

- [ ] **Step 3: Write minimal implementation**

Create `lib/domain/sensor_slot.ml`:

```ocaml
type meter_kind = Counter | Gauge | Either

type t = {
  kind : string;
  min : int option;
  max : int option;
  meter_type : meter_kind;
  purposes : string list option;
}

let validate t =
  if t.kind = "" then Error "sensor slot kind must be non-empty"
  else
    match t.min, t.max with
    | Some a, Some b when a > b ->
        Error (Printf.sprintf "sensor slot %S has min > max" t.kind)
    | _ -> Ok ()

let allows_meter_type slot = function
  | Sensor.Counter -> (match slot.meter_type with Counter | Either -> true | _ -> false)
  | Sensor.Gauge   -> (match slot.meter_type with Gauge   | Either -> true | _ -> false)

let allows_purpose slot purpose =
  match slot.purposes with
  | None -> true
  | Some xs -> List.mem purpose xs
```

- [ ] **Step 4: Run test to verify it passes**

Run: `dune test`
Expected: PASS — five Sensor_slot tests green.

- [ ] **Step 5: Commit**

```bash
git add lib/domain/sensor_slot.ml test/test_domain_sensor_slot.ml test/test_ocaml_lambda_test.ml
git commit -m "feat(domain): Sensor_slot with kind, cardinality, purpose whitelist"
```

---

### Task 7: Schema `sensors` field and validation

**Files:**
- Modify: `lib/domain/schema.ml`
- Modify: `test/test_domain_schema.ml`
- Modify: `itest/test_dynamo.ml`
- Modify: `test/test_logic_hierarchy.ml`
- Modify: `test/test_logic_properties.ml`
- Modify: `test/test_logic_schema_check.ml`
- Modify: `test/test_api_query.ml`
- Modify: `test/test_api_command.ml`
- Modify: `lib/repo/codec.ml`

Adding a field to `Schema.t` breaks every call site that constructs one. This task fixes them all to pass `sensors = []`, then adds positive tests for the new field.

- [ ] **Step 1: Write the failing test**

Append to `test/test_domain_schema.ml`:

```ocaml
let sensors_field_lookup () =
  let s : Schema.t =
    { (sample_schema ()) with
      sensors = [
        (Level.Hn4, [
          Sensor_slot.{
            kind = "electricity"; min = None; max = Some 1;
            meter_type = Either; purposes = Some [ "Electricity" ];
          };
        ]);
      ];
    }
  in
  let slots = Schema.sensors_for s Level.Hn4 in
  Alcotest.(check int) "one slot" 1 (List.length slots);
  let slots_hn3 = Schema.sensors_for s Level.Hn3 in
  Alcotest.(check int) "no slots at hn3" 0 (List.length slots_hn3)

let rejects_duplicate_sensor_kind () =
  let s : Schema.t =
    { (sample_schema ()) with
      sensors = [
        (Level.Hn4, [
          Sensor_slot.{ kind = "electricity"; min = None; max = None;
                        meter_type = Either; purposes = None };
          Sensor_slot.{ kind = "electricity"; min = None; max = None;
                        meter_type = Counter; purposes = None };
        ]);
      ];
    }
  in
  match Schema.validate s with
  | Ok () -> Alcotest.fail "expected duplicate kind error"
  | Error _ -> ()
```

Extend `tests`:

```ocaml
    Alcotest.test_case "sensors_for lookup"           `Quick sensors_field_lookup;
    Alcotest.test_case "rejects duplicate sensor kind" `Quick rejects_duplicate_sensor_kind;
```

- [ ] **Step 2: Run test to verify it fails**

Run: `dune test`
Expected: build failure — `The record field "sensors" does not belong to type Schema.t`.

- [ ] **Step 3: Extend Schema and fix all call sites**

In `lib/domain/schema.ml`, replace the top of the file:

```ocaml
type edge_spec = { label : string; min : int option; max : int option }

type t = {
  version : int;
  edges : (Level.t * (Level.t * edge_spec list) list) list;
  metadata : (Level.t * (string * Metadata.field_spec) list) list;
  sensors : (Level.t * Sensor_slot.t list) list;
}

let allowed_children t parent =
  match List.assoc_opt parent t.edges with
  | Some cs -> cs
  | None -> []

let edges_between t parent child =
  match List.assoc_opt child (allowed_children t parent) with
  | Some specs -> specs
  | None -> []

let metadata_for t level =
  match List.assoc_opt level t.metadata with
  | Some fs -> fs
  | None -> []

let sensors_for t level =
  match List.assoc_opt level t.sensors with
  | Some xs -> xs
  | None -> []
```

In the existing `validate` function, after the metadata iteration and before `Ok ()`, add:

```ocaml
    List.iter
      (fun (level, slots) ->
        let seen = Hashtbl.create 4 in
        List.iter
          (fun (slot : Sensor_slot.t) ->
            (match Sensor_slot.validate slot with
             | Ok () -> ()
             | Error msg ->
                 raise (Bad (Printf.sprintf "%s sensor slot: %s"
                               (Level.to_string level) msg)));
            if Hashtbl.mem seen slot.kind then
              raise (Bad (Printf.sprintf "%s sensor slot: duplicate kind %S"
                            (Level.to_string level) slot.kind));
            Hashtbl.add seen slot.kind ())
          slots)
      t.sensors;
```

Now fix every call site. Search with `rg 'Schema\.\{'` and add `sensors = [];` to each record literal:

- `lib/repo/codec.ml` in `decode_schema`: after `metadata` binding, return `Ok Schema.{ version; edges; metadata; sensors = [] }`.
- `test/test_domain_schema.ml` `sample_schema ()` record.
- `test/test_logic_hierarchy.ml` `sample_schema` record.
- `test/test_logic_schema_check.ml` `sample_schema` record.
- `test/test_logic_properties.ml` — any Schema records.
- `test/test_api_query.ml`, `test/test_api_command.ml` — any Schema records.
- `itest/test_dynamo.ml` two schema constructors (`property_schema`, `chargepoint_schema`) and the inline one in `fresh_root`.

- [ ] **Step 4: Run test to verify it passes**

Run: `dune test`
Expected: PASS — existing tests still green + two new Schema tests green.

- [ ] **Step 5: Commit**

```bash
git add lib/domain/schema.ml lib/repo/codec.ml test/ itest/
git commit -m "feat(schema): add sensors field with per-level Sensor_slot list"
```

---

## Phase 2 — Effects and in-memory handler

### Task 8: Sensor effects

**Files:**
- Modify: `lib/effects.ml`

No test in this task — effects are consumed in Task 9 where the memory handler gets its own tests.

- [ ] **Step 1: (skip) — no direct test**

- [ ] **Step 2: (skip)**

- [ ] **Step 3: Add sensor effects**

Append to `lib/effects.ml`, before `let get_node ...`:

```ocaml
type _ Effect.t +=
  | Put_sensor            : { sensor : Sensor.t; parent : Node_id.t } -> unit Effect.t
  | Get_active_sensor     : Sensor_id.t -> Sensor.t option Effect.t
  | List_sensor_ids       : Node_id.t -> Sensor_id.t list Effect.t
  | Replace_sensor_device : {
      old_active_from : Ptime.t;
      new_sensor : Sensor.t;
    } -> unit Effect.t
  | Delete_sensor         : { sensor_id : Sensor_id.t; parent : Node_id.t }
                              -> unit Effect.t
  | Get_sensor_reading    : Sensor_id.t -> float option Effect.t
```

And the thin wrappers:

```ocaml
let put_sensor ~sensor ~parent =
  Effect.perform (Put_sensor { sensor; parent })

let get_active_sensor id =
  Effect.perform (Get_active_sensor id)

let list_sensor_ids parent =
  Effect.perform (List_sensor_ids parent)

let replace_sensor_device ~old_active_from ~new_sensor =
  Effect.perform (Replace_sensor_device { old_active_from; new_sensor })

let delete_sensor ~sensor_id ~parent =
  Effect.perform (Delete_sensor { sensor_id; parent })

let get_sensor_reading id =
  Effect.perform (Get_sensor_reading id)
```

- [ ] **Step 4: Build to verify it still compiles**

Run: `dune build`
Expected: success — no tests run yet.

- [ ] **Step 5: Commit**

```bash
git add lib/effects.ml
git commit -m "feat(effects): add sensor effects (put, get_active, list_ids, replace_device, reading)"
```

---

### Task 9: In-memory handler for sensor effects

**Files:**
- Modify: `lib/repo/memory.ml`
- Modify: `test/test_repo_memory.ml`

- [ ] **Step 1: Write the failing test**

Append to `test/test_repo_memory.ml`:

```ocaml
let ptime_of s = Ptime.of_rfc3339 s |> Result.get_ok |> fun (t, _, _) -> t
let uuid_of s = Uuidm.of_string s |> Option.get

let mk_sensor ~sensor_uuid ~parent ~active_from ~daq : Sensor.t =
  {
    id = Sensor_id.make sensor_uuid;
    active_from;
    parent;
    daq_address = daq;
    hierarchy_path = "";
    purpose = "Electricity";
    meter_type = Sensor.Counter;
    unit = Some "kWh";
    formula = Formula.Identity;
  }

let memory_put_and_get_active_sensor () =
  let st = Memory.empty () in
  let parent = Node_id.make Level.Hn4 (uuid_of "aaaaaaaa-0000-4000-8000-000000000001") in
  let s_uuid = uuid_of "bbbbbbbb-0000-4000-8000-000000000001" in
  let s =
    mk_sensor ~sensor_uuid:s_uuid ~parent
      ~active_from:(ptime_of "2026-04-18T10:00:00Z")
      ~daq:"daq:x"
  in
  Memory.run st (fun () ->
    Effects.put_sensor ~sensor:s ~parent;
    match Effects.get_active_sensor s.Sensor.id with
    | Some s2 ->
        Alcotest.(check string) "daq preserved" "daq:x" s2.Sensor.daq_address
    | None -> Alcotest.fail "expected active")

let memory_list_sensor_ids_returns_attached () =
  let st = Memory.empty () in
  let parent = Node_id.make Level.Hn4 (uuid_of "aaaaaaaa-0000-4000-8000-000000000002") in
  let s1 =
    mk_sensor
      ~sensor_uuid:(uuid_of "bbbbbbbb-0000-4000-8000-000000000010") ~parent
      ~active_from:(ptime_of "2026-04-18T10:00:00Z") ~daq:"daq:a"
  in
  let s2 =
    mk_sensor
      ~sensor_uuid:(uuid_of "bbbbbbbb-0000-4000-8000-000000000011") ~parent
      ~active_from:(ptime_of "2026-04-18T11:00:00Z") ~daq:"daq:b"
  in
  Memory.run st (fun () ->
    Effects.put_sensor ~sensor:s1 ~parent;
    Effects.put_sensor ~sensor:s2 ~parent;
    let ids = Effects.list_sensor_ids parent in
    Alcotest.(check int) "two sensors attached" 2 (List.length ids))

let memory_replace_device_demotes_old_and_promotes_new () =
  let st = Memory.empty () in
  let parent = Node_id.make Level.Hn4 (uuid_of "aaaaaaaa-0000-4000-8000-000000000003") in
  let s_uuid = uuid_of "bbbbbbbb-0000-4000-8000-000000000020" in
  let old_t = ptime_of "2026-03-01T00:00:00Z" in
  let new_t = ptime_of "2026-04-18T10:00:00Z" in
  let old_s = mk_sensor ~sensor_uuid:s_uuid ~parent ~active_from:old_t ~daq:"daq:old" in
  let new_s = mk_sensor ~sensor_uuid:s_uuid ~parent ~active_from:new_t ~daq:"daq:new" in
  Memory.run st (fun () ->
    Effects.put_sensor ~sensor:old_s ~parent;
    Effects.replace_sensor_device ~old_active_from:old_t ~new_sensor:new_s;
    match Effects.get_active_sensor old_s.Sensor.id with
    | Some s ->
        Alcotest.(check string) "active daq is the new one"
          "daq:new" s.Sensor.daq_address
    | None -> Alcotest.fail "expected active after replace")

let tests =
  tests @
  [
    Alcotest.test_case "put + get_active"         `Quick memory_put_and_get_active_sensor;
    Alcotest.test_case "list_sensor_ids"          `Quick memory_list_sensor_ids_returns_attached;
    Alcotest.test_case "replace_device"           `Quick memory_replace_device_demotes_old_and_promotes_new;
  ]
```

- [ ] **Step 2: Run test to verify it fails**

Run: `dune test`
Expected: build failure — `Unbound value Effects.put_sensor` or similar.

- [ ] **Step 3: Extend the memory handler**

In `lib/repo/memory.ml`, extend `state`:

```ocaml
type sensor_row = Active of Sensor.t | History of Sensor.t

type state = {
  nodes    : (string, Node.t) Hashtbl.t;
  edges    : (Node_id.t * string * Node_id.t) list ref;
  sensors  : (string, sensor_row list) Hashtbl.t;
  rng      : Random.State.t;
  clock    : unit -> Ptime.t;
}
```

Update `empty`:

```ocaml
let empty ?(seed = 42) ?(clock = Ptime_clock.now) () =
  {
    nodes = Hashtbl.create 32;
    edges = ref [];
    sensors = Hashtbl.create 32;
    rng = Random.State.make [| seed |];
    clock;
  }
```

Add helpers near `find_node`:

```ocaml
let sensor_rows st id =
  Hashtbl.find_opt st.sensors (Sensor_id.to_string id) |> Option.value ~default:[]

let set_sensor_rows st id rows =
  Hashtbl.replace st.sensors (Sensor_id.to_string id) rows

let active_of_rows rows =
  List.find_map (function Active s -> Some s | History _ -> None) rows
```

Extend the `effc` block by adding new match arms:

```ocaml
          | Effects.Put_sensor { sensor; parent } ->
              let rows = sensor_rows st sensor.Sensor.id in
              set_sensor_rows st sensor.Sensor.id (Active sensor :: rows);
              st.edges :=
                (parent, "sensor", Node_id.make Level.Hn9 (Sensor_id.uuid sensor.Sensor.id))
                :: !(st.edges);
              Some (fun k -> continue k ())
          | Effects.Get_active_sensor id ->
              let v = active_of_rows (sensor_rows st id) in
              Some (fun k -> continue k v)
          | Effects.List_sensor_ids parent ->
              let ids =
                List.filter_map
                  (fun (p, lbl, c) ->
                    if Node_id.equal p parent && lbl = "sensor"
                    then Some (Sensor_id.make (Node_id.uuid c))
                    else None)
                  !(st.edges)
              in
              Some (fun k -> continue k ids)
          | Effects.Replace_sensor_device { old_active_from; new_sensor } ->
              let id = new_sensor.Sensor.id in
              let rows = sensor_rows st id in
              let demote = function
                | Active s when Ptime.equal s.Sensor.active_from old_active_from ->
                    History s
                | other -> other
              in
              let demoted = List.map demote rows in
              set_sensor_rows st id (Active new_sensor :: demoted);
              Some (fun k -> continue k ())
          | Effects.Delete_sensor { sensor_id; parent } ->
              Hashtbl.remove st.sensors (Sensor_id.to_string sensor_id);
              let target = Sensor_id.uuid sensor_id in
              st.edges :=
                List.filter
                  (fun (p, lbl, c) ->
                    not
                      (Node_id.equal p parent && lbl = "sensor"
                       && Uuidm.equal (Node_id.uuid c) target))
                  !(st.edges);
              Some (fun k -> continue k ())
          | Effects.Get_sensor_reading _id ->
              Some (fun k -> continue k None)
```

(The memory handler stores sensor edges using the existing edges list with label `"sensor"`; the encoding for a sensor edge target is a synthetic `Node_id` carrying only the uuid — good enough for in-memory tests.)

- [ ] **Step 4: Run test to verify it passes**

Run: `dune test`
Expected: PASS — all three new memory tests green, existing tests unaffected.

- [ ] **Step 5: Commit**

```bash
git add lib/repo/memory.ml test/test_repo_memory.ml
git commit -m "feat(repo/memory): handle sensor effects with active/history rows"
```

---

## Phase 3 — Sensor business logic

### Task 10: `Sensors.attach`

**Files:**
- Create: `lib/logic/sensors.ml`
- Create: `test/test_logic_sensors.ml`
- Modify: `test/test_ocaml_lambda_test.ml`

- [ ] **Step 1: Write the failing test**

Create `test/test_logic_sensors.ml`:

```ocaml
open Ocaml_lambda_test

let uuid_of s = Uuidm.of_string s |> Option.get

let sample_schema () : Schema.t =
  Schema.{
    version = 1;
    edges = [
      (Level.Hn2, [ (Level.Hn3, [ { label = "building"; min = None; max = None } ]) ]);
    ];
    metadata = [];
    sensors = [
      (Level.Hn3, [
        Sensor_slot.{
          kind = "electricity"; min = None; max = Some 1;
          meter_type = Either; purposes = Some [ "Electricity" ];
        };
      ]);
    ];
  }

let seed_company st =
  let c2 = Node_id.make Level.Hn2 (uuid_of "4b6a6f20-0000-0000-0000-00000000cccc") in
  let n2 =
    Node.make ~uuid:(Node_id.uuid c2) ~level:Level.Hn2 ~name:"Acme"
      ~parent:Node_id.root ~created:Ptime.epoch
      ~metadata:(`Assoc []) ~schema:(Some (sample_schema ()))
  in
  Memory.run st (fun () -> Effects.put_node n2);
  c2

let seed_building st c2 =
  Memory.run st (fun () ->
    match
      Hierarchy.add_node
        ~parent:c2 ~level:Level.Hn3 ~name:"B" ~metadata:(`Assoc []) ()
    with
    | Ok b -> b.Node.id
    | Error e -> Alcotest.failf "seed: %s" (Errors.message e))

let attach_happy () =
  let st = Memory.empty () in
  let c2 = seed_company st in
  let bldg = seed_building st c2 in
  Memory.run st (fun () ->
    match
      Sensors.attach
        ~parent:bldg ~kind:"electricity" ~daq_address:"daq:1"
        ~purpose:"Electricity" ~meter_type:Sensor.Counter ~unit:"kWh"
        ~formula:Formula.Identity ()
    with
    | Error e -> Alcotest.failf "attach: %s" (Errors.message e)
    | Ok s ->
        Alcotest.(check string) "daq" "daq:1" s.Sensor.daq_address;
        Alcotest.(check string) "parent wired"
          (Node_id.to_string bldg) (Node_id.to_string s.Sensor.parent))

let rejects_missing_kind_slot () =
  let st = Memory.empty () in
  let c2 = seed_company st in
  let bldg = seed_building st c2 in
  Memory.run st (fun () ->
    match
      Sensors.attach
        ~parent:bldg ~kind:"water" ~daq_address:"daq:2"
        ~purpose:"Water" ~meter_type:Sensor.Counter ()
    with
    | Ok _ -> Alcotest.fail "expected Validation"
    | Error (Errors.Validation _) -> ()
    | Error e -> Alcotest.failf "wrong error: %s" (Errors.message e))

let rejects_max_exceeded () =
  let st = Memory.empty () in
  let c2 = seed_company st in
  let bldg = seed_building st c2 in
  Memory.run st (fun () ->
    let _ = Sensors.attach ~parent:bldg ~kind:"electricity"
              ~daq_address:"daq:a" ~purpose:"Electricity"
              ~meter_type:Sensor.Counter () in
    match
      Sensors.attach ~parent:bldg ~kind:"electricity"
        ~daq_address:"daq:b" ~purpose:"Electricity"
        ~meter_type:Sensor.Counter ()
    with
    | Ok _ -> Alcotest.fail "max=1 should have rejected second attach"
    | Error (Errors.Validation _) -> ()
    | Error e -> Alcotest.failf "wrong error: %s" (Errors.message e))

let rejects_purpose_not_in_whitelist () =
  let st = Memory.empty () in
  let c2 = seed_company st in
  let bldg = seed_building st c2 in
  Memory.run st (fun () ->
    match
      Sensors.attach ~parent:bldg ~kind:"electricity"
        ~daq_address:"daq:x" ~purpose:"Water"
        ~meter_type:Sensor.Counter ()
    with
    | Ok _ -> Alcotest.fail "purpose Water should be rejected"
    | Error (Errors.Validation _) -> ()
    | Error e -> Alcotest.failf "wrong error: %s" (Errors.message e))

let rejects_meter_type_mismatch () =
  let st = Memory.empty () in
  let c2 =
    Memory.run st (fun () ->
      let u = uuid_of "4b6a6f20-0000-0000-0000-00000000dddd" in
      let sch : Schema.t =
        { (sample_schema ()) with
          sensors = [
            (Level.Hn3, [
              Sensor_slot.{
                kind = "electricity"; min = None; max = None;
                meter_type = Counter; purposes = None;
              };
            ]);
          ];
        }
      in
      let n = Node.make ~uuid:u ~level:Level.Hn2 ~name:"C"
                ~parent:Node_id.root ~created:Ptime.epoch
                ~metadata:(`Assoc []) ~schema:(Some sch)
      in
      Effects.put_node n;
      Node_id.make Level.Hn2 u)
  in
  let bldg = seed_building st c2 in
  Memory.run st (fun () ->
    match
      Sensors.attach ~parent:bldg ~kind:"electricity"
        ~daq_address:"daq:y" ~purpose:"Electricity"
        ~meter_type:Sensor.Gauge ()
    with
    | Ok _ -> Alcotest.fail "gauge should be rejected in Counter slot"
    | Error (Errors.Validation _) -> ()
    | Error e -> Alcotest.failf "wrong error: %s" (Errors.message e))

let tests =
  [
    Alcotest.test_case "attach happy path"       `Quick attach_happy;
    Alcotest.test_case "rejects missing kind"    `Quick rejects_missing_kind_slot;
    Alcotest.test_case "rejects max exceeded"    `Quick rejects_max_exceeded;
    Alcotest.test_case "rejects bad purpose"     `Quick rejects_purpose_not_in_whitelist;
    Alcotest.test_case "rejects meter mismatch"  `Quick rejects_meter_type_mismatch;
  ]
```

Register `("logic.sensors", Test_logic_sensors.tests);` in `test/test_ocaml_lambda_test.ml`.

- [ ] **Step 2: Run test to verify it fails**

Run: `dune test`
Expected: build failure — `Unbound module Sensors`.

- [ ] **Step 3: Write minimal implementation**

Create `lib/logic/sensors.ml`:

```ocaml
let ( let* ) = Result.bind

let validation_err msg =
  Errors.Validation [ { Metadata.path = ""; message = msg } ]

let attach ?(formula = Formula.Identity) ?unit ~parent ~kind
    ~daq_address ~purpose ~meter_type () =
  let* parent_node =
    match Effects.get_node parent with
    | None -> Error (Errors.Not_found parent)
    | Some n -> Ok n
  in
  let parent_level = Node_id.level parent_node.Node.id in
  let* _host, schema = Schema_check.find_for parent in
  let slots = Schema.sensors_for schema parent_level in
  let* slot =
    match List.find_opt (fun (s : Sensor_slot.t) -> s.kind = kind) slots with
    | Some s -> Ok s
    | None ->
        Error (validation_err
                 (Printf.sprintf "no sensor slot %S at %s"
                    kind (Level.to_string parent_level)))
  in
  let* () =
    if Sensor_slot.allows_meter_type slot meter_type then Ok ()
    else Error (validation_err
                  (Printf.sprintf "meter type not allowed by slot %S" kind))
  in
  let* () =
    if Sensor_slot.allows_purpose slot purpose then Ok ()
    else Error (validation_err
                  (Printf.sprintf "purpose %S not allowed by slot %S"
                     purpose kind))
  in
  let attached = Effects.list_sensor_ids parent in
  let same_kind_count =
    List.fold_left
      (fun acc id ->
        match Effects.get_active_sensor id with
        | Some s when s.Sensor.purpose = purpose || s.Sensor.daq_address = daq_address ->
            acc + 1
        | _ -> acc)
      0 attached
  in
  let* () =
    match slot.max with
    | Some m when same_kind_count >= m ->
        Error (validation_err
                 (Printf.sprintf "max %d sensors of kind %S per parent"
                    m kind))
    | _ -> Ok ()
  in
  let uuid = Effects.gen_uuid () in
  let now = Effects.now () in
  let sensor : Sensor.t =
    {
      id = Sensor_id.make uuid;
      active_from = now;
      parent;
      daq_address;
      hierarchy_path = Node_id.to_string parent;
      purpose;
      meter_type;
      unit;
      formula;
    }
  in
  Effects.put_sensor ~sensor ~parent;
  Ok sensor
```

- [ ] **Step 4: Run test to verify it passes**

Run: `dune test`
Expected: PASS — all five attach tests green.

- [ ] **Step 5: Commit**

```bash
git add lib/logic/sensors.ml test/test_logic_sensors.ml test/test_ocaml_lambda_test.ml
git commit -m "feat(logic/sensors): attach with slot/meter/purpose/max validation"
```

---

### Task 11: `Sensors.list_active`

**Files:**
- Modify: `lib/logic/sensors.ml`
- Modify: `test/test_logic_sensors.ml`

- [ ] **Step 1: Write the failing test**

Append to `test/test_logic_sensors.ml`:

```ocaml
let list_active_returns_attached () =
  let st = Memory.empty () in
  let c2 = seed_company st in
  let bldg = seed_building st c2 in
  Memory.run st (fun () ->
    let _ = Sensors.attach ~parent:bldg ~kind:"electricity"
              ~daq_address:"daq:1" ~purpose:"Electricity"
              ~meter_type:Sensor.Counter () in
    match Sensors.list_active ~parent:bldg with
    | Error e -> Alcotest.failf "list: %s" (Errors.message e)
    | Ok xs ->
        Alcotest.(check int) "one sensor" 1 (List.length xs);
        let s = List.hd xs in
        Alcotest.(check string) "daq" "daq:1" s.Sensor.daq_address)

let list_active_empty_when_none () =
  let st = Memory.empty () in
  let c2 = seed_company st in
  let bldg = seed_building st c2 in
  Memory.run st (fun () ->
    match Sensors.list_active ~parent:bldg with
    | Error e -> Alcotest.failf "list: %s" (Errors.message e)
    | Ok xs -> Alcotest.(check int) "zero" 0 (List.length xs))
```

Extend `tests`:

```ocaml
    Alcotest.test_case "list_active returns attached" `Quick list_active_returns_attached;
    Alcotest.test_case "list_active empty"            `Quick list_active_empty_when_none;
```

- [ ] **Step 2: Run test to verify it fails**

Run: `dune test`
Expected: build failure — `Unbound value Sensors.list_active`.

- [ ] **Step 3: Implement `list_active`**

Append to `lib/logic/sensors.ml`:

```ocaml
let list_active ~parent =
  let ids = Effects.list_sensor_ids parent in
  let xs =
    List.filter_map
      (fun id -> Effects.get_active_sensor id)
      ids
  in
  Ok xs
```

- [ ] **Step 4: Run test to verify it passes**

Run: `dune test`
Expected: PASS — two new list_active cases green.

- [ ] **Step 5: Commit**

```bash
git add lib/logic/sensors.ml test/test_logic_sensors.ml
git commit -m "feat(logic/sensors): list_active via edges + per-sensor active fetch"
```

---

### Task 12: `Sensors.get_active`

**Files:**
- Modify: `lib/logic/sensors.ml`
- Modify: `test/test_logic_sensors.ml`

- [ ] **Step 1: Write the failing test**

Append to `test/test_logic_sensors.ml`:

```ocaml
let get_active_happy () =
  let st = Memory.empty () in
  let c2 = seed_company st in
  let bldg = seed_building st c2 in
  Memory.run st (fun () ->
    let s =
      match
        Sensors.attach ~parent:bldg ~kind:"electricity"
          ~daq_address:"daq:k" ~purpose:"Electricity"
          ~meter_type:Sensor.Counter ()
      with
      | Ok x -> x
      | Error e -> Alcotest.failf "attach: %s" (Errors.message e)
    in
    match Sensors.get_active s.Sensor.id with
    | Ok s2 -> Alcotest.(check string) "daq" "daq:k" s2.Sensor.daq_address
    | Error e -> Alcotest.failf "get: %s" (Errors.message e))

let get_active_unknown () =
  let st = Memory.empty () in
  Memory.run st (fun () ->
    let id = Sensor_id.make (uuid_of "ffffffff-0000-4000-8000-000000000001") in
    match Sensors.get_active id with
    | Ok _ -> Alcotest.fail "expected not found"
    | Error (Errors.Not_found _) -> ()
    | Error e -> Alcotest.failf "wrong error: %s" (Errors.message e))
```

Extend `tests`:

```ocaml
    Alcotest.test_case "get_active happy"   `Quick get_active_happy;
    Alcotest.test_case "get_active unknown" `Quick get_active_unknown;
```

- [ ] **Step 2: Run test to verify it fails**

Run: `dune test`
Expected: build failure — `Unbound value Sensors.get_active`.

- [ ] **Step 3: Implement `get_active`**

Append to `lib/logic/sensors.ml`:

```ocaml
let get_active id =
  match Effects.get_active_sensor id with
  | Some s -> Ok s
  | None ->
      (* Reuse Not_found by synthesizing a pseudo node-id from the sensor's uuid *)
      Error (Errors.Not_found
               (Node_id.make Level.Hn9 (Sensor_id.uuid id)))
```

- [ ] **Step 4: Run test to verify it passes**

Run: `dune test`
Expected: PASS — both get_active tests green.

- [ ] **Step 5: Commit**

```bash
git add lib/logic/sensors.ml test/test_logic_sensors.ml
git commit -m "feat(logic/sensors): get_active with Not_found on missing sensor"
```

---

### Task 13: `Sensors.replace_device`

**Files:**
- Modify: `lib/logic/sensors.ml`
- Modify: `test/test_logic_sensors.ml`

- [ ] **Step 1: Write the failing test**

Append to `test/test_logic_sensors.ml`:

```ocaml
let replace_device_promotes_new () =
  let st = Memory.empty () in
  let c2 = seed_company st in
  let bldg = seed_building st c2 in
  Memory.run st (fun () ->
    let s =
      match
        Sensors.attach ~parent:bldg ~kind:"electricity"
          ~daq_address:"daq:old" ~purpose:"Electricity"
          ~meter_type:Sensor.Counter ()
      with
      | Ok x -> x
      | Error e -> Alcotest.failf "attach: %s" (Errors.message e)
    in
    match Sensors.replace_device ~sensor_id:s.Sensor.id ~new_daq_address:"daq:new" () with
    | Error e -> Alcotest.failf "replace: %s" (Errors.message e)
    | Ok s2 ->
        Alcotest.(check string) "new daq" "daq:new" s2.Sensor.daq_address;
        (match Sensors.get_active s.Sensor.id with
         | Ok cur ->
             Alcotest.(check string) "active is new" "daq:new" cur.Sensor.daq_address;
             Alcotest.(check bool) "active_from updated" true
               (not (Ptime.equal s.Sensor.active_from cur.Sensor.active_from))
         | Error e -> Alcotest.failf "get: %s" (Errors.message e)))

let replace_unknown_fails () =
  let st = Memory.empty () in
  Memory.run st (fun () ->
    let id = Sensor_id.make (uuid_of "ffffffff-0000-4000-8000-000000000002") in
    match Sensors.replace_device ~sensor_id:id ~new_daq_address:"x" () with
    | Ok _ -> Alcotest.fail "expected Not_found"
    | Error (Errors.Not_found _) -> ()
    | Error e -> Alcotest.failf "wrong error: %s" (Errors.message e))
```

Extend `tests`:

```ocaml
    Alcotest.test_case "replace_device promotes new" `Quick replace_device_promotes_new;
    Alcotest.test_case "replace_device unknown"      `Quick replace_unknown_fails;
```

- [ ] **Step 2: Run test to verify it fails**

Run: `dune test`
Expected: build failure — `Unbound value Sensors.replace_device`.

- [ ] **Step 3: Implement `replace_device`**

Append to `lib/logic/sensors.ml`:

```ocaml
let replace_device ~sensor_id ~new_daq_address () =
  let* old =
    match Effects.get_active_sensor sensor_id with
    | Some s -> Ok s
    | None ->
        Error (Errors.Not_found
                 (Node_id.make Level.Hn9 (Sensor_id.uuid sensor_id)))
  in
  let now = Effects.now () in
  let new_sensor =
    { old with
      Sensor.active_from = now;
      daq_address = new_daq_address;
    }
  in
  Effects.replace_sensor_device
    ~old_active_from:old.Sensor.active_from ~new_sensor;
  Ok new_sensor
```

- [ ] **Step 4: Run test to verify it passes**

Run: `dune test`
Expected: PASS — two replace tests green.

- [ ] **Step 5: Commit**

```bash
git add lib/logic/sensors.ml test/test_logic_sensors.ml
git commit -m "feat(logic/sensors): replace_device promotes new, demotes old to history"
```

---

### Task 14: Cycle detection on attach

**Files:**
- Modify: `lib/domain/formula.ml`
- Modify: `lib/logic/sensors.ml`
- Modify: `test/test_domain_formula.ml`
- Modify: `test/test_logic_sensors.ml`

- [ ] **Step 1: Write the failing test**

Append to `test/test_domain_formula.ml`:

```ocaml
let collect_refs_identity_empty () =
  let xs = Formula.referenced_uuids Formula.Identity in
  Alcotest.(check int) "none" 0 (List.length xs)

let collect_refs_of_expr () =
  let ast = Formula.Sub (Formula.Ref "S1", Formula.Ref "S2") in
  let f = Formula.Expr { ast; refs = [ ("S1", uuid_a); ("S2", uuid_b) ] } in
  let xs = Formula.referenced_uuids f |> List.sort Uuidm.compare in
  let expected = List.sort Uuidm.compare [ uuid_a; uuid_b ] in
  Alcotest.(check int) "two" 2 (List.length xs);
  Alcotest.(check bool) "set equal" true
    (List.for_all2 Uuidm.equal xs expected)
```

Extend `tests`:

```ocaml
    Alcotest.test_case "referenced_uuids identity" `Quick collect_refs_identity_empty;
    Alcotest.test_case "referenced_uuids expr"     `Quick collect_refs_of_expr;
```

Append to `test/test_logic_sensors.ml`:

```ocaml
let attach_detects_self_cycle () =
  let st = Memory.empty () in
  let c2 = seed_company st in
  let bldg = seed_building st c2 in
  Memory.run st (fun () ->
    let s =
      match
        Sensors.attach ~parent:bldg ~kind:"electricity"
          ~daq_address:"daq:1" ~purpose:"Electricity"
          ~meter_type:Sensor.Counter ()
      with
      | Ok x -> x
      | Error e -> Alcotest.failf "attach: %s" (Errors.message e)
    in
    let cyc =
      Formula.Expr {
        ast = Formula.Ref "self_again";
        refs = [ ("self_again", Sensor_id.uuid s.Sensor.id) ];
      }
    in
    match
      Sensors.set_formula ~sensor_id:s.Sensor.id ~formula:cyc ()
    with
    | Ok _ -> Alcotest.fail "should reject cycle"
    | Error (Errors.Validation _) -> ()
    | Error e -> Alcotest.failf "wrong error: %s" (Errors.message e))
```

Extend `tests`:

```ocaml
    Alcotest.test_case "attach rejects cycle" `Quick attach_detects_self_cycle;
```

- [ ] **Step 2: Run test to verify it fails**

Run: `dune test`
Expected: build failure — `Unbound value Formula.referenced_uuids` / `Sensors.set_formula`.

- [ ] **Step 3: Implement helpers and cycle detection**

Append to `lib/domain/formula.ml`:

```ocaml
let referenced_uuids = function
  | Identity -> []
  | Expr { refs; _ } -> List.map snd refs
```

Append to `lib/logic/sensors.ml`:

```ocaml
let rec walk_refs visited uuid =
  if List.exists (Uuidm.equal uuid) visited then `Cycle
  else
    let id = Sensor_id.make uuid in
    match Effects.get_active_sensor id with
    | None -> `Ok
    | Some s ->
        let next_uuids = Formula.referenced_uuids s.Sensor.formula in
        let visited' = uuid :: visited in
        List.fold_left
          (fun acc u ->
            match acc with
            | `Cycle -> `Cycle
            | `Ok -> walk_refs visited' u)
          `Ok next_uuids

let has_cycle ~self_uuid formula =
  let next = Formula.referenced_uuids formula in
  List.exists
    (fun u ->
      Uuidm.equal u self_uuid
      || walk_refs [ self_uuid ] u = `Cycle)
    next

let set_formula ~sensor_id ~formula () =
  let* old = get_active sensor_id in
  let self_uuid = Sensor_id.uuid sensor_id in
  let* () =
    if has_cycle ~self_uuid formula
    then Error (validation_err "formula refs form a cycle")
    else Ok ()
  in
  let now = Effects.now () in
  let new_sensor =
    { old with Sensor.active_from = now; formula }
  in
  Effects.replace_sensor_device
    ~old_active_from:old.Sensor.active_from ~new_sensor;
  Ok new_sensor
```

Also gate cycles in the plain `attach` flow — add this check before `Effects.put_sensor` inside `attach`:

```ocaml
  let* () =
    if has_cycle ~self_uuid:uuid formula
    then Error (validation_err "formula refs form a cycle")
    else Ok ()
  in
```

- [ ] **Step 4: Run test to verify it passes**

Run: `dune test`
Expected: PASS — cycle detection tests green, existing sensor tests still green.

- [ ] **Step 5: Commit**

```bash
git add lib/domain/formula.ml lib/logic/sensors.ml test/test_domain_formula.ml test/test_logic_sensors.ml
git commit -m "feat(logic/sensors): reject formula ref cycles on attach and set_formula"
```

---

### Task 15: `Sensors.evaluate` (compute S' with recursive ref resolution)

**Files:**
- Modify: `lib/logic/sensors.ml`
- Modify: `test/test_logic_sensors.ml`

This task uses the `Get_sensor_reading` effect as the raw-reading source. For in-memory tests we swap in a wrapping handler that returns canned readings.

- [ ] **Step 1: Write the failing test**

Append to `test/test_logic_sensors.ml`:

```ocaml
(* Wraps a Memory-backed state with a canned reading lookup. *)
let run_with_readings st readings f =
  let open Effect.Deep in
  try_with
    (fun () -> Memory.run st f)
    ()
    {
      effc =
        (fun (type a) (eff : a Effect.t) ->
          match eff with
          | Effects.Get_sensor_reading id ->
              let v = List.assoc_opt (Sensor_id.to_string id) readings in
              Some (fun (k : (a, _) continuation) -> continue k v)
          | _ -> None);
    }

let evaluate_identity () =
  let st = Memory.empty () in
  let c2 = seed_company st in
  let bldg = seed_building st c2 in
  let s =
    Memory.run st (fun () ->
      match
        Sensors.attach ~parent:bldg ~kind:"electricity"
          ~daq_address:"daq:1" ~purpose:"Electricity"
          ~meter_type:Sensor.Counter ()
      with
      | Ok x -> x
      | Error e -> Alcotest.failf "attach: %s" (Errors.message e))
  in
  let readings = [ (Sensor_id.to_string s.Sensor.id, 42.0) ] in
  run_with_readings st readings (fun () ->
    match Sensors.evaluate s.Sensor.id with
    | Ok v -> Alcotest.(check (float 1e-9)) "raw passed through" 42.0 v
    | Error e -> Alcotest.failf "eval: %s" (Errors.message e))

let evaluate_composite () =
  let st = Memory.empty () in
  let c2 = seed_company st in
  let bldg = seed_building st c2 in
  Memory.run st (fun () ->
    let s4 =
      match
        Sensors.attach ~parent:bldg ~kind:"electricity"
          ~daq_address:"daq:4" ~purpose:"Electricity"
          ~meter_type:Sensor.Counter ()
      with
      | Ok x -> x
      | Error e -> Alcotest.failf "attach s4: %s" (Errors.message e)
    in
    let formula_s3 =
      Formula.Expr {
        ast = Formula.Abs (Formula.Sub (Formula.Self, Formula.Ref "s4"));
        refs = [ ("s4", Sensor_id.uuid s4.Sensor.id) ];
      }
    in
    let s3 =
      match
        Sensors.attach ~parent:bldg ~kind:"electricity"
          ~daq_address:"daq:3" ~purpose:"Electricity"
          ~meter_type:Sensor.Counter ~formula:formula_s3 ()
      with
      | Ok _ ->
          Alcotest.fail "max=1 should block; widen the slot for this test"
      | Error _ ->
          (* Relax the schema for this test: use a slot with max=None *)
          let sch : Schema.t =
            { (sample_schema ()) with
              sensors = [
                (Level.Hn3, [
                  Sensor_slot.{
                    kind = "electricity"; min = None; max = None;
                    meter_type = Either;
                    purposes = Some [ "Electricity" ];
                  };
                ]);
              ];
            }
          in
          let c2_node =
            Option.get (Effects.get_node c2)
          in
          Effects.put_node { c2_node with Node.schema = Some sch };
          (match
             Sensors.attach ~parent:bldg ~kind:"electricity"
               ~daq_address:"daq:3" ~purpose:"Electricity"
               ~meter_type:Sensor.Counter ~formula:formula_s3 ()
           with
           | Ok x -> x
           | Error e -> Alcotest.failf "attach s3: %s" (Errors.message e))
    in
    let readings =
      [
        (Sensor_id.to_string s4.Sensor.id, 3.0);
        (Sensor_id.to_string s3.Sensor.id, 10.0);
      ]
    in
    run_with_readings st readings (fun () ->
      match Sensors.evaluate s3.Sensor.id with
      | Ok v ->
          Alcotest.(check (float 1e-9)) "|10 - 3| = 7" 7.0 v
      | Error e -> Alcotest.failf "eval: %s" (Errors.message e)))
```

Extend `tests`:

```ocaml
    Alcotest.test_case "evaluate identity"  `Quick evaluate_identity;
    Alcotest.test_case "evaluate composite" `Quick evaluate_composite;
```

- [ ] **Step 2: Run test to verify it fails**

Run: `dune test`
Expected: build failure — `Unbound value Sensors.evaluate`.

- [ ] **Step 3: Implement `evaluate`**

Append to `lib/logic/sensors.ml`:

```ocaml
let rec evaluate id =
  let* s = get_active id in
  let* self_reading =
    match Effects.get_sensor_reading id with
    | Some v -> Ok v
    | None -> Error (Errors.Not_found (Node_id.make Level.Hn9 (Sensor_id.uuid id)))
  in
  match s.Sensor.formula with
  | Formula.Identity -> Ok self_reading
  | Formula.Expr { refs; _ } as f ->
      let rec resolve_all acc = function
        | [] -> Ok (List.rev acc)
        | (alias, uuid) :: rest ->
            let* v = evaluate (Sensor_id.make uuid) in
            resolve_all ((alias, v) :: acc) rest
      in
      let* resolved = resolve_all [] refs in
      let resolve_alias alias =
        match List.assoc_opt alias resolved with
        | Some v -> v
        | None -> raise (Formula.Unknown_ref alias)
      in
      Ok (Formula.eval ~self:self_reading ~resolve:resolve_alias f)
```

- [ ] **Step 4: Run test to verify it passes**

Run: `dune test`
Expected: PASS — both evaluate tests green.

- [ ] **Step 5: Commit**

```bash
git add lib/logic/sensors.ml test/test_logic_sensors.ml
git commit -m "feat(logic/sensors): evaluate resolves refs recursively into computed S'"
```

---

## Phase 4 — Persistence

### Task 16: Codec for sensors and formulas

**Files:**
- Modify: `lib/repo/codec.ml`
- Modify: `test/test_repo_codec.ml`

- [ ] **Step 1: Write the failing test**

Append to `test/test_repo_codec.ml`:

```ocaml
let sensor_round_trip () =
  let uuid = Uuidm.of_string "11111111-2222-4333-8444-000000000001" |> Option.get in
  let parent =
    Node_id.make Level.Hn4
      (Uuidm.of_string "22222222-2222-4333-8444-000000000002" |> Option.get)
  in
  let t = Ptime.of_rfc3339 "2026-04-18T10:00:00Z" |> Result.get_ok
          |> fun (t, _, _) -> t
  in
  let s : Sensor.t =
    {
      id = Sensor_id.make uuid;
      active_from = t;
      parent;
      daq_address = "daq:x:y:z";
      hierarchy_path = "P#C#B";
      purpose = "Electricity";
      meter_type = Sensor.Counter;
      unit = Some "kWh";
      formula = Formula.Expr {
        ast = Formula.Abs (Formula.Sub (Formula.Self, Formula.Ref "r"));
        refs = [ ("r", uuid) ];
      };
    }
  in
  let item = Codec.sensor_to_item ~active:true s in
  match Codec.sensor_of_item item with
  | Ok s2 ->
      Alcotest.(check string) "daq" s.daq_address s2.Sensor.daq_address;
      Alcotest.(check string) "purpose" s.purpose s2.Sensor.purpose;
      Alcotest.(check bool)   "formula kind" true
        (match s2.Sensor.formula with Formula.Expr _ -> true | _ -> false)
  | Error e -> Alcotest.failf "decode: %s" e

let sensor_active_sk_prefixed () =
  let uuid = Uuidm.of_string "11111111-2222-4333-8444-000000000001" |> Option.get in
  let t = Ptime.of_rfc3339 "2026-04-18T10:00:00Z" |> Result.get_ok
          |> fun (t, _, _) -> t
  in
  let s : Sensor.t =
    {
      id = Sensor_id.make uuid;
      active_from = t;
      parent = Node_id.root;
      daq_address = "daq";
      hierarchy_path = "";
      purpose = "Heat";
      meter_type = Sensor.Gauge;
      unit = None;
      formula = Formula.Identity;
    }
  in
  let item_active = Codec.sensor_to_item ~active:true s in
  let sk_active =
    match List.assoc_opt "sk" item_active with
    | Some (Smaws_Client_DynamoDB.S s) -> s
    | _ -> Alcotest.fail "no sk"
  in
  Alcotest.(check string) "active prefixed"
    "active#2026-04-18T10:00:00Z" sk_active;
  let item_hist = Codec.sensor_to_item ~active:false s in
  let sk_hist =
    match List.assoc_opt "sk" item_hist with
    | Some (Smaws_Client_DynamoDB.S s) -> s
    | _ -> Alcotest.fail "no sk"
  in
  Alcotest.(check string) "history bare timestamp"
    "2026-04-18T10:00:00Z" sk_hist
```

Extend `tests`:

```ocaml
    Alcotest.test_case "sensor round trip"        `Quick sensor_round_trip;
    Alcotest.test_case "sensor sk active/history" `Quick sensor_active_sk_prefixed;
```

- [ ] **Step 2: Run test to verify it fails**

Run: `dune test`
Expected: build failure — `Unbound value Codec.sensor_to_item`.

- [ ] **Step 3: Implement sensor codec**

Append to `lib/repo/codec.ml`:

```ocaml
let rec expr_to_attr : Formula.expr -> Dyn.attribute_value = function
  | Formula.Num f  -> Dyn.M [ ("t", s "num"); ("v", n (Printf.sprintf "%.17g" f)) ]
  | Formula.Self   -> Dyn.M [ ("t", s "self") ]
  | Formula.Ref a  -> Dyn.M [ ("t", s "ref"); ("a", s a) ]
  | Formula.Abs e  -> Dyn.M [ ("t", s "abs"); ("e", expr_to_attr e) ]
  | Formula.Add (a, b) ->
      Dyn.M [ ("t", s "add"); ("l", expr_to_attr a); ("r", expr_to_attr b) ]
  | Formula.Sub (a, b) ->
      Dyn.M [ ("t", s "sub"); ("l", expr_to_attr a); ("r", expr_to_attr b) ]
  | Formula.Mul (a, b) ->
      Dyn.M [ ("t", s "mul"); ("l", expr_to_attr a); ("r", expr_to_attr b) ]
  | Formula.Div (a, b) ->
      Dyn.M [ ("t", s "div"); ("l", expr_to_attr a); ("r", expr_to_attr b) ]

let rec expr_of_attr (v : Dyn.attribute_value) : (Formula.expr, string) result =
  let* kvs = as_map v in
  let* tag = field kvs "t" in
  let* tag_s = as_string tag in
  match tag_s with
  | "num" ->
      let* v = field kvs "v" in
      (match v with
       | Dyn.N s -> (try Ok (Formula.Num (float_of_string s))
                     with _ -> Error "bad num")
       | _ -> Error "num needs N")
  | "self" -> Ok Formula.Self
  | "ref" ->
      let* a = field kvs "a" in
      let* a_s = as_string a in
      Ok (Formula.Ref a_s)
  | "abs" ->
      let* e = field kvs "e" in
      let* e' = expr_of_attr e in
      Ok (Formula.Abs e')
  | "add" | "sub" | "mul" | "div" ->
      let* l = field kvs "l" in
      let* l' = expr_of_attr l in
      let* r = field kvs "r" in
      let* r' = expr_of_attr r in
      (match tag_s with
       | "add" -> Ok (Formula.Add (l', r'))
       | "sub" -> Ok (Formula.Sub (l', r'))
       | "mul" -> Ok (Formula.Mul (l', r'))
       | _     -> Ok (Formula.Div (l', r')))
  | other -> Error (Printf.sprintf "unknown expr tag %S" other)

let formula_to_attr (f : Formula.t) : Dyn.attribute_value =
  match f with
  | Formula.Identity -> Dyn.M [ ("kind", s "identity") ]
  | Formula.Expr { ast; refs } ->
      let refs_m =
        List.map (fun (a, u) -> (a, s (Uuidm.to_string u))) refs
      in
      Dyn.M [
        ("kind", s "expr");
        ("ast",  expr_to_attr ast);
        ("refs", Dyn.M refs_m);
      ]

let formula_of_attr (v : Dyn.attribute_value) : (Formula.t, string) result =
  let* kvs = as_map v in
  let* kind_v = field kvs "kind" in
  let* kind_s = as_string kind_v in
  match kind_s with
  | "identity" -> Ok Formula.Identity
  | "expr" ->
      let* ast_v = field kvs "ast" in
      let* ast = expr_of_attr ast_v in
      let* refs_v = field kvs "refs" in
      let* refs_m = as_map refs_v in
      let* refs =
        List.fold_left
          (fun acc (a, v) ->
            let* acc = acc in
            let* u_s = as_string v in
            match Uuidm.of_string u_s with
            | Some u -> Ok ((a, u) :: acc)
            | None -> Error (Printf.sprintf "bad ref uuid %S" u_s))
          (Ok []) refs_m
      in
      Ok (Formula.Expr { ast; refs = List.rev refs })
  | other -> Error (Printf.sprintf "unknown formula kind %S" other)

let sensor_to_item ~active (sn : Sensor.t) : (string * Dyn.attribute_value) list =
  let pk = Sensor_id.to_string sn.Sensor.id in
  let sk_t =
    if active
    then Sensor_sk.Active sn.Sensor.active_from
    else Sensor_sk.History sn.Sensor.active_from
  in
  let base =
    [
      ("pk", s pk);
      ("sk", s (Sensor_sk.to_string sk_t));
      ("type", s "sensor");
      ("parent", s (Node_id.to_string sn.Sensor.parent));
      ("daq_address", s sn.Sensor.daq_address);
      ("hierarchy_path", s sn.Sensor.hierarchy_path);
      ("purpose", s sn.Sensor.purpose);
      ("meter_type", s (Sensor.meter_type_to_string sn.Sensor.meter_type));
      ("formula", formula_to_attr sn.Sensor.formula);
      ("active_from", s (Ptime.to_rfc3339 ~tz_offset_s:0 sn.Sensor.active_from));
    ]
  in
  match sn.Sensor.unit with
  | Some u -> ("unit", s u) :: base
  | None -> base

let sensor_edge_item ~parent ~sensor_id ~created =
  let pk = Node_id.to_string parent in
  let sid = Sensor_id.to_string sensor_id in
  [
    ("pk", s pk);
    ("sk", s (Printf.sprintf "has_sensor#%s" sid));
    ("type", s "sensor_edge");
    ("sensor_id", s sid);
    ("created", s (Ptime.to_rfc3339 ~tz_offset_s:0 created));
  ]

let sensor_of_item kvs : (Sensor.t, string) result =
  let* pk = field kvs "pk" in
  let* pk_s = as_string pk in
  let* id = Sensor_id.of_string pk_s in
  let* parent_v = field kvs "parent" in
  let* parent_s = as_string parent_v in
  let* parent = Node_id.of_string parent_s in
  let* daq_v = field kvs "daq_address" in
  let* daq_address = as_string daq_v in
  let* hp_v = field kvs "hierarchy_path" in
  let* hierarchy_path = as_string hp_v in
  let* purpose_v = field kvs "purpose" in
  let* purpose = as_string purpose_v in
  let* mt_v = field kvs "meter_type" in
  let* mt_s = as_string mt_v in
  let* meter_type = Sensor.meter_type_of_string mt_s in
  let unit =
    match List.assoc_opt "unit" kvs with
    | Some (Dyn.S u) -> Some u
    | _ -> None
  in
  let* af_v = field kvs "active_from" in
  let* af_s = as_string af_v in
  let active_from =
    match Ptime.of_rfc3339 af_s with
    | Ok (t, _, _) -> t
    | Error _ -> Ptime.epoch
  in
  let* formula =
    match List.assoc_opt "formula" kvs with
    | Some v -> formula_of_attr v
    | None -> Ok Formula.Identity
  in
  Ok Sensor.{
    id; active_from; parent; daq_address; hierarchy_path;
    purpose; meter_type; unit; formula;
  }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `dune test`
Expected: PASS — codec round-trip and sk tests green.

- [ ] **Step 5: Commit**

```bash
git add lib/repo/codec.ml test/test_repo_codec.ml
git commit -m "feat(repo/codec): encode/decode sensors, formulas, and sensor edges"
```

---

### Task 17: DynamoDB handler for sensor effects

**Files:**
- Modify: `lib/repo/dynamo.ml`

No unit test in this task — dynamo code exercises via Task 19's integration test against a real table.

- [ ] **Step 1: (skip) — covered by integration test in Task 19**

- [ ] **Step 2: (skip)**

- [ ] **Step 3: Extend dynamo handler**

In `lib/repo/dynamo.ml`, add helpers after `delete_node`:

```ocaml
let active_sk_prefix = "active#"
let has_sensor_sk_prefix = "has_sensor#"

let put_sensor_active cfg (sensor : Sensor.t) =
  let item = Codec.sensor_to_item ~active:true sensor in
  let input = Dyn.make_put_item_input ~item ~table_name:cfg.table () in
  match Dyn.PutItem.request cfg.ctx input with
  | Ok _ -> ()
  | Error _ -> failwith "PutItem sensor active failed"

let put_sensor_edge cfg ~parent ~sensor_id =
  let item =
    Codec.sensor_edge_item ~parent ~sensor_id ~created:(Ptime_clock.now ())
  in
  let input = Dyn.make_put_item_input ~item ~table_name:cfg.table () in
  match Dyn.PutItem.request cfg.ctx input with
  | Ok _ -> ()
  | Error _ -> failwith "PutItem sensor edge failed"

let query_active_sensor cfg (id : Sensor_id.t) =
  let input =
    Dyn.make_query_input
      ~key_condition_expression:"#pk = :pk AND begins_with(#sk, :sk)"
      ~expression_attribute_names:[ ("#pk", "pk"); ("#sk", "sk") ]
      ~expression_attribute_values:[
        (":pk", s (Sensor_id.to_string id));
        (":sk", s active_sk_prefix);
      ]
      ~limit:1
      ~scan_index_forward:false
      ~table_name:cfg.table ()
  in
  match Dyn.Query.request cfg.ctx input with
  | Error _ -> None
  | Ok { items = None; _ } -> None
  | Ok { items = Some []; _ } -> None
  | Ok { items = Some (kvs :: _); _ } ->
      (match Codec.sensor_of_item kvs with
       | Ok s -> Some s
       | Error _ -> None)

let query_sensor_ids cfg parent =
  let pk_val = s (Node_id.to_string parent) in
  let input =
    Dyn.make_query_input
      ~key_condition_expression:"#pk = :pk AND begins_with(#sk, :sk)"
      ~expression_attribute_names:[ ("#pk", "pk"); ("#sk", "sk") ]
      ~expression_attribute_values:[ (":pk", pk_val); (":sk", s has_sensor_sk_prefix) ]
      ~table_name:cfg.table ()
  in
  match Dyn.Query.request cfg.ctx input with
  | Error _ -> []
  | Ok { items = None; _ } -> []
  | Ok { items = Some rows; _ } ->
      List.filter_map
        (fun kvs ->
          match List.assoc_opt "sensor_id" kvs with
          | Some (Dyn.S sid_s) ->
              (match Sensor_id.of_string sid_s with
               | Ok id -> Some id
               | Error _ -> None)
          | _ -> None)
        rows

let transact_replace cfg ~old_active_from ~new_sensor =
  let old_item = Codec.sensor_to_item ~active:true
    { new_sensor with Sensor.active_from = old_active_from }
  in
  let old_sk =
    match List.assoc_opt "sk" old_item with
    | Some (Dyn.S v) -> v
    | _ -> failwith "old_sk"
  in
  let old_pk =
    match List.assoc_opt "pk" old_item with
    | Some (Dyn.S v) -> v
    | _ -> failwith "old_pk"
  in
  let history_item =
    Codec.sensor_to_item ~active:false
      { new_sensor with Sensor.active_from = old_active_from }
  in
  let new_active_item = Codec.sensor_to_item ~active:true new_sensor in
  let delete =
    Dyn.make_delete
      ~key:[ ("pk", s old_pk); ("sk", s old_sk) ]
      ~table_name:cfg.table ()
  in
  let put_hist =
    Dyn.make_put ~item:history_item ~table_name:cfg.table ()
  in
  let put_new =
    Dyn.make_put ~item:new_active_item ~table_name:cfg.table ()
  in
  let items =
    [
      Dyn.make_transact_write_item ~delete ();
      Dyn.make_transact_write_item ~put:put_hist ();
      Dyn.make_transact_write_item ~put:put_new ();
    ]
  in
  let input = Dyn.make_transact_write_items_input ~transact_items:items () in
  match Dyn.TransactWriteItems.request cfg.ctx input with
  | Ok _ -> ()
  | Error _ -> failwith "TransactWriteItems replace failed"

let delete_sensor cfg (id : Sensor_id.t) (parent : Node_id.t) =
  let id_s = Sensor_id.to_string id in
  (* Delete active + history rows in the sensor's partition *)
  let input =
    Dyn.make_query_input
      ~key_condition_expression:"#pk = :pk"
      ~expression_attribute_names:[ ("#pk", "pk") ]
      ~expression_attribute_values:[ (":pk", s id_s) ]
      ~table_name:cfg.table ()
  in
  (match Dyn.Query.request cfg.ctx input with
   | Error _ -> ()
   | Ok { items = None; _ } -> ()
   | Ok { items = Some rows; _ } ->
       List.iter
         (fun kvs ->
           match List.assoc_opt "pk" kvs, List.assoc_opt "sk" kvs with
           | Some pkv, Some skv ->
               let _ = Dyn.DeleteItem.request cfg.ctx
                 (Dyn.make_delete_item_input
                    ~key:[ ("pk", pkv); ("sk", skv) ]
                    ~table_name:cfg.table ())
               in ()
           | _ -> ())
         rows);
  (* Delete the parent edge row *)
  let edge_sk = Printf.sprintf "has_sensor#%s" id_s in
  let _ =
    Dyn.DeleteItem.request cfg.ctx
      (Dyn.make_delete_item_input
         ~key:[ ("pk", s (Node_id.to_string parent)); ("sk", s edge_sk) ]
         ~table_name:cfg.table ())
  in
  ()
```

Extend the `effc` block in `run`:

```ocaml
          | Effects.Put_sensor { sensor; parent } ->
              put_sensor_active cfg sensor;
              put_sensor_edge cfg ~parent ~sensor_id:sensor.Sensor.id;
              Some (fun k -> continue k ())
          | Effects.Get_active_sensor id ->
              Some (fun k -> continue k (query_active_sensor cfg id))
          | Effects.List_sensor_ids parent ->
              Some (fun k -> continue k (query_sensor_ids cfg parent))
          | Effects.Replace_sensor_device { old_active_from; new_sensor } ->
              transact_replace cfg ~old_active_from ~new_sensor;
              Some (fun k -> continue k ())
          | Effects.Delete_sensor { sensor_id; parent } ->
              delete_sensor cfg sensor_id parent;
              Some (fun k -> continue k ())
          | Effects.Get_sensor_reading _ ->
              (* raw readings come from the flink-optimized table, not this one *)
              Some (fun k -> continue k None)
```

- [ ] **Step 4: Build to verify it compiles**

Run: `dune build`
Expected: success.

- [ ] **Step 5: Commit**

```bash
git add lib/repo/dynamo.ml
git commit -m "feat(repo/dynamo): sensor ops with TransactWriteItems-backed replace"
```

---

## Phase 5 — API and integration test

### Task 18: API command + query endpoints for sensors

**Files:**
- Modify: `lib/api/api_command.ml`
- Modify: `lib/api/api_query.ml`
- Modify: `lib/api/api_json.ml`
- Modify: `test/test_api_command.ml`
- Modify: `test/test_api_query.ml`

- [ ] **Step 1: Write the failing test**

Append to `test/test_api_command.ml`:

```ocaml
let attach_sensor_happy () =
  (* Tests that follow the existing seed pattern for a company with an electricity slot *)
  let st = Memory.empty () in
  let c2 =
    Memory.run st (fun () ->
      let u = Uuidm.of_string "44444444-0000-4000-8000-000000000001" |> Option.get in
      let sch : Schema.t =
        Schema.{
          version = 1;
          edges = [
            (Level.Hn2, [ (Level.Hn3, [ { label = "building"; min = None; max = None } ]) ]);
          ];
          metadata = [];
          sensors = [
            (Level.Hn3, [
              Sensor_slot.{ kind = "electricity"; min = None; max = None;
                            meter_type = Either;
                            purposes = Some [ "Electricity" ] };
            ]);
          ];
        }
      in
      let n2 = Node.make ~uuid:u ~level:Level.Hn2 ~name:"Co"
                 ~parent:Node_id.root ~created:Ptime.epoch
                 ~metadata:(`Assoc []) ~schema:(Some sch) in
      Effects.put_node n2;
      Node_id.make Level.Hn2 u)
  in
  let bldg =
    Memory.run st (fun () ->
      match Hierarchy.add_node ~parent:c2 ~level:Level.Hn3
              ~name:"B" ~metadata:(`Assoc []) () with
      | Ok b -> b.Node.id
      | Error e -> Alcotest.failf "seed: %s" (Errors.message e))
  in
  Memory.run st (fun () ->
    let body =
      Printf.sprintf
        {|{"action":"attach_sensor","parent_id":%S,"kind":"electricity","daq_address":"daq:1","purpose":"Electricity","meter_type":"counter","unit":"kWh"}|}
        (Node_id.to_string bldg)
    in
    let resp = Api_command.dispatch ~body in
    Alcotest.(check int) "200" 200 resp.status_code)
```

Append to `test/test_api_query.ml`:

```ocaml
let list_sensors_empty () =
  let st = Memory.empty () in
  let parent =
    Node_id.make Level.Hn4
      (Uuidm.of_string "55555555-0000-4000-8000-000000000001" |> Option.get)
  in
  Memory.run st (fun () ->
    let resp =
      Api_query.dispatch ~action:"list_sensors"
        ~params:[ ("parent", Node_id.to_string parent) ]
    in
    Alcotest.(check int) "200" 200 resp.status_code)
```

Add the two new cases to the `tests` lists in each file.

- [ ] **Step 2: Run test to verify it fails**

Run: `dune test`
Expected: build failure — unknown actions `attach_sensor` / `list_sensors` or JSON helpers missing.

- [ ] **Step 3: Implement endpoints**

In `lib/api/api_json.ml`, add a sensor serializer:

```ocaml
let sensor_to_json (s : Sensor.t) : Yojson.Safe.t =
  let unit_json = match s.unit with Some u -> `String u | None -> `Null in
  `Assoc [
    ("id",             `String (Sensor_id.to_string s.id));
    ("active_from",    `String (Ptime.to_rfc3339 ~tz_offset_s:0 s.active_from));
    ("parent",         `String (Node_id.to_string s.parent));
    ("daq_address",    `String s.daq_address);
    ("hierarchy_path", `String s.hierarchy_path);
    ("purpose",        `String s.purpose);
    ("meter_type",     `String (Sensor.meter_type_to_string s.meter_type));
    ("unit",           unit_json);
  ]
```

In `lib/api/api_command.ml`, add the `attach_sensor` handler and extend `dispatch`:

```ocaml
let run_attach_sensor json =
  let* parent_s = require_string json "parent_id" in
  let* kind     = require_string json "kind"      in
  let* daq      = require_string json "daq_address" in
  let* purpose  = require_string json "purpose"   in
  let* mt_s     = require_string json "meter_type" in
  let unit =
    match field json "unit" with
    | Some (`String u) -> Some u
    | _ -> None
  in
  let* parent     = Node_id.of_string parent_s in
  let* meter_type = Sensor.meter_type_of_string mt_s in
  match Sensors.attach ~parent ~kind ~daq_address:daq ~purpose ~meter_type ?unit () with
  | Ok s -> Ok (Api_json.ok_response (Api_json.sensor_to_json s))
  | Error e -> Ok (Api_json.error_response e)

let run_replace_sensor_device json =
  let* id_s = require_string json "sensor_id" in
  let* daq  = require_string json "daq_address" in
  let* id   = Sensor_id.of_string id_s in
  match Sensors.replace_device ~sensor_id:id ~new_daq_address:daq () with
  | Ok s -> Ok (Api_json.ok_response (Api_json.sensor_to_json s))
  | Error e -> Ok (Api_json.error_response e)
```

In `dispatch`, add two more arms:

```ocaml
       | Ok "attach_sensor" ->
           (match run_attach_sensor json with
            | Ok resp -> resp
            | Error m -> err_bad_request m)
       | Ok "replace_sensor_device" ->
           (match run_replace_sensor_device json with
            | Ok resp -> resp
            | Error m -> err_bad_request m)
```

In `lib/api/api_query.ml`, add `list_sensors` and `get_sensor`:

```ocaml
let list_sensors ~params =
  match param params "parent" with
  | None -> err_bad_request "missing parent"
  | Some pid ->
      (match Node_id.of_string pid with
       | Error e -> err_bad_request e
       | Ok p ->
           (match Sensors.list_active ~parent:p with
            | Error err -> Api_json.error_response err
            | Ok xs ->
                Api_json.ok_response
                  (`Assoc [ ("sensors", `List (List.map Api_json.sensor_to_json xs)) ])))

let get_sensor ~params =
  match param params "id" with
  | None -> err_bad_request "missing id"
  | Some sid ->
      (match Sensor_id.of_string sid with
       | Error e -> err_bad_request e
       | Ok id ->
           (match Sensors.get_active id with
            | Ok s -> Api_json.ok_response (Api_json.sensor_to_json s)
            | Error err -> Api_json.error_response err))
```

And extend the `dispatch` action switch:

```ocaml
  | "list_sensors" -> list_sensors ~params
  | "get_sensor"   -> get_sensor   ~params
```

- [ ] **Step 4: Run test to verify it passes**

Run: `dune test`
Expected: PASS — API tests green.

- [ ] **Step 5: Commit**

```bash
git add lib/api/ test/test_api_command.ml test/test_api_query.ml
git commit -m "feat(api): attach_sensor, replace_sensor_device, list_sensors, get_sensor"
```

---

### Task 19: DynamoDB integration test for sensors

**Files:**
- Modify: `itest/test_dynamo.ml`

- [ ] **Step 1: Write the failing test**

Append to `itest/test_dynamo.ml`, before the final `let () = ...`:

```ocaml
let sensor_schema () : Schema.t =
  Schema.{
    version = 1;
    edges = [
      (Level.Hn2, [ (Level.Hn3, [ { label = "property"; min = None; max = None } ]) ]);
      (Level.Hn3, [ (Level.Hn4, [ { label = "building"; min = None; max = None } ]) ]);
      (Level.Hn4, [ (Level.Hn5, [ { label = "area";     min = None; max = None } ]) ]);
    ];
    metadata = [
      (Level.Hn4, [
        ("lat", Metadata.{ typ = Number { min = Some (-90.); max = Some 90. }; required = true });
      ]);
    ];
    sensors = [
      (Level.Hn4, [
        Sensor_slot.{
          kind = "electricity"; min = None; max = Some 2;
          meter_type = Either;
          purposes = Some [ "Electricity" ];
        };
      ]);
      (Level.Hn5, [
        Sensor_slot.{
          kind = "electricity"; min = None; max = None;
          meter_type = Either; purposes = None;
        };
      ]);
    ];
  }

let fresh_with_sensor_schema cfg =
  Dynamo.run cfg (fun () ->
    let u = Effects.gen_uuid () in
    let c2 = Node_id.make Level.Hn2 u in
    let n2 =
      Node.make ~uuid:u ~level:Level.Hn2 ~name:"SensorCo"
        ~parent:Node_id.root ~created:(Ptime_clock.now ())
        ~metadata:(`Assoc []) ~schema:(Some (sensor_schema ()))
    in
    Effects.put_node n2;
    c2)

let attach_list_replace cfg =
  let c2 = fresh_with_sensor_schema cfg in
  Dynamo.run cfg (fun () ->
    let bail tag e = Alcotest.failf "%s: %s" tag (Errors.message e) in
    let prop =
      match Hierarchy.add_node ~parent:c2 ~level:Level.Hn3
              ~name:"P" ~metadata:(`Assoc []) () with
      | Ok n -> n | Error e -> bail "add prop" e
    in
    let bldg =
      match Hierarchy.add_node ~parent:prop.Node.id ~level:Level.Hn4
              ~name:"B" ~metadata:(`Assoc [ ("lat", `Float 55.) ]) () with
      | Ok n -> n | Error e -> bail "add bldg" e
    in
    let s =
      match Sensors.attach ~parent:bldg.Node.id ~kind:"electricity"
              ~daq_address:"daq:itest:old" ~purpose:"Electricity"
              ~meter_type:Sensor.Counter ~unit:"kWh" () with
      | Ok s -> s | Error e -> bail "attach" e
    in
    (match Sensors.list_active ~parent:bldg.Node.id with
     | Ok xs -> Alcotest.(check int) "one sensor attached" 1 (List.length xs)
     | Error e -> bail "list" e);
    (match Sensors.replace_device ~sensor_id:s.Sensor.id
             ~new_daq_address:"daq:itest:new" () with
     | Ok s2 ->
         Alcotest.(check string) "new daq active"
           "daq:itest:new" s2.Sensor.daq_address
     | Error e -> bail "replace" e);
    (match Sensors.get_active s.Sensor.id with
     | Ok s3 ->
         Alcotest.(check string) "read back new"
           "daq:itest:new" s3.Sensor.daq_address
     | Error e -> bail "get" e);
    (* teardown sensor and tree *)
    Effects.delete_sensor ~sensor_id:s.Sensor.id ~parent:bldg.Node.id;
    Effects.delete_node c2)
```

Register the test in the `Alcotest.run` block:

```ocaml
      ("sensors", [
        Alcotest.test_case "attach + list + replace" `Quick (fun () -> attach_list_replace cfg);
      ]);
```

- [ ] **Step 2: Run test to verify it fails**

Run: `AWS_DEFAULT_REGION=<your region> ITEST_DYNAMO_TABLE=<your table> dune exec itest/test_dynamo.exe`
Expected: either compile failure (if Sensors name is unimported) or a test failure if an implementation gap exists. Fix imports as needed.

- [ ] **Step 3: Verify clean run**

Run the same command again after any missed `open` lines are added.
Expected: PASS — sensor integration test green, existing hierarchy tests still pass.

- [ ] **Step 4: (covered by step 3)**

- [ ] **Step 5: Commit**

```bash
git add itest/test_dynamo.ml
git commit -m "test(itest): end-to-end sensor attach + list + replace against DynamoDB"
```

---

## Notes for the implementer

- **TDD discipline:** every task starts with a failing test. Do not edit `.ml` files until the test file references the to-be-created symbol and `dune test` reports the expected failure.
- **Effect dispatch gotcha:** if a task wires a new effect and the test handler isn't extended, `Effect.Deep.try_with` will raise `Effect.Unhandled` at runtime — not a compile error. If tests pass a different effect handler (e.g. `run_with_readings` in Task 15), make sure earlier effects still fall through to the inner `Memory.run`.
- **Schema positional fields:** Task 7 touches every call site that builds a `Schema.t` record literal. The grep list in that task is exhaustive; if you see a build error mentioning `Schema.{`, add `sensors = [];` there.
- **DynamoDB smaws API drift:** the exact function names (`Dyn.make_transact_write_items_input`, `Dyn.make_put`, `Dyn.make_delete`) come from the `smaws-clients` version pinned by the project. If an API call in Task 17 fails to typecheck, check `_opam/lib/smaws-clients/Smaws_Client_DynamoDB.mli` for the current signature and adapt.
- **No CLI dependency:** none of these tasks assume a dev server or browser; `dune test` and `dune exec itest/test_dynamo.exe` are the only commands.
