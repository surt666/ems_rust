# Sensor Formula Input Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a user specify a sensor's `formula` when adding a sensor (default `Identity`), including cross-sensor references entered as a text expression with alias→sensor binding, built in a separate dialog.

**Architecture:** All work is in `services/hierarchy` (OCaml onion architecture: `domain` → `effects` → `repo` handlers → `logic` → `api`). A new pure text parser produces the existing `Formula.expr` AST; a new `List_sensors_under_path` effect enumerates the HN2-company subtree (gsi1 projects ALL) to populate the reference picker; the API gains formula parse/serialize; the htmx server-rendered add-sensor form gets a compact formula control plus a separate formula-builder dialog.

**Tech Stack:** OCaml, `dune`, Alcotest, OCaml effects (`Effect.Deep`), Yojson, Pure_html (server-rendered HTML), htmx + hyperscript.

**Spec:** `docs/superpowers/specs/2026-06-06-sensor-formula-input-design.md`

**Conventions:**
- Build: `dune build` (run from `services/hierarchy`).
- Test (all): `dune runtest` (from `services/hierarchy`).
- A step that adds a test references a not-yet-defined function → the test executable fails to **compile**; that is the "red" state for those steps.
- Commit after each task.

---

### Task 1: Formula AST helpers — `expr_aliases` and `to_string`

**Files:**
- Modify: `services/hierarchy/lib/domain/formula.ml`
- Test: `services/hierarchy/test/test_domain_formula.ml`

- [ ] **Step 1: Write the failing tests**

Append these test functions just above `let tests =` in `test/test_domain_formula.ml`:

```ocaml
let expr_aliases_distinct_in_order () =
  let open Ocaml_lambda_hierarchy.Formula in
  let e = Abs (Sub (Sub (Self, Ref "a"), Add (Ref "b", Ref "a"))) in
  Alcotest.(check (list string)) "distinct aliases, first-seen order"
    [ "a"; "b" ] (expr_aliases e)

let to_string_roundtrips () =
  let open Ocaml_lambda_hierarchy.Formula in
  let e = Abs (Sub (Sub (Self, Ref "a"), Ref "b")) in
  Alcotest.(check string) "renders with minimal parens"
    "abs(self - a - b)" (expr_to_string e)
```

Add to the `tests` list:

```ocaml
    Alcotest.test_case "expr_aliases distinct/in order" `Quick expr_aliases_distinct_in_order;
    Alcotest.test_case "expr_to_string round-trips"     `Quick to_string_roundtrips;
```

- [ ] **Step 2: Run to verify it fails**

Run: `dune runtest`
Expected: compile error — `Unbound value expr_aliases` / `expr_to_string`.

- [ ] **Step 3: Implement the helpers**

Append to `lib/domain/formula.ml` (after `referenced_ids`):

```ocaml
(* Distinct alias names referenced by an expression, in first-seen order. *)
let expr_aliases (e : expr) : string list =
  let rec go acc = function
    | Num _ | Self -> acc
    | Ref a -> if List.mem a acc then acc else a :: acc
    | Abs e -> go acc e
    | Add (a, b) | Sub (a, b) | Mul (a, b) | Div (a, b) -> go (go acc a) b
  in
  List.rev (go [] e)

(* Render an expression back to source text with minimal parentheses.
   Precedence: + - = 1, * / = 2; all binops left-associative. *)
let expr_to_string (e : expr) : string =
  let wrap outer p s = if outer > p then "(" ^ s ^ ")" else s in
  let rec go prec = function
    | Num n -> Printf.sprintf "%g" n
    | Self -> "self"
    | Ref a -> a
    | Abs e -> "abs(" ^ go 0 e ^ ")"
    | Add (a, b) -> wrap prec 1 (go 1 a ^ " + " ^ go 2 b)
    | Sub (a, b) -> wrap prec 1 (go 1 a ^ " - " ^ go 2 b)
    | Mul (a, b) -> wrap prec 2 (go 2 a ^ " * " ^ go 3 b)
    | Div (a, b) -> wrap prec 2 (go 2 a ^ " / " ^ go 3 b)
  in
  go 0 e

let to_string (f : t) : string =
  match f with
  | Identity -> "identity"
  | Zero -> "zero"
  | Expr { ast; _ } -> expr_to_string ast
```

- [ ] **Step 4: Run to verify it passes**

Run: `dune runtest`
Expected: PASS (domain.formula suite green).

- [ ] **Step 5: Commit**

```bash
git add services/hierarchy/lib/domain/formula.ml services/hierarchy/test/test_domain_formula.ml
git commit -m "feat(formula): add expr_aliases and expr_to_string/to_string helpers"
```

---

### Task 2: Formula text parser

**Files:**
- Create: `services/hierarchy/lib/domain/formula_parser.ml`
- Test: `services/hierarchy/test/test_domain_formula.ml`

> Note: the `domain` library uses dune `(modules ...)` auto-discovery; a new `.ml` in `lib/domain/` is picked up automatically. Confirm by building.

- [ ] **Step 1: Write the failing tests**

Append to `test/test_domain_formula.ml` (above `let tests =`):

```ocaml
let parse_ok s expected =
  match Ocaml_lambda_hierarchy.Formula_parser.parse s with
  | Ok e -> Alcotest.(check string) ("parse " ^ s)
              (Ocaml_lambda_hierarchy.Formula.expr_to_string expected)
              (Ocaml_lambda_hierarchy.Formula.expr_to_string e)
  | Error m -> Alcotest.failf "parse %S failed: %s" s m

let parse_err s =
  match Ocaml_lambda_hierarchy.Formula_parser.parse s with
  | Ok _ -> Alcotest.failf "expected parse error for %S" s
  | Error _ -> ()

let parser_precedence_and_assoc () =
  let open Ocaml_lambda_hierarchy.Formula in
  parse_ok "self - a - b" (Sub (Sub (Self, Ref "a"), Ref "b"));
  parse_ok "self + a * b" (Add (Self, Mul (Ref "a", Ref "b")));
  parse_ok "(self + a) * b" (Mul (Add (Self, Ref "a"), Ref "b"));
  parse_ok "abs(self - a - b)" (Abs (Sub (Sub (Self, Ref "a"), Ref "b")));
  parse_ok "self * -1" (Mul (Self, Sub (Num 0., Num 1.)));
  parse_ok "2.5 * self" (Mul (Num 2.5, Self))

let parser_rejects_garbage () =
  parse_err "";
  parse_err "self +";
  parse_err "abs self";
  parse_err "(self + a";
  parse_err "self # a"
```

