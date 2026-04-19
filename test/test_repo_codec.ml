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
  let item =
    Codec.edge_item
      ~from_:(Node_id.to_string p)
      ~to_:(Node_id.to_string c)
      ~kind:(Edge_kind.Has_label "building")
      ~name:"Bld-1"
      ~created:Ptime.epoch
  in
  let name =
    match List.assoc_opt "name" item with
    | Some (Smaws_Client_DynamoDB.S s) -> s
    | _ -> Alcotest.fail "name missing"
  in
  Alcotest.(check string) "name on edge" "Bld-1" name;
  let sk =
    let v : Smaws_Client_DynamoDB.attribute_value = List.assoc "sk" item in
    match v with Smaws_Client_DynamoDB.S s -> s | _ -> Alcotest.fail "sk not S"
  in
  Alcotest.(check string) "sk"
    ("has_building#" ^ Node_id.to_string c) sk;
  let kind_s =
    let v : Smaws_Client_DynamoDB.attribute_value = List.assoc "kind" item in
    match v with Smaws_Client_DynamoDB.S s -> s | _ -> Alcotest.fail "kind not S"
  in
  Alcotest.(check string) "kind attribute" "has_label:building" kind_s;
  let gsi1pk =
    let v : Smaws_Client_DynamoDB.attribute_value = List.assoc "gsi1pk" item in
    match v with Smaws_Client_DynamoDB.S s -> s | _ -> Alcotest.fail "gsi1pk not S"
  in
  Alcotest.(check string) "gsi1pk" (Node_id.to_string c) gsi1pk;
  let gsi1sk =
    let v : Smaws_Client_DynamoDB.attribute_value = List.assoc "gsi1sk" item in
    match v with Smaws_Client_DynamoDB.S s -> s | _ -> Alcotest.fail "gsi1sk not S"
  in
  Alcotest.(check string) "gsi1sk"
    ("parent_of#" ^ Node_id.to_string p) gsi1sk

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
      created = t;
      parent;
      daq_id = "daq:x:y:z";
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
      Alcotest.(check string) "daq" s.daq_id s2.Sensor.daq_id;
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
      created = t;
      parent = Node_id.root;
      daq_id = "daq";
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

let sensor_edge_item_shape () =
  let parent = Node_id.make Level.Hn4 (uuid "4b6a6f20-0000-0000-0000-000000000050") in
  let sid =
    Sensor_id.make (uuid "4b6a6f20-0000-0000-0000-000000000051")
  in
  let item = Codec.sensor_edge_item ~parent ~sensor_id:sid ~created:Ptime.epoch in
  let s_of k =
    match List.assoc_opt k item with
    | Some (Smaws_Client_DynamoDB.S v) -> v
    | _ -> Alcotest.failf "missing/non-S field %s" k
  in
  Alcotest.(check string) "type"   "edge"       (s_of "type");
  Alcotest.(check string) "kind"   "has_sensor" (s_of "kind");
  Alcotest.(check string) "pk"     (Node_id.to_string parent)   (s_of "pk");
  Alcotest.(check string) "sk"
    ("has_sensor#" ^ Sensor_id.to_string sid) (s_of "sk");
  Alcotest.(check string) "gsi1pk" (Sensor_id.to_string sid)    (s_of "gsi1pk");
  Alcotest.(check string) "gsi1sk"
    ("sensor_of#" ^ Node_id.to_string parent) (s_of "gsi1sk");
  Alcotest.(check string) "name"   "" (s_of "name")

let schema_sensors_roundtrip () =
  let id = Node_id.make Level.Hn2 (uuid "4b6a6f20-0000-0000-0000-00000000dddd") in
  let sch : Schema.t =
    Schema.{
      version = 1;
      edges = [];
      metadata = [];
      sensors = [ Level.Hn4; Level.Hn5 ];
    }
  in
  let n =
    Node.make ~uuid:(Node_id.uuid id) ~level:Level.Hn2 ~name:"Co"
      ~parent:Node_id.root ~created:Ptime.epoch
      ~metadata:(`Assoc []) ~schema:(Some sch)
  in
  let item = Codec.node_to_item n in
  match Codec.node_of_item item with
  | Error e -> Alcotest.failf "decode: %s" e
  | Ok n2 ->
      let sch2 = Option.get n2.Node.schema in
      Alcotest.(check (list string)) "levels preserved"
        [ "hn4"; "hn5" ]
        (List.map Level.to_string sch2.Schema.sensors)

let tests =
  [
    Alcotest.test_case "node roundtrip" `Quick node_roundtrip_without_schema;
    Alcotest.test_case "edge item shape" `Quick edge_item_shape;
    Alcotest.test_case "sensor round trip"        `Quick sensor_round_trip;
    Alcotest.test_case "sensor sk active/history" `Quick sensor_active_sk_prefixed;
    Alcotest.test_case "sensor edge item shape"   `Quick sensor_edge_item_shape;
    Alcotest.test_case "schema sensors roundtrip" `Quick schema_sensors_roundtrip;
  ]
