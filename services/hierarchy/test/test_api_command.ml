open Ocaml_lambda_hierarchy

let contains hay needle =
  let nh = String.length needle and hh = String.length hay in
  let rec go i = i + nh <= hh && (String.sub hay i nh = needle || go (i + 1)) in
  nh = 0 || go 0

let seed () =
  let st = Memory.empty () in
  let c2 = Node_id.make Level.Hn2 10002 in
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
  let parent_path =
    Node_id.to_string Node_id.root ^ "|"
    ^ Node_id.to_string (Node_id.make Level.Hn1 10001)
  in
  let n2 =
    Node.make ~id:10002 ~level:Level.Hn2 ~name:"Acme"
      ~parent:Node_id.root ~parent_path ~created:Ptime.epoch
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

let attach_sensor_happy () =
  (* Tests that follow the existing seed pattern for a company with an electricity slot *)
  let st = Memory.empty () in
  let bldg = seed_building st in
  Memory.run st (fun () ->
    let body =
      Printf.sprintf
        {|{"action":"attach_sensor","parent_id":%S,"daq_id":"daq:1","purpose":"Electricity","meter_type":"counter","unit":"kWh","resample_minutes":15}|}
        (Node_id.to_string bldg)
    in
    let resp = Api_command.dispatch ~body in
    let status =
      Yojson.Safe.Util.(Yojson.Safe.from_string resp |> member "statusCode" |> to_int)
    in
    Alcotest.(check int) "200" 200 status)

let attach_sensor_resample_from_string () =
  (* The HTML form posts resample_minutes as a string ("data.resample_minutes=15"
     → `String "15"), and an empty number input posts "". run_attach_sensor must
     coerce a numeric string to Some, and treat "" / absent as unset (graceful
     Null). The legacy "binning" key is also still accepted for backward compat. *)
  let st = Memory.empty () in
  let bldg = seed_building st in
  let resample_of resp =
    Yojson.Safe.Util.(
      Yojson.Safe.from_string resp
      |> member "body" |> to_string
      |> Yojson.Safe.from_string
      |> member "resample_minutes")
  in
  Memory.run st (fun () ->
    let body ?(key = "resample_minutes") daq bn =
      Printf.sprintf
        {|{"action":"attach_sensor","parent_id":%S,"daq_id":%S,"purpose":"Electricity","meter_type":"counter",%S:%s}|}
        (Node_id.to_string bldg) daq key bn
    in
    let resp = Api_command.dispatch ~body:(body "daq:str" {|"15"|}) in
    Alcotest.(check (option int)) "numeric-string resample_minutes coerced to 15"
      (Some 15)
      (match resample_of resp with `Int i -> Some i | _ -> None);
    let resp2 = Api_command.dispatch ~body:(body "daq:empty" {|""|}) in
    Alcotest.(check bool) "empty-string resample_minutes is unset (null)" true
      (match resample_of resp2 with `Null -> true | _ -> false);
    (* legacy "binning" input key still maps to resample_minutes output *)
    let resp3 = Api_command.dispatch ~body:(body ~key:"binning" "daq:legacy" {|15|}) in
    Alcotest.(check (option int)) "legacy binning key coerced to 15"
      (Some 15)
      (match resample_of resp3 with `Int i -> Some i | _ -> None))

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
    Alcotest.(check int) "400 on unbound alias" 400 status;
    let body = Yojson.Safe.Util.(Yojson.Safe.from_string resp |> member "body" |> to_string) in
    Alcotest.(check bool) "mentions unbound alias" true (contains body "unbound alias"))

let company_sensors_fragment_lists_options () =
  let st = Memory.empty () in
  let bldg = seed_building st in
  let sid =
    Memory.run st (fun () ->
      let resp = Api_command.dispatch ~body:(Printf.sprintf
        {|{"action":"attach_sensor","parent_id":%S,"daq_id":"daq:opt","purpose":"Electricity","meter_type":"counter"}|}
        (Node_id.to_string bldg)) in
      Yojson.Safe.Util.(Yojson.Safe.from_string resp |> member "body" |> to_string
        |> Yojson.Safe.from_string |> member "id" |> to_string))
  in
  let bldg_path =
    Memory.run st (fun () ->
      match Effects.get_node bldg with Some n -> n.Node.path | None -> "")
  in
  let resp =
    Memory.run st (fun () ->
      Api_html.dispatch ~action:"company_sensors" ~params:[ ("nodepath", bldg_path) ])
  in
  let body = Yojson.Safe.Util.(Yojson.Safe.from_string resp |> member "body" |> to_string) in
  Alcotest.(check bool) "fragment renders an option" true (contains body "<option");
  Alcotest.(check bool) "fragment lists the company sensor by id" true (contains body sid)

let company_sensors_missing_nodepath_400 () =
  let resp =
    Memory.run (Memory.empty ()) (fun () ->
      Api_html.dispatch ~action:"company_sensors" ~params:[]) in
  let status = Yojson.Safe.Util.(Yojson.Safe.from_string resp |> member "statusCode" |> to_int) in
  Alcotest.(check int) "400 when nodepath missing" 400 status

let add_sensor_form_has_formula_controls () =
  let st = Memory.empty () in
  let bldg = seed_building st in
  let resp =
    Memory.run st (fun () ->
      Api_html.dispatch ~action:"node" ~params:[ ("id", Node_id.to_string bldg) ])
  in
  let body = Yojson.Safe.Util.(Yojson.Safe.from_string resp |> member "body" |> to_string) in
  Alcotest.(check bool) "has formula.kind hidden field" true (contains body "data.formula.kind");
  Alcotest.(check bool) "has formula dialog" true (contains body "formula-dialog")

let tests =
  [
    Alcotest.test_case "add_node happy path" `Quick add_node_happy_path;
    Alcotest.test_case "delete roundtrip" `Quick delete_node_roundtrip;
    Alcotest.test_case "invalid json -> 400" `Quick invalid_json_is_400;
    Alcotest.test_case "attach_sensor happy" `Quick attach_sensor_happy;
    Alcotest.test_case "attach_sensor resample_minutes from string" `Quick attach_sensor_resample_from_string;
    Alcotest.test_case "attach_sensor with formula"         `Quick attach_sensor_with_formula;
    Alcotest.test_case "attach_sensor unbound alias -> 400"  `Quick attach_sensor_unbound_alias_400;
    Alcotest.test_case "create_user happy" `Quick create_user_happy;
    Alcotest.test_case "delete_user roundtrip" `Quick delete_user_roundtrip;
    Alcotest.test_case "block_user happy" `Quick block_user_happy;
    Alcotest.test_case "unblock_user roundtrip" `Quick unblock_user_roundtrip;
    Alcotest.test_case "company_sensors fragment lists options" `Quick company_sensors_fragment_lists_options;
    Alcotest.test_case "company_sensors missing nodepath -> 400" `Quick company_sensors_missing_nodepath_400;
    Alcotest.test_case "add-sensor form has formula controls" `Quick add_sensor_form_has_formula_controls;
  ]