Add to the `tests` list:

```ocaml
    Alcotest.test_case "parser precedence/assoc" `Quick parser_precedence_and_assoc;
    Alcotest.test_case "parser rejects garbage"  `Quick parser_rejects_garbage;
```

- [ ] **Step 2: Run to verify it fails**

Run: `dune runtest`
Expected: compile error — `Unbound module Formula_parser`.

- [ ] **Step 3: Implement the parser**

Create `lib/domain/formula_parser.ml`:

```ocaml
(* Recursive-descent parser for sensor formula expressions.
     expr    := term (('+' | '-') term)*
     term    := factor (('*' | '/') factor)*
     factor  := '-' factor | primary
     primary := number | 'self' | ident | 'abs' '(' expr ')' | '(' expr ')'
   `self` and `abs` are reserved; any other identifier becomes a Ref alias. *)

let ( let* ) = Result.bind

type state = { src : string; mutable pos : int }

let peek st = if st.pos < String.length st.src then Some st.src.[st.pos] else None
let advance st = st.pos <- st.pos + 1

let rec skip_ws st =
  match peek st with
  | Some (' ' | '\t' | '\n' | '\r') -> advance st; skip_ws st
  | _ -> ()

let is_digit c = c >= '0' && c <= '9'
let is_ident_start c = (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || c = '_'
let is_ident_char c = is_ident_start c || is_digit c

let parse_number st =
  let start = st.pos in
  let rec go () =
    match peek st with
    | Some c when is_digit c || c = '.' || c = 'e' || c = 'E' -> advance st; go ()
    | Some ('+' | '-') when st.pos > start
                            && (let p = st.src.[st.pos - 1] in p = 'e' || p = 'E') ->
        advance st; go ()
    | _ -> ()
  in
  go ();
  let tok = String.sub st.src start (st.pos - start) in
  match float_of_string_opt tok with
  | Some f -> Ok (Formula.Num f)
  | None -> Error (Printf.sprintf "invalid number %S" tok)

let parse_ident st =
  let start = st.pos in
  let rec go () =
    match peek st with
    | Some c when is_ident_char c -> advance st; go ()
    | _ -> ()
  in
  go ();
  String.sub st.src start (st.pos - start)

let rec parse_expr st =
  let* left = parse_term st in
  parse_expr_tail st left
and parse_expr_tail st left =
  skip_ws st;
  match peek st with
  | Some '+' -> advance st;
      let* r = parse_term st in parse_expr_tail st (Formula.Add (left, r))
  | Some '-' -> advance st;
      let* r = parse_term st in parse_expr_tail st (Formula.Sub (left, r))
  | _ -> Ok left
and parse_term st =
  let* left = parse_factor st in
  parse_term_tail st left
and parse_term_tail st left =
  skip_ws st;
  match peek st with
  | Some '*' -> advance st;
      let* r = parse_factor st in parse_term_tail st (Formula.Mul (left, r))
  | Some '/' -> advance st;
      let* r = parse_factor st in parse_term_tail st (Formula.Div (left, r))
  | _ -> Ok left
and parse_factor st =
  skip_ws st;
  match peek st with
  | Some '-' -> advance st;
      let* e = parse_factor st in Ok (Formula.Sub (Formula.Num 0., e))
  | _ -> parse_primary st
and parse_primary st =
  skip_ws st;
  match peek st with
  | None -> Error "unexpected end of expression"
  | Some '(' ->
      advance st;
      let* e = parse_expr st in
      skip_ws st;
      (match peek st with
       | Some ')' -> advance st; Ok e
       | _ -> Error "expected ')'")
  | Some c when is_digit c || c = '.' -> parse_number st
  | Some c when is_ident_start c ->
      (match parse_ident st with
       | "self" -> Ok Formula.Self
       | "abs" ->
           skip_ws st;
           (match peek st with
            | Some '(' ->
                advance st;
                let* e = parse_expr st in
                skip_ws st;
                (match peek st with
                 | Some ')' -> advance st; Ok (Formula.Abs e)
                 | _ -> Error "expected ')' after abs(")
            | _ -> Error "expected '(' after abs")
       | name -> Ok (Formula.Ref name))
  | Some c -> Error (Printf.sprintf "unexpected character %C" c)

let parse (s : string) : (Formula.expr, string) result =
  let st = { src = s; pos = 0 } in
  let* e = parse_expr st in
  skip_ws st;
  if st.pos < String.length st.src then
    Error (Printf.sprintf "unexpected trailing input near position %d" st.pos)
  else Ok e
```

- [ ] **Step 4: Run to verify it passes**

