open Ocaml_lambda_test

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

let tests =
  [
    Alcotest.test_case "error body shape" `Quick error_body_has_expected_shape;
    Alcotest.test_case "node JSON keys" `Quick node_to_json_has_expected_keys;
  ]
