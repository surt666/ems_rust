open Ocaml_lambda_test

let uuid s = Uuidm.of_string s |> Option.get

let node_roundtrip_without_schema () =
  let id = Node_id.make Level.Hn4 (uuid "4b6a6f20-0000-0000-0000-000000000001") in
  let parent = Node_id.make Level.Hn3 (uuid "4b6a6f20-0000-0000-0000-000000000002") in
  let n =
    Node.make ~uuid:(Node_id.uuid id) ~level:Level.Hn4 ~name:"Building"
      ~parent ~created:(Ptime.epoch)
      ~metadata:(`Assoc [ ("lat", `Float 55.0) ])
      ~schema:None
  in
  let item = Codec.node_to_item n in
  match Codec.node_of_item item with
  | Ok n2 ->
      Alcotest.(check string) "same id"
        (Node_id.to_string n.Node.id) (Node_id.to_string n2.Node.id);
      Alcotest.(check string) "same name" n.Node.name n2.Node.name
  | Error e -> Alcotest.failf "decode failure: %s" e

let edge_item_shape () =
  let p = Node_id.make Level.Hn3 (uuid "4b6a6f20-0000-0000-0000-000000000003") in
  let c = Node_id.make Level.Hn4 (uuid "4b6a6f20-0000-0000-0000-000000000004") in
  let item = Codec.edge_item ~from_:p ~to_:c ~label:"building" ~created:Ptime.epoch in
  let sk =
    let v : Smaws_Client_DynamoDB.attribute_value = List.assoc "sk" item in
    match v with Smaws_Client_DynamoDB.S s -> s | _ -> Alcotest.fail "sk not S"
  in
  Alcotest.(check bool) "sk has has_building#"
    true (Astring.String.is_prefix ~affix:"has_building#HN4#" sk)

let tests =
  [
    Alcotest.test_case "node roundtrip" `Quick node_roundtrip_without_schema;
    Alcotest.test_case "edge item shape" `Quick edge_item_shape;
  ]