Run: `dune runtest`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add services/hierarchy/lib/domain/formula_parser.ml services/hierarchy/test/test_domain_formula.ml
git commit -m "feat(formula): add recursive-descent text expression parser"
```

---

### Task 3: `List_sensors_under_path` effect + repo handlers

**Files:**
- Modify: `services/hierarchy/lib/effects.ml`
- Modify: `services/hierarchy/lib/repo/memory.ml`
- Modify: `services/hierarchy/lib/repo/dynamo.ml`
- Test: `services/hierarchy/test/test_repo_memory.ml`

- [ ] **Step 1: Write the failing test**

Append to `test/test_repo_memory.ml` (above its `let tests =`):

```ocaml
let list_sensors_under_path_filters_by_prefix () =
  let st = Memory.empty () in
  (* Two sensors under company HN2#200, one under a different company HN2#999. *)
  let mk id path =
    Sensor.{
      id = Sensor_id.make id;
      created = Ptime.epoch;
      daq_id = Printf.sprintf "daq:%d" id;
      path;
      purpose = "Electricity";
      meter_type = Sensor.Counter;
      unit = Some "kWh";
      formula = Formula.Identity;
      resample_minutes = None;
    }
  in
  let in1 = mk 1 "HN0#root|HN1#10|HN2#200|HN3#1|S#1" in
  let in2 = mk 2 "HN0#root|HN1#10|HN2#200|HN3#2|S#2" in
  let other = mk 3 "HN0#root|HN1#10|HN2#999|HN3#3|S#3" in
  List.iter
    (fun (s : Sensor.t) ->
      Memory.run st (fun () -> ignore (Effects.add_sensor ~build:(fun ~id:_ ->
        (s, { Effects.from_ = "HN0#root"; to_ = Sensor_id.to_string s.Sensor.id;
              kind = Edge_kind.Has_sensor; name = ""; created = Ptime.epoch;
              self_path = Some s.Sensor.path })))))
    [ in1; in2; other ];
  let got =
    Memory.run st (fun () ->
      Effects.list_sensors_under_path "HN0#root|HN1#10|HN2#200|")
  in
  let ids = List.map (fun (s : Sensor.t) -> Sensor_id.id s.Sensor.id) got |> List.sort compare in
  Alcotest.(check (list int)) "only the two company sensors" [ 1; 2 ] ids
```

Add to that file's `tests` list:

```ocaml
    Alcotest.test_case "list_sensors_under_path filters by prefix" `Quick list_sensors_under_path_filters_by_prefix;
```

> Note: `Add_sensor` in the memory handler ignores the `~id` from `build` and allocates its own; the test only checks `path`, so the returned record's `path` is what matters. The allocated id replaces `s.id` in storage keying but the stored value is the record returned by `build`, so `Sensor_id.id` in assertions reflects the seeded ids. If the memory handler keys by the allocated id, adjust the assertion to compare on `path` instead:
> ```ocaml
> let paths = List.map (fun (s:Sensor.t) -> s.Sensor.path) got |> List.sort compare in
> Alcotest.(check int) "two sensors under company" 2 (List.length paths)
> ```
> Use the `path`-count form to stay robust to id allocation.

Replace the assertion block with the robust path-count form:

```ocaml
  let got =
    Memory.run st (fun () ->
      Effects.list_sensors_under_path "HN0#root|HN1#10|HN2#200|")
  in
  Alcotest.(check int) "two sensors under company HN2#200" 2 (List.length got);
  Alcotest.(check bool) "excludes other company" true
    (not (List.exists (fun (s : Sensor.t) ->
       s.Sensor.path = "HN0#root|HN1#10|HN2#999|HN3#3|S#3") got))
```

- [ ] **Step 2: Run to verify it fails**

Run: `dune runtest`
Expected: compile error — `Unbound value Effects.list_sensors_under_path`.

- [ ] **Step 3a: Declare the effect**

In `lib/effects.ml`, inside the sensor effects `type _ Effect.t +=` block (the one starting at `Add_sensor`), add a constructor after `Get_sensor_reading`:

```ocaml
  | List_sensors_under_path : string -> Sensor.t list Effect.t
```

And add the performer near `let list_sensor_ids`:

```ocaml
let list_sensors_under_path prefix =
  Effect.perform (List_sensors_under_path prefix)
```

- [ ] **Step 3b: Handle it in the memory repo**

In `lib/repo/memory.ml`, inside `run`'s `effc` match, add a case (next to `List_sensor_ids`):

```ocaml
          | Effects.List_sensors_under_path prefix ->
              let acc =
                Hashtbl.fold
                  (fun _ rows acc ->
                    match active_of_rows rows with
                    | Some s when String.starts_with ~prefix s.Sensor.path -> s :: acc
                    | _ -> acc)
                  st.sensors []
              in
              Some (fun k -> continue k acc)
```

- [ ] **Step 3c: Handle it in the dynamo repo**

In `lib/repo/dynamo.ml`, add this helper after `query_sensor_ids` (~line 507):

```ocaml
(* All active sensors whose gsi1sk (= path) begins with [path_prefix].
   gsi1 projects ALL, so items decode directly. *)
let query_sensors_under_path cfg ~path_prefix =
  let rows = query_gsi_partition cfg ~gsi1pk_v:Codec.sensor_gsi1pk ~path_prefix in
  List.filter_map
    (fun kvs ->
      match (List.assoc_opt "sk" kvs : Dyn.attribute_value option) with
      | Some (Dyn.S sk) when String.starts_with ~prefix:active_sk_prefix sk ->
          (match Codec.sensor_of_item kvs with Ok s -> Some s | Error _ -> None)
      | _ -> None)
    rows
```

In `dynamo.ml`'s `run` `effc` match, add a case next to `List_sensor_ids`:

```ocaml
          | Effects.List_sensors_under_path prefix ->
              Some (fun (k : (a, _) continuation) ->
                continue k (query_sensors_under_path cfg ~path_prefix:prefix))
```

- [ ] **Step 4: Run to verify it passes**

Run: `dune runtest`
Expected: PASS (repo.memory suite green; whole build compiles, so dynamo case typechecks too).

- [ ] **Step 5: Commit**

```bash
git add services/hierarchy/lib/effects.ml services/hierarchy/lib/repo/memory.ml services/hierarchy/lib/repo/dynamo.ml services/hierarchy/test/test_repo_memory.ml
git commit -m "feat(effects): add List_sensors_under_path effect + memory/dynamo handlers"
```

---

### Task 4: `Sensors.list_under_company`

**Files:**
- Modify: `services/hierarchy/lib/logic/sensors.ml`
- Test: `services/hierarchy/test/test_logic_sensors.ml`

- [ ] **Step 1: Write the failing test**

Append to `test/test_logic_sensors.ml` (above its `let tests =`). This seeds a company subtree via the existing helpers used in that file (it already builds nodes with schemas; mirror the existing `bldg` seeding pattern at the top of the file). Minimal version using the memory repo directly:

```ocaml
let list_under_company_scopes_to_hn2 () =
  let st = Memory.empty () in
  let seed (s : Sensor.t) =
    Memory.run st (fun () -> ignore (Effects.add_sensor ~build:(fun ~id:_ ->
      (s, { Effects.from_ = "HN0#root"; to_ = Sensor_id.to_string s.Sensor.id;
            kind = Edge_kind.Has_sensor; name = ""; created = Ptime.epoch;
            self_path = Some s.Sensor.path }))))
  in
  let mk id path = Sensor.{
    id = Sensor_id.make id; created = Ptime.epoch; daq_id = Printf.sprintf "d%d" id;
    path; purpose = "E"; meter_type = Sensor.Counter; unit = None;
    formula = Formula.Identity; resample_minutes = None } in
  seed (mk 1 "HN0#root|HN1#10|HN2#200|HN3#1|S#1");
  seed (mk 2 "HN0#root|HN1#10|HN2#999|HN3#9|S#2");
  (* Parent node HN3#1 under company HN2#200. *)
  let parent = Node_id.make Level.Hn3 1 in
  Memory.run st (fun () ->
    Effects.put_node (Node.make ~id:1 ~level:Level.Hn3 ~name:"b"
      ~parent:(Node_id.make Level.Hn2 200)
      ~parent_path:"HN0#root|HN1#10|HN2#200" ~created:Ptime.epoch
      ~metadata:(`Assoc []) ~schema:None));
  let got =
    Memory.run st (fun () ->
      match Sensors.list_under_company ~parent with Ok xs -> xs | Error _ -> [])
  in
  Alcotest.(check int) "only company HN2#200 sensors" 1 (List.length got)
