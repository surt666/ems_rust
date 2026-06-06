open Ocaml_lambda_hierarchy

let node_roundtrip_without_schema () =
  let parent = Node_id.make Level.Hn3 10003 in
  let parent_path =
    Node_id.to_string Node_id.root ^ "|"
    ^ Node_id.to_string (Node_id.make Level.Hn1 10001) ^ "|"
    ^ Node_id.to_string (Node_id.make Level.Hn2 10002) ^ "|"
    ^ Node_id.to_string parent
  in
  let n =
    Node.make ~id:10004 ~level:Level.Hn4 ~name:"Building"
      ~parent ~parent_path ~created:(Ptime.epoch)
      ~metadata:(`Assoc [ ("lat", `Float 55.0) ])
      ~schema:None
  in
  let item = Codec.node_to_item n in
  match Codec.node_of_item item with
  | Ok n2 ->
      Alcotest.(check string) "same id"
        (Node_id.to_string n.Node.id) (Node_id.to_string n2.Node.id);
      Alcotest.(check string) "same name" n.Node.name n2.Node.name;
      Alcotest.(check string) "same path" n.Node.path n2.Node.path
  | Error e -> Alcotest.failf "decode failure: %s" e

let edge_with_anchor_shape () =
  let p = Node_id.make Level.Hn3 10003 in
  let c = Node_id.make Level.Hn4 10004 in
  let child_path =
    Node_id.to_string Node_id.root ^ "|HN1#10001|HN2#10002|"
    ^ Node_id.to_string p ^ "|" ^ Node_id.to_string c
  in
  let item =
    Codec.edge_with_anchor
      ~from_:(Node_id.to_string p)
      ~to_:(Node_id.to_string c)
      ~kind:(Edge_kind.Has_label "building")
      ~name:"Bld-1"
      ~created:Ptime.epoch
      ~self_path:child_path
  in
  let s_of k =
    match List.assoc_opt k item with
    | Some (Smaws_Client_DynamoDB.S v) -> v
    | _ -> Alcotest.failf "missing/non-S field %s" k
  in
  Alcotest.(check string) "name on edge" "Bld-1" (s_of "name");
  Alcotest.(check string) "sk"
    ("has_building#" ^ Node_id.to_string c) (s_of "sk");
  Alcotest.(check string) "kind attribute" "has_label:building" (s_of "kind");
  Alcotest.(check string) "gsi1pk = child level" "HN4" (s_of "gsi1pk");
  Alcotest.(check string) "gsi1sk = self path" child_path (s_of "gsi1sk")

let sensor_round_trip () =
  let parent = Node_id.make Level.Hn4 10004 in
  let t = Ptime.of_rfc3339 "2026-04-18T10:00:00Z" |> Result.get_ok
          |> fun (t, _, _) -> t
  in
  let id = Sensor_id.make 20001 in
  let path =
    Node_id.to_string Node_id.root
    ^ "|HN1#10001|HN2#10002|HN3#10003|"
    ^ Node_id.to_string parent
    ^ "|" ^ Sensor_id.to_string id
  in
  let s : Sensor.t =
    {
      id;
      created = t;
      daq_id = "daq:x:y:z";
      path;
      purpose = "Electricity";
      meter_type = Sensor.Counter;
      unit = Some "kWh";
      formula = Formula.Expr {
        ast = Formula.Abs (Formula.Sub (Formula.Self, Formula.Ref "r"));
        refs = [ ("r", id) ];
      };
      resample_minutes = Some 15;
    }
  in
  let item = Codec.sensor_to_item ~active:true s in
  match Codec.sensor_of_item item with
  | Ok s2 ->
      Alcotest.(check string) "daq" s.daq_id s2.Sensor.daq_id;
      Alcotest.(check string) "path" s.path s2.Sensor.path;
      Alcotest.(check string) "parent_id derived"
        (Node_id.to_string parent)
        (Node_id.to_string (Sensor.parent_id s2));
      Alcotest.(check string) "purpose" s.purpose s2.Sensor.purpose;
      Alcotest.(check (option int)) "resample_minutes" (Some 15) s2.Sensor.resample_minutes;
      Alcotest.(check bool)   "formula kind" true
        (match s2.Sensor.formula with Formula.Expr _ -> true | _ -> false)
  | Error e -> Alcotest.failf "decode: %s" e

