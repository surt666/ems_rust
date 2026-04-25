open Ocaml_lambda_hierarchy

let uuid s = Uuidm.of_string s |> Option.get

let error_body_has_expected_shape () =
  let err = Errors.Bad_request "boom" in
  let body = Api_json.error_body err in
  let json = Yojson.Safe.from_string body in
  let code = Yojson.Safe.Util.(json |> member "error" |> member "code" |> to_string) in
  let msg  = Yojson.Safe.Util.(json |> member "error" |> member "message" |> to_string) in
  Alcotest.(check string) "code" "bad_request" code;
  Alcotest.(check string) "message" "boom" msg

let node_to_json_has_expected_keys () =
  let id = Node_id.make Level.Hn4 (uuid "4b6a6f20-0000-0000-0000-000000000001") in
  let n =
    Node.make ~uuid:(Node_id.uuid id) ~level:Level.Hn4 ~name:"X"
      ~parent:(Node_id.make Level.Hn3 (uuid "4b6a6f20-0000-0000-0000-000000000002"))
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

let tests =
  [
    Alcotest.test_case "error body shape" `Quick error_body_has_expected_shape;
    Alcotest.test_case "node JSON keys" `Quick node_to_json_has_expected_keys;
    Alcotest.test_case "schema JSON roundtrip" `Quick schema_json_roundtrip;
  ]