```

Add to the `tests` list:

```ocaml
    Alcotest.test_case "list_under_company scopes to HN2" `Quick list_under_company_scopes_to_hn2;
```

- [ ] **Step 2: Run to verify it fails**

Run: `dune runtest`
Expected: compile error — `Unbound value Sensors.list_under_company`.

- [ ] **Step 3: Implement**

Append to `lib/logic/sensors.ml`:

```ocaml
(* Prefix of [path] up to and including its HN2 (company) segment, with a
   trailing path separator so "HN2#1|" does not also match "HN2#10|...".
   None when the path has no HN2 ancestor. *)
let company_prefix_of_path path =
  let segs = String.split_on_char '|' path in
  let rec take acc = function
    | [] -> None
    | seg :: rest ->
        let acc = seg :: acc in
        (match Node_id.of_string seg with
         | Ok nid when Node_id.level nid = Level.Hn2 ->
             Some (String.concat "|" (List.rev acc) ^ "|")
         | _ -> take acc rest)
  in
  take [] segs

(* Active sensors in the same HN2 company subtree as [parent]. Used to populate
   the formula reference picker. *)
let list_under_company ~parent =
  match Effects.get_node parent with
  | None -> Error (Errors.Not_found parent)
  | Some n ->
      (match company_prefix_of_path n.Node.path with
       | None -> Ok []
       | Some prefix -> Ok (Effects.list_sensors_under_path prefix))
```

- [ ] **Step 4: Run to verify it passes**

Run: `dune runtest`
Expected: PASS (logic.sensors suite green).

- [ ] **Step 5: Commit**

```bash
git add services/hierarchy/lib/logic/sensors.ml services/hierarchy/test/test_logic_sensors.ml
git commit -m "feat(sensors): list_under_company enumerates the HN2 company subtree"
```

---

### Task 5: API formula codec — `formula_of_json` + `sensor_to_json`

**Files:**
- Modify: `services/hierarchy/lib/api/api_json.ml`
- Test: `services/hierarchy/test/test_api_json.ml`

- [ ] **Step 1: Write the failing tests**

Append to `test/test_api_json.ml` (above its `let tests =`):

```ocaml
let formula_of_json_default_identity () =
  match Api_json.formula_of_json None with
  | Ok Formula.Identity -> ()
  | _ -> Alcotest.fail "absent formula should be Identity"

let formula_of_json_zero () =
  match Api_json.formula_of_json (Some (`Assoc [ ("kind", `String "zero") ])) with
  | Ok Formula.Zero -> ()
  | _ -> Alcotest.fail "kind=zero should be Zero"

let formula_of_json_expr_with_refs () =
  let j =
    `Assoc [ ("kind", `String "expr");
             ("expr", `String "abs(self - a)");
             ("refs", `Assoc [ ("a", `String "S#12") ]) ]
  in
  match Api_json.formula_of_json (Some j) with
  | Ok (Formula.Expr { ast; refs }) ->
      Alcotest.(check string) "ast" "abs(self - a)" (Formula.expr_to_string ast);
      Alcotest.(check int) "one ref" 1 (List.length refs)
  | _ -> Alcotest.fail "should parse expr"

let formula_of_json_refs_as_string () =
  let j =
    `Assoc [ ("kind", `String "expr");
             ("expr", `String "self - a");
             ("refs", `String {|{"a":"S#7"}|}) ]
  in
  match Api_json.formula_of_json (Some j) with
  | Ok (Formula.Expr _) -> ()
  | _ -> Alcotest.fail "refs as JSON string should parse"

let formula_of_json_unbound_alias_errors () =
  let j =
    `Assoc [ ("kind", `String "expr");
             ("expr", `String "self - a");
             ("refs", `Assoc []) ]
  in
  match Api_json.formula_of_json (Some j) with
  | Error _ -> ()
  | Ok _ -> Alcotest.fail "unbound alias must error"

let sensor_to_json_includes_formula () =
  let s = Sensor.{
    id = Sensor_id.make 5; created = Ptime.epoch; daq_id = "d"; path = "HN0#root|S#5";
    purpose = "E"; meter_type = Sensor.Counter; unit = None;
    formula = Formula.Zero; resample_minutes = None }
  in
  let j = Api_json.sensor_to_json s in
  let kind = Yojson.Safe.Util.(j |> member "formula" |> member "kind" |> to_string) in
  Alcotest.(check string) "formula kind serialized" "zero" kind
```

Add to the `tests` list:

```ocaml
    Alcotest.test_case "formula_of_json default identity" `Quick formula_of_json_default_identity;
    Alcotest.test_case "formula_of_json zero"             `Quick formula_of_json_zero;
    Alcotest.test_case "formula_of_json expr+refs"        `Quick formula_of_json_expr_with_refs;
    Alcotest.test_case "formula_of_json refs as string"   `Quick formula_of_json_refs_as_string;
    Alcotest.test_case "formula_of_json unbound alias"    `Quick formula_of_json_unbound_alias_errors;
    Alcotest.test_case "sensor_to_json includes formula"  `Quick sensor_to_json_includes_formula;
