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

let tests =
  [
    Alcotest.test_case "node roundtrip" `Quick node_roundtrip_without_schema;
    Alcotest.test_case "edge item shape" `Quick edge_item_shape;
    Alcotest.test_case "sensor round trip"        `Quick sensor_round_trip;
    Alcotest.test_case "sensor sk active/history" `Quick sensor_active_sk_prefixed;
  ]
