open Ocaml_lambda_test

let uuid s = Uuidm.of_string s |> Option.get

let seed () =
  let st = Memory.empty () in
  let c2 = Node_id.make Level.Hn2 (uuid "4b6a6f20-0000-0000-0000-00000000dddd") in
  let schema : Schema.t =
    Schema.{
      version = 1;
      edges = [
        (Level.Hn2, [ (Level.Hn3, [ { label = "property"; min = None; max = None } ]) ]);
      ];
      metadata = [];
      sensors = [];
    }
  in
  let n2 =
    Node.make ~uuid:(Node_id.uuid c2) ~level:Level.Hn2 ~name:"Acme"
      ~parent:Node_id.root ~created:Ptime.epoch
      ~metadata:(`Assoc []) ~schema:(Some schema)
  in
  Memory.run st (fun () -> Effects.put_node n2);
  (st, c2)

let add_node_happy_path () =
  let st, c2 = seed () in
  let body =
    Printf.sprintf
      {|{"action":"add_node","parent_id":%S,"level":"hn3","name":"P","metadata":{}}|}
      (Node_id.to_string c2)
  in
  let resp = Memory.run st (fun () -> Api_command.dispatch ~body) in
  let status =
    Yojson.Safe.Util.(Yojson.Safe.from_string resp |> member "statusCode" |> to_int)
  in
  Alcotest.(check int) "status 200" 200 status

let delete_node_roundtrip () =
  let st, c2 = seed () in
  Memory.run st (fun () ->
    let add_body =
      Printf.sprintf
        {|{"action":"add_node","parent_id":%S,"level":"hn3","name":"P","metadata":{}}|}
        (Node_id.to_string c2)
    in
    let resp = Api_command.dispatch ~body:add_body in
    let inner =
      Yojson.Safe.Util.(Yojson.Safe.from_string resp |> member "body" |> to_string |> Yojson.Safe.from_string)
    in
    let id = Yojson.Safe.Util.(inner |> member "id" |> to_string) in
    let del = Printf.sprintf {|{"action":"delete_node","id":%S}|} id in
    let resp = Api_command.dispatch ~body:del in
    let status =
      Yojson.Safe.Util.(Yojson.Safe.from_string resp |> member "statusCode" |> to_int)
    in
    Alcotest.(check int) "delete status" 200 status)

let invalid_json_is_400 () =
  let resp = Memory.run (Memory.empty ()) (fun () -> Api_command.dispatch ~body:"not json") in
  let status =
    Yojson.Safe.Util.(Yojson.Safe.from_string resp |> member "statusCode" |> to_int)
  in
  Alcotest.(check int) "400" 400 status

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
          sensors = [ Level.Hn3 ];
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
        {|{"action":"attach_sensor","parent_id":%S,"daq_id":"daq:1","purpose":"Electricity","meter_type":"counter","unit":"kWh"}|}
        (Node_id.to_string bldg)
    in
    let resp = Api_command.dispatch ~body in
    let status =
      Yojson.Safe.Util.(Yojson.Safe.from_string resp |> member "statusCode" |> to_int)
    in
    Alcotest.(check int) "200" 200 status)

let create_user_happy () =
  let resp =
    Memory.run (Memory.empty ()) (fun () ->
      Api_command.dispatch
        ~body:{|{"action":"create_user","email":"alice@ex","name":"Alice","cognito_group":"writer"}|})
  in
  let status =
    Yojson.Safe.Util.(Yojson.Safe.from_string resp |> member "statusCode" |> to_int)
  in
  Alcotest.(check int) "status 200" 200 status;
  let inner =
    Yojson.Safe.Util.(
      Yojson.Safe.from_string resp
      |> member "body" |> to_string
      |> Yojson.Safe.from_string)
  in
  let email = Yojson.Safe.Util.(inner |> member "email" |> to_string) in
  Alcotest.(check string) "email echoed" "alice@ex" email

let delete_user_roundtrip () =
  let st = Memory.empty () in
  Memory.run st (fun () ->
    let _ =
      Api_command.dispatch
        ~body:{|{"action":"create_user","email":"bob@ex","name":"Bob","cognito_group":"reader"}|}
    in
    let resp =
      Api_command.dispatch
        ~body:{|{"action":"delete_user","id":"U#bob@ex"}|}
    in
    let status =
      Yojson.Safe.Util.(Yojson.Safe.from_string resp |> member "statusCode" |> to_int)
    in
    Alcotest.(check int) "status 200" 200 status)

let block_user_happy () =
  let st, c2 = seed () in
  Memory.run st (fun () ->
    let _ =
      Api_command.dispatch
        ~body:{|{"action":"create_user","email":"dave@ex","name":"Dave","cognito_group":"writer"}|}
    in
    let body =
      Printf.sprintf
        {|{"action":"block_user","user_id":"U#dave@ex","node_id":%S}|}
        (Node_id.to_string c2)
    in
    let resp = Api_command.dispatch ~body in
    let json = Yojson.Safe.from_string resp in
    let status = Yojson.Safe.Util.(json |> member "statusCode" |> to_int) in
    Alcotest.(check int) "status 200" 200 status;
    let inner =
      Yojson.Safe.Util.(json |> member "body" |> to_string |> Yojson.Safe.from_string)
    in
    let ok = Yojson.Safe.Util.(inner |> member "ok" |> to_bool) in
    Alcotest.(check bool) "ok true" true ok)

let unblock_user_roundtrip () =
  let st, c2 = seed () in
  Memory.run st (fun () ->
    let _ =
      Api_command.dispatch
        ~body:{|{"action":"create_user","email":"eve@ex","name":"Eve","cognito_group":"writer"}|}
    in
    let block_body =
      Printf.sprintf
        {|{"action":"block_user","user_id":"U#eve@ex","node_id":%S}|}
        (Node_id.to_string c2)
    in
    let _ = Api_command.dispatch ~body:block_body in
    let unblock_body =
      Printf.sprintf
        {|{"action":"unblock_user","user_id":"U#eve@ex","node_id":%S}|}
        (Node_id.to_string c2)
    in
    let resp = Api_command.dispatch ~body:unblock_body in
    let status =
      Yojson.Safe.Util.(Yojson.Safe.from_string resp |> member "statusCode" |> to_int)
    in
    Alcotest.(check int) "status 200" 200 status)

let tests =
  [
    Alcotest.test_case "add_node happy path" `Quick add_node_happy_path;
    Alcotest.test_case "delete roundtrip" `Quick delete_node_roundtrip;
    Alcotest.test_case "invalid json -> 400" `Quick invalid_json_is_400;
    Alcotest.test_case "attach_sensor happy" `Quick attach_sensor_happy;
    Alcotest.test_case "create_user happy" `Quick create_user_happy;
    Alcotest.test_case "delete_user roundtrip" `Quick delete_user_roundtrip;
    Alcotest.test_case "block_user happy" `Quick block_user_happy;
    Alcotest.test_case "unblock_user roundtrip" `Quick unblock_user_roundtrip;
  ]