```

- [ ] **Step 2: Run to verify it fails**

Run: `dune runtest`
Expected: compile error — `Unbound value Api_json.formula_of_json` (and the `member "formula"` assertion would fail once it compiles).

- [ ] **Step 3: Implement**

In `lib/api/api_json.ml`, add before `sensor_to_json`:

```ocaml
let formula_to_json (f : Formula.t) : Yojson.Safe.t =
  match f with
  | Formula.Identity -> `Assoc [ ("kind", `String "identity") ]
  | Formula.Zero -> `Assoc [ ("kind", `String "zero") ]
  | Formula.Expr { ast; refs } ->
      `Assoc [
        ("kind", `String "expr");
        ("expr", `String (Formula.expr_to_string ast));
        ("refs", `Assoc (List.map
                  (fun (a, id) -> (a, `String (Sensor_id.to_string id))) refs));
      ]

let parse_refs_json (v : Yojson.Safe.t) : ((string * Sensor_id.t) list, string) result =
  let* kvs =
    match v with
    | `Assoc kvs -> Ok kvs
    | `Null -> Ok []
    | `String s ->
        (match Yojson.Safe.from_string s with
         | `Assoc kvs -> Ok kvs
         | _ -> Error "formula.refs string must encode a JSON object"
         | exception _ -> Error "formula.refs is not valid JSON")
    | _ -> Error "formula.refs must be an object"
  in
  List.fold_left
    (fun acc (alias, idv) ->
      let* acc = acc in
      match idv with
      | `String s ->
          (match Sensor_id.of_string s with
           | Ok id -> Ok ((alias, id) :: acc)
           | Error e -> Error (Printf.sprintf "bad sensor id for alias %S: %s" alias e))
      | _ -> Error (Printf.sprintf "ref for alias %S must be a string sensor id" alias))
    (Ok []) kvs
  |> Result.map List.rev

let formula_of_json (v : Yojson.Safe.t option) : (Formula.t, string) result =
  match v with
  | None | Some `Null -> Ok Formula.Identity
  | Some (`String "identity") -> Ok Formula.Identity
  | Some (`String "zero") -> Ok Formula.Zero
  | Some (`Assoc _ as obj) ->
      let kind =
        match Yojson.Safe.Util.member "kind" obj with `String k -> Some k | _ -> None
      in
      let has_expr =
        match Yojson.Safe.Util.member "expr" obj with `String _ -> true | _ -> false
      in
      let want_expr = kind = Some "expr" || (kind = None && has_expr) in
      (match kind with
       | Some "identity" -> Ok Formula.Identity
       | Some "zero" -> Ok Formula.Zero
       | _ when want_expr ->
           let* expr_s =
             match Yojson.Safe.Util.member "expr" obj with
             | `String s -> Ok s
             | _ -> Error "formula.expr must be a string"
           in
           let* ast = Formula_parser.parse expr_s in
           let* refs = parse_refs_json (Yojson.Safe.Util.member "refs" obj) in
           let aliases = Formula.expr_aliases ast in
           (match List.find_opt (fun a -> not (List.mem_assoc a refs)) aliases with
            | Some a -> Error (Printf.sprintf "formula references unbound alias %S" a)
            | None -> Ok (Formula.Expr { ast; refs }))
       | _ -> Error "unknown or incomplete formula object")
  | Some _ -> Error "formula must be an object or string"
```

> The `let* ` operator (`Result.bind`) is already defined at the top of `api_json.ml`.

In `sensor_to_json`, add a `formula` field to the assoc list (after `resample_minutes`):

```ocaml
    ("resample_minutes", resample_json);
    ("formula", formula_to_json s.formula);
```

- [ ] **Step 4: Run to verify it passes**

Run: `dune runtest`
Expected: PASS (api.json suite green).

- [ ] **Step 5: Commit**

```bash
git add services/hierarchy/lib/api/api_json.ml services/hierarchy/test/test_api_json.ml
git commit -m "feat(api): formula_of_json/formula_to_json + formula in sensor_to_json"
```

---

### Task 6: Accept formula in `attach_sensor`

**Files:**
- Modify: `services/hierarchy/lib/api/api_command.ml:59-84`
- Test: `services/hierarchy/test/test_api_command.ml`

- [ ] **Step 1: Add a shared `seed_building` helper**

Add this helper near the top of `test/test_api_command.ml` (below the existing `field`/`require` style helpers, above the first test). It creates an HN2 company (`HN2#10002`, path `HN0#root|HN1#10001|HN2#10002`) with a schema that allows sensors at HN3, plus an HN3 building, and returns the building `Node_id`. It is the same seed the existing `attach_sensor_happy` inlines:

```ocaml
let seed_building st =
  let c2 =
    Memory.run st (fun () ->
      let sch : Schema.t =
        Schema.{
          version = 1;
          edges = [
            (Level.Hn2, [ (Level.Hn3, [ { label = "building"; min = None; max = None } ]) ]);
          ];
          metadata = [];
          sensors = [ Level.Hn3 ];
        }
      in
      let parent_path =
        Node_id.to_string Node_id.root ^ "|"
        ^ Node_id.to_string (Node_id.make Level.Hn1 10001)
      in
      let n2 = Node.make ~id:10002 ~level:Level.Hn2 ~name:"Co"
                 ~parent:Node_id.root ~parent_path ~created:Ptime.epoch
                 ~metadata:(`Assoc []) ~schema:(Some sch) in
      Effects.put_node n2;
      Node_id.make Level.Hn2 10002)
  in
  Memory.run st (fun () ->
    match Hierarchy.add_node ~parent:c2 ~level:Level.Hn3
            ~name:"B" ~metadata:(`Assoc []) () with
    | Ok b -> b.Node.id
    | Error e -> Alcotest.failf "seed: %s" (Errors.message e))
```

- [ ] **Step 2: Write the failing tests**

Append to `test/test_api_command.ml` (above its `let tests =`):

```ocaml
let attach_sensor_with_formula () =
  let st = Memory.empty () in
  let bldg = seed_building st in
  Memory.run st (fun () ->
    let body =
      Printf.sprintf
        {|{"action":"attach_sensor","parent_id":%S,"daq_id":"daq:f","purpose":"E","meter_type":"counter","formula":{"kind":"expr","expr":"abs(self - a)","refs":{"a":"S#1"}}}|}
        (Node_id.to_string bldg)
    in
    let resp = Api_command.dispatch ~body in
    let status = Yojson.Safe.Util.(Yojson.Safe.from_string resp |> member "statusCode" |> to_int) in
    Alcotest.(check int) "200" 200 status;
    let kind =
      Yojson.Safe.Util.(
        Yojson.Safe.from_string resp |> member "body" |> to_string
        |> Yojson.Safe.from_string |> member "formula" |> member "kind" |> to_string)
    in
    Alcotest.(check string) "formula echoed as expr" "expr" kind)

let attach_sensor_unbound_alias_400 () =
  let st = Memory.empty () in
  let bldg = seed_building st in
  Memory.run st (fun () ->
    let body =
      Printf.sprintf
        {|{"action":"attach_sensor","parent_id":%S,"daq_id":"daq:bad","purpose":"E","meter_type":"counter","formula":{"kind":"expr","expr":"self - a","refs":{}}}|}
        (Node_id.to_string bldg)
    in
    let resp = Api_command.dispatch ~body in
    let status = Yojson.Safe.Util.(Yojson.Safe.from_string resp |> member "statusCode" |> to_int) in
    Alcotest.(check int) "400 on unbound alias" 400 status)
```

Add to the `tests` list:

```ocaml
    Alcotest.test_case "attach_sensor with formula"        `Quick attach_sensor_with_formula;
    Alcotest.test_case "attach_sensor unbound alias -> 400" `Quick attach_sensor_unbound_alias_400;
```

- [ ] **Step 3: Run to verify it fails**

Run: `dune runtest`
Expected: api.command suite fails — formula ignored, response `formula.kind` is `"identity"` (mismatch), and the unbound-alias case returns 200 instead of 400.

- [ ] **Step 4: Implement**

In `lib/api/api_command.ml`, in `run_attach_sensor`, after the `resample_minutes` block and before `let* parent = Node_id.of_string parent_s in`, add:

```ocaml
  let* formula = Api_json.formula_of_json (field json "formula") in
```

Then thread it into the attach call — change:

```ocaml
  match Sensors.attach ~parent ~daq_id:daq ~purpose ~meter_type ?resample_minutes
          ?unit () with
```

to:

```ocaml
  match Sensors.attach ~parent ~daq_id:daq ~purpose ~meter_type ?resample_minutes
          ~formula ?unit () with
```

(`Sensors.attach`'s `?(formula = Formula.Identity)` accepts `~formula`; passing the parsed value — which defaults to `Identity` when absent — preserves current behavior.)

- [ ] **Step 5: Run to verify it passes**

Run: `dune runtest`
Expected: PASS (api.command green; existing attach tests still pass because absent formula → Identity).

- [ ] **Step 6: Commit**

```bash
git add services/hierarchy/lib/api/api_command.ml services/hierarchy/test/test_api_command.ml
git commit -m "feat(api): attach_sensor accepts an optional formula"
```

---

### Task 7: Company-sensor options endpoint (htmx fragment)

**Files:**
- Modify: `services/hierarchy/lib/api/api_html.ml` (add `render_company_sensors` + route in `dispatch`)
- Test: `services/hierarchy/test/test_api_query.ml` (or a new render test in an existing api test file)

- [ ] **Step 1: Write the failing test**

Append to `test/test_api_query.ml` (it already exercises query dispatch; if it lacks Memory seeding helpers, place this in `test_api_command.ml` instead, reusing `seed_building`). The test seeds a sensor under a company and asserts the rendered fragment contains an `<option>` for it:

```ocaml
let company_sensors_fragment_lists_options () =
  let st = Memory.empty () in
  let bldg = seed_building st in   (* HN3 building under some HN2 company *)
  (* attach one sensor so there is an option to render *)
  Memory.run st (fun () ->
    ignore (Api_command.dispatch ~body:(Printf.sprintf
      {|{"action":"attach_sensor","parent_id":%S,"daq_id":"daq:opt","purpose":"E","meter_type":"counter"}|}
      (Node_id.to_string bldg))));
  let bldg_path =
    Memory.run st (fun () ->
      match Effects.get_node bldg with Some n -> n.Node.path | None -> "")
  in
  let resp =
    Memory.run st (fun () ->
      Api_html.dispatch ~action:"company_sensors" ~params:[ ("nodepath", bldg_path) ])
  in
  let body = Yojson.Safe.Util.(Yojson.Safe.from_string resp |> member "body" |> to_string) in
  Alcotest.(check bool) "fragment mentions the sensor daq id" true
    (let re = Str.regexp_string "daq:opt" in
     try ignore (Str.search_forward re body 0); true with Not_found -> false)
```

> If `Str` is not already a dependency of the test stanza, replace the substring check with a manual scan:
> ```ocaml
> let contains hay needle =
>   let nh = String.length needle and hh = String.length hay in
>   let rec go i = i + nh <= hh && (String.sub hay i nh = needle || go (i + 1)) in
>   nh = 0 || go 0
> in
> Alcotest.(check bool) "fragment mentions daq id" true (contains body "daq:opt")
> ```
> Prefer the manual-scan form to avoid adding a dependency.

Add to that file's `tests` list:

```ocaml
    Alcotest.test_case "company_sensors fragment lists options" `Quick company_sensors_fragment_lists_options;
```

- [ ] **Step 2: Run to verify it fails**

Run: `dune runtest`
Expected: fails — `dispatch ~action:"company_sensors"` falls through to the default "no matching route", body won't contain the daq id.

- [ ] **Step 3: Implement**

In `lib/api/api_html.ml`, add `render_company_sensors` near `render_sensors` (reuse the same `nodepath` → leaf-node-id extraction `render_sensors` uses; factor that extraction into a local `leaf_node_id nodepath` helper if convenient, otherwise duplicate the existing inline logic):

```ocaml
(* /hierarchy/query/company_sensors?nodepath=<path>
   <option> list of active sensors in the parent's HN2 company subtree,
   for the formula reference picker. *)
let render_company_sensors ~params =
  match List.assoc_opt "nodepath" params with
  | None -> respond_error "missing nodepath"
  | Some nodepath ->
      let last =
        (* same leaf-id extraction as render_sensors *)
        match String.rindex_opt nodepath '#' with
        | Some _ ->
            let rec last_node_id i =
              if i < 0 then nodepath
              else match nodepath.[i] with
                | 'H' when i + 1 < String.length nodepath && nodepath.[i + 1] = 'N' ->
                    String.sub nodepath i (String.length nodepath - i)
                | _ -> last_node_id (i - 1)
            in last_node_id (String.length nodepath - 1)
        | None -> nodepath
      in
      (match Node_id.of_string last with
       | Error e -> respond_error e
       | Ok nid ->
           (match Sensors.list_under_company ~parent:nid with
            | Error err -> respond_error ~status:(Errors.http_status err) (Errors.message err)
            | Ok ss ->
                let opts =
                  List.map (fun (s : Sensor.t) ->
                    option [ value "%s" (Sensor_id.to_string s.Sensor.id) ]
                      "%s (%s)" (Sensor_id.to_string s.Sensor.id) s.Sensor.purpose)
                    ss
                in
                respond (null opts)))
```

> `option`, `value`, `null`, `respond`, `respond_error` are already used in this module (see `render_sensors` / `option_nodes`). Match their exact signatures from nearby code; `value "%s" x` is printf-style.

Add the route in `dispatch` (next to `"sensors"`):

```ocaml
  | "company_sensors" -> render_company_sensors ~params
```

- [ ] **Step 4: Run to verify it passes**

Run: `dune runtest`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add services/hierarchy/lib/api/api_html.ml services/hierarchy/test/test_api_query.ml
git commit -m "feat(api-html): company_sensors htmx fragment for formula ref picker"
```

---

### Task 8: Add-sensor form — compact formula control + formula dialog

**Files:**
- Modify: `services/hierarchy/lib/api/api_html.ml` (`sensor_dialog`, ~line 300-369)
- Test: `services/hierarchy/test/test_api_command.ml` (render assertion) or manual

This task is mostly server-rendered HTML + a small inline JS serializer. It is verified by (a) a render test asserting the hidden formula fields and the formula dialog exist, and (b) a manual smoke check.

> **Signature change:** `sensor_dialog ~nid_str` becomes `sensor_dialog ~nid_str ~parent_str` (it needs the node's full path to load company sensors). Update its one caller in `sensor_block` (~line 392) from `sensor_dialog ~nid_str` to `sensor_dialog ~nid_str ~parent_str` (`parent_str` is already in scope there). `sensor_dialog` must now return `null [ <add-sensor dialog>; <formula dialog>; script [...] ]` instead of a single `dialog`, since it emits multiple top-level nodes.
>
> **Loader URL:** this module deliberately avoids putting `HN…#…` paths in htmx URLs (the `#` breaks URI encoding — see the `pct`/`hx_get` comment near the top). Mirror the existing `sensor-list` loader, which passes the path via `Hx.vals` instead of the URL.

- [ ] **Step 1: Write the failing render test**

Append to `test/test_api_command.ml` (or wherever `sensor_dialog` is reachable; `Api_html.render_node` renders it). Simplest: assert the rendered node page for a sensor-allowing node contains the new field names. If `sensor_dialog` is not exported, add a tiny exported wrapper, or assert via `render_node`. Use the manual `contains` helper from Task 7:

```ocaml
let add_sensor_form_has_formula_controls () =
  let st = Memory.empty () in
  let bldg = seed_building st in
  let bldg_path =
    Memory.run st (fun () ->
      match Effects.get_node bldg with Some n -> n.Node.path | None -> "")
  in
  let resp =
    Memory.run st (fun () ->
      Api_html.dispatch ~action:"node" ~params:[ ("nodepath", bldg_path) ])
  in
  let body = Yojson.Safe.Util.(Yojson.Safe.from_string resp |> member "body" |> to_string) in
  let contains hay needle =
    let nh = String.length needle and hh = String.length hay in
    let rec go i = i + nh <= hh && (String.sub hay i nh = needle || go (i + 1)) in
    nh = 0 || go 0
  in
  Alcotest.(check bool) "has formula.kind hidden field" true (contains body "data.formula.kind");
  Alcotest.(check bool) "has formula dialog" true (contains body "formula-dialog")
```

Add to the `tests` list:

```ocaml
    Alcotest.test_case "add-sensor form has formula controls" `Quick add_sensor_form_has_formula_controls;
```

- [ ] **Step 2: Run to verify it fails**

Run: `dune runtest`
Expected: fails — body lacks `data.formula.kind` / `formula-dialog`.

- [ ] **Step 3: Implement the form changes**

In `sensor_dialog` (`lib/api/api_html.ml`), inside the `add-sensor-form` `<form>` children, after the resample-minutes `div [ class_ "form-row" ] [...]`, add the compact formula control + hidden fields:

```ocaml
              ; div [ class_ "form-row" ]
                  [ label [ class_ "form-label" ] [ txt "Formula" ];
                    div [ class_ "form-inline" ]
                      [ span [ id "formula-summary"; class_ "form-summary" ]
                          [ txt "Identity (default)" ];
                        button
                          [ type_ "button"; class_ "btn-secondary";
                            Hx.__ "on click call #formula-dialog.showModal()" ]
                          [ txt "Edit formula…" ] ] ]
                ; input [ type_ "hidden"; name "data.formula.kind"; id "formula-kind"; value "identity" ]
                ; input [ type_ "hidden"; name "data.formula.expr"; id "formula-expr-field"; value "" ]
                ; input [ type_ "hidden"; name "data.formula.refs"; id "formula-refs-field"; value "" ]
```

After the main `add-sensor-dialog` `</dialog>` (i.e., as a sibling node returned by `sensor_dialog`; wrap both in `null [ … ]` if needed), add the formula dialog and its options source. The options `<select>` is loaded via htmx when this fragment loads:

```ocaml
  ; dialog [ id "formula-dialog"; class_ "dialog" ]
      [ div [ class_ "dialog-content" ]
          [ h2 [] [ txt "Build formula" ]
          ; div [ class_ "form-row" ]
              [ label [ class_ "form-label" ] [ txt "Kind" ]
              ; select [ id "formula-kind-select"; class_ "form-select" ]
                  [ option [ value "identity" ] "Identity (default)"
                  ; option [ value "zero" ] "Zero"
                  ; option [ value "expr" ] "Expression" ] ]
          ; div [ id "formula-expr-section"; style_ "display:none;" ]
              [ div [ class_ "form-row" ]
                  [ label [ class_ "form-label" ] [ txt "Expression" ]
                  ; input [ type_ "text"; id "formula-expr-input"; class_ "form-input";
                            string_attr "placeholder" "abs(self - a - b)" ] ]
              ; div [ class_ "form-hint" ]
                  [ txt "Use self, numbers, + − × ÷, abs(), and aliases bound below." ]
              ; div [ id "formula-refs-rows" ] []
              ; button [ type_ "button"; class_ "btn-secondary"; id "formula-add-ref" ]
                  [ txt "+ Add reference" ] ]
          ; div [ id "formula-dialog-error"; class_ "login-error"; style_ "display:none;" ] []
          ; div [ class_ "dialog-footer" ]
              [ button [ type_ "button"; class_ "btn-warning"; id "formula-apply" ] [ txt "Apply" ]
              ; button [ type_ "button"; Hx.__ "on click call #formula-dialog.close()" ] [ txt "Cancel" ] ] ]
      (* Hidden source of <option>s for ref dropdowns; loaded once via htmx.
         nodepath is passed via Hx.vals (not the URL) to avoid '#'-encoding
         issues, mirroring the sensor-list loader in sensor_block. *)
      ; select [ id "ref-sensor-options-src"; style_ "display:none;";
                 Hx.get "/hierarchy/query/company_sensors";
                 Hx.vals {|{"nodepath": "%s"}|} parent_str;
                 Hx.trigger "load"; Hx.target "#ref-sensor-options-src";
                 Hx.swap "innerHTML"; Hx.request {|{"noHeaders": true}|} ]
          [] ]
  ; script [] [ txt ~raw:true "%s" formula_dialog_js ]
```

> Adapt element wrapping to the module's return type: `sensor_dialog` currently returns one node; change it to return `null [ <existing dialog>; <formula dialog>; <script> ]`. `dialog`, `select`, `option`, `script`, `span`, `h2`, `button`, `input`, `div`, `label`, `txt`, `style_`, `string_attr`, `value`, `name`, `id`, `type_`, `class_` are all `Pure_html.HTML` / helpers already used here. `Hx.get`/`Hx.trigger`/`Hx.target`/`Hx.swap`/`Hx.__` are the htmx helpers used elsewhere in this file. `txt ~raw:true "%s" s` renders unescaped (used already at line 309).

Define `formula_dialog_js` as a module-level string constant near the top of `api_html.ml` (vanilla JS — no framework). It wires the kind select, ref rows, and Apply serialization into the main form's hidden fields:

```ocaml
let formula_dialog_js = {js|
(function () {
  function $(id){ return document.getElementById(id); }
  function refreshExprVisibility(){
    var k = $('formula-kind-select').value;
    $('formula-expr-section').style.display = (k === 'expr') ? '' : 'none';
  }
  document.addEventListener('change', function (e) {
    if (e.target && e.target.id === 'formula-kind-select') refreshExprVisibility();
  });
  document.addEventListener('click', function (e) {
    if (!e.target) return;
    if (e.target.id === 'formula-add-ref') {
      var rows = $('formula-refs-rows');
      var src = $('ref-sensor-options-src');
      var row = document.createElement('div');
      row.className = 'form-row formula-ref-row';
      var alias = document.createElement('input');
      alias.type = 'text'; alias.className = 'form-input formula-ref-alias';
      alias.placeholder = 'alias (e.g. a)'; alias.style.maxWidth = '8rem';
      var sel = document.createElement('select');
      sel.className = 'form-select formula-ref-sensor';
      sel.innerHTML = src ? src.innerHTML : '';
      row.appendChild(alias); row.appendChild(document.createTextNode(' → ')); row.appendChild(sel);
      rows.appendChild(row);
    }
    if (e.target.id === 'formula-apply') {
      var err = $('formula-dialog-error');
      err.style.display = 'none'; err.textContent = '';
      var kind = $('formula-kind-select').value;
      $('formula-kind').value = kind;
      if (kind !== 'expr') {
        $('formula-expr-field').value = '';
        $('formula-refs-field').value = '';
        $('formula-summary').textContent = (kind === 'zero') ? 'Zero' : 'Identity (default)';
        $('formula-dialog').close();
        return;
      }
      var expr = ($('formula-expr-input').value || '').trim();
      if (!expr) { err.textContent = 'Expression is required.'; err.style.display = ''; return; }
      var refs = {};
      var rows = document.querySelectorAll('#formula-refs-rows .formula-ref-row');
      for (var i = 0; i < rows.length; i++) {
        var a = rows[i].querySelector('.formula-ref-alias').value.trim();
        var s = rows[i].querySelector('.formula-ref-sensor').value;
        if (a) refs[a] = s;
      }
      $('formula-expr-field').value = expr;
      $('formula-refs-field').value = JSON.stringify(refs);
      $('formula-summary').textContent = expr;
      $('formula-dialog').close();
    }
  });
})();
|js}
```

- [ ] **Step 4: Run to verify the render test passes**

Run: `dune runtest`
Expected: PASS (`data.formula.kind` and `formula-dialog` present).

- [ ] **Step 5: Manual smoke check**

Build and exercise the form against the memory/dev runner if available (or note for QA):
- Open a node that allows sensors → "Add sensor" → the form shows "Formula: Identity (default) · Edit formula…".
- Submitting without touching formula creates an Identity sensor (existing behavior).
- "Edit formula…" → choose Expression → type `abs(self - a)` → Add reference, alias `a`, pick a sensor → Apply → summary shows the expression → submit → the created sensor's JSON has `formula.kind = "expr"`.

- [ ] **Step 6: Commit**

```bash
git add services/hierarchy/lib/api/api_html.ml services/hierarchy/test/test_api_command.ml
git commit -m "feat(api-html): formula control + builder dialog on add-sensor form"
```

---

### Task 9: Docs

**Files:**
- Modify: `docs/api.md`
- Modify: `docs/hierarchy-and-sensors.md`

- [ ] **Step 1: Update the attach_sensor field table in `docs/api.md`**

Add a `formula` row to the attach_sensor request table (after `resample_minutes`):

```markdown
| formula     | no       | `{kind:"identity"\|"zero"\|"expr", expr, refs}`; default identity. For `expr`, `expr` is a text formula over `self`, numbers, `+ − × ÷`, `abs()`, and aliases; `refs` maps each alias to a sensor id (object or JSON string). Every alias must be bound. |
```

Note that `sensor_to_json` now includes a `formula` object.

- [ ] **Step 2: Note formula input in `docs/hierarchy-and-sensors.md`**

Add a short paragraph under the sensor model describing that a formula may be supplied at attach time (default Identity), with cross-sensor references scoped to the HN2 company, and that the expression grammar is `self`, numbers, `+ − × ÷`, `abs()`, parentheses, and aliases.

- [ ] **Step 3: Commit**

```bash
git add docs/api.md docs/hierarchy-and-sensors.md
git commit -m "docs: document sensor formula input on attach_sensor"
```

---

## Final verification

- [ ] Run the full suite: `dune runtest` — expect all suites green (domain.formula, repo.memory, logic.sensors, api.json, api.command, api.query).
- [ ] Run `dune build` — no warnings-as-errors failures.
- [ ] Confirm backward compatibility: existing `attach_sensor` calls without a `formula` still create Identity sensors (covered by the pre-existing attach tests).
