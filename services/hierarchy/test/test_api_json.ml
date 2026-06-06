open Ocaml_lambda_hierarchy

let error_body_has_expected_shape () =
  let err = Errors.Bad_request "boom" in
  let body = Api_json.error_body err in
  let json = Yojson.Safe.from_string body in
  let code = Yojson.Safe.Util.(json |> member "error" |> member "code" |> to_string) in
  let msg  = Yojson.Safe.Util.(json |> member "error" |> member "message" |> to_string) in
  Alcotest.(check string) "code" "bad_request" code;
  Alcotest.(check string) "message" "boom" msg

let node_to_json_has_expected_keys () =
  let n =
    Node.make ~id:10004 ~level:Level.Hn4 ~name:"X"
      ~parent:(Node_id.make Level.Hn3 10003)
      ~parent_path:(Node_id.to_string Node_id.root)
      ~created:Ptime.epoch
      ~metadata:(`Assoc [ ("lat", `Float 55.0) ]) ~schema:None
  in
  let j = Api_json.node_to_json n in
  let keys =
    match j with
    | `Assoc kvs -> List.map fst kvs
    | _ -> []
  in
  List.iter
    (fun k -> Alcotest.(check bool) ("has key " ^ k) true (List.mem k keys))
    [ "id"; "name"; "parent"; "created"; "metadata" ]

let schema_json_roundtrip () =
  let schema : Schema.t =
    Schema.{
      version = 1;
      edges = [
        (Level.Hn2, [ (Level.Hn3, [
          { label = "property"; min = None; max = None };
          { label = "group";    min = None; max = Some 3 };
        ]) ]);
        (Level.Hn3, [ (Level.Hn4, [ { label = "building"; min = Some 1; max = None } ]) ]);
      ];
      metadata = [
        (Level.Hn4, [
          ("lat", Metadata.{ typ = Number { min = Some (-90.); max = Some 90. }; required = true });
          ("kind", Metadata.{ typ = Enum { one_of = [ "a"; "b" ] }; required = false });
        ]);
      ];
      sensors = [ Level.Hn4 ];
    }
  in
  let j = Api_json.schema_to_json schema in
  match Api_json.schema_of_json j with
  | Error msg -> Alcotest.failf "decode: %s" msg
  | Ok s2 ->
      Alcotest.(check int) "version" schema.version s2.Schema.version;
      Alcotest.(check int) "edges parents" (List.length schema.edges) (List.length s2.Schema.edges);
      Alcotest.(check int) "sensors" 1 (List.length s2.Schema.sensors)

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

let formula_of_json_unused_ref_errors () =
  let j = `Assoc [ ("kind", `String "expr"); ("expr", `String "self");
                   ("refs", `Assoc [ ("a", `String "S#1") ]) ] in
  match Api_json.formula_of_json (Some j) with
  | Error _ -> () | Ok _ -> Alcotest.fail "unused ref must error"

let formula_of_json_implicit_expr () =
  let j = `Assoc [ ("expr", `String "self - a");
                   ("refs", `Assoc [ ("a", `String "S#1") ]) ] in
  match Api_json.formula_of_json (Some j) with
  | Ok (Formula.Expr _) -> () | _ -> Alcotest.fail "kind-absent expr should parse"

let formula_of_json_string_shorthands () =
  (match Api_json.formula_of_json (Some `Null) with Ok Formula.Identity -> () | _ -> Alcotest.fail "null->identity");
  (match Api_json.formula_of_json (Some (`String "identity")) with Ok Formula.Identity -> () | _ -> Alcotest.fail "string identity");
  (match Api_json.formula_of_json (Some (`String "zero")) with Ok Formula.Zero -> () | _ -> Alcotest.fail "string zero")

let formula_json_roundtrips () =
  let f = Formula.Expr { ast = Ocaml_lambda_hierarchy.Formula_parser.parse "abs(self - a)" |> Result.get_ok;
                         refs = [ ("a", Sensor_id.make 12) ] } in
  match Api_json.formula_of_json (Some (Api_json.formula_to_json f)) with
  | Ok (Formula.Expr { ast; refs }) ->
      Alcotest.(check string) "ast" "abs(self - a)" (Formula.expr_to_string ast);
      Alcotest.(check int) "refs" 1 (List.length refs)
  | _ -> Alcotest.fail "round-trip failed"

let tests =
  [
    Alcotest.test_case "error body shape" `Quick error_body_has_expected_shape;
    Alcotest.test_case "node JSON keys" `Quick node_to_json_has_expected_keys;
    Alcotest.test_case "schema JSON roundtrip" `Quick schema_json_roundtrip;
    Alcotest.test_case "formula_of_json default identity" `Quick formula_of_json_default_identity;
    Alcotest.test_case "formula_of_json zero"             `Quick formula_of_json_zero;
    Alcotest.test_case "formula_of_json expr+refs"        `Quick formula_of_json_expr_with_refs;
    Alcotest.test_case "formula_of_json refs as string"   `Quick formula_of_json_refs_as_string;
    Alcotest.test_case "formula_of_json unbound alias"    `Quick formula_of_json_unbound_alias_errors;
    Alcotest.test_case "sensor_to_json includes formula"  `Quick sensor_to_json_includes_formula;
    Alcotest.test_case "formula_of_json unused ref errors" `Quick formula_of_json_unused_ref_errors;
    Alcotest.test_case "formula_of_json implicit expr"     `Quick formula_of_json_implicit_expr;
    Alcotest.test_case "formula_of_json string shorthands" `Quick formula_of_json_string_shorthands;
    Alcotest.test_case "formula json round-trips"          `Quick formula_json_roundtrips;
  ]