let sensor_active_sk_prefixed () =
  let t = Ptime.of_rfc3339 "2026-04-18T10:00:00Z" |> Result.get_ok
          |> fun (t, _, _) -> t
  in
  let id = Sensor_id.make 20002 in
  let s : Sensor.t =
    {
      id;
      created = t;
      daq_id = "daq";
      path = Node_id.to_string Node_id.root ^ "|" ^ Sensor_id.to_string id;
      purpose = "Heat";
      meter_type = Sensor.Gauge;
      unit = None;
      formula = Formula.Identity;
      resample_minutes = Some 60;
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
  let parent = Node_id.make Level.Hn4 10050 in
  let sid = Sensor_id.make 20051 in
  let self_path =
    "HN0#root|HN1#10001|HN2#10002|HN3#10003|"
    ^ Node_id.to_string parent ^ "|" ^ Sensor_id.to_string sid
  in
  let item =
    Codec.sensor_edge_item ~parent ~sensor_id:sid ~created:Ptime.epoch
      ~self_path
  in
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
  Alcotest.(check string) "gsi1pk = S" "S" (s_of "gsi1pk");
  Alcotest.(check string) "gsi1sk = sensor path" self_path (s_of "gsi1sk");
  Alcotest.(check string) "name"   "" (s_of "name")

let schema_sensors_roundtrip () =
  let sch : Schema.t =
    Schema.{
      version = 1;
      edges = [];
      metadata = [];
      sensors = [ Level.Hn4; Level.Hn5 ];
    }
  in
  let parent_path =
    Node_id.to_string Node_id.root ^ "|"
    ^ Node_id.to_string (Node_id.make Level.Hn1 10001)
  in
  let n =
    Node.make ~id:10002 ~level:Level.Hn2 ~name:"Co"
      ~parent:Node_id.root ~parent_path ~created:Ptime.epoch
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

let user_roundtrip () =
  let u =
    User.make ~email:"alice@example.com" ~name:"Alice"
      ~cognito_group:Cognito_group.Writer
      ~language:Language.English
      ~currency:Currency.EUR
      ~created:Ptime.epoch ()
  in
  let item = Codec.user_item u in
  (match List.assoc_opt "gsi1pk" item with
   | Some (Smaws_Client_DynamoDB.S s) ->
       Alcotest.(check string) "gsi1pk" "user" s
   | _ -> Alcotest.fail "gsi1pk missing");
  match Codec.user_of_item item with
  | Ok u' ->
      Alcotest.(check string) "id"
        (User_id.to_string u.User.id) (User_id.to_string u'.User.id);
      Alcotest.(check string) "name" u.User.name u'.User.name;
      Alcotest.(check string) "cognito"
        (Cognito_group.to_string u.User.cognito_group)
        (Cognito_group.to_string u'.User.cognito_group);
      Alcotest.(check string) "language"
        (Language.to_string u.User.language)
        (Language.to_string u'.User.language);
      Alcotest.(check string) "currency"
        (Currency.to_string u.User.currency)
        (Currency.to_string u'.User.currency)
  | Error e -> Alcotest.failf "user_of_item: %s" e

let tests =
  [
    Alcotest.test_case "node roundtrip" `Quick node_roundtrip_without_schema;
    Alcotest.test_case "edge with anchor shape" `Quick edge_with_anchor_shape;
    Alcotest.test_case "sensor round trip"        `Quick sensor_round_trip;
    Alcotest.test_case "sensor sk active/history" `Quick sensor_active_sk_prefixed;
    Alcotest.test_case "sensor edge item shape"   `Quick sensor_edge_item_shape;
    Alcotest.test_case "schema sensors roundtrip" `Quick schema_sensors_roundtrip;
    Alcotest.test_case "user roundtrip"           `Quick user_roundtrip;
  ]
