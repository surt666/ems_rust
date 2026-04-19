open Ocaml_lambda_test

let uuid s = Uuidm.of_string s |> Option.get

let seed_with_one_child () =
  let st = Memory.empty () in
  let c2 = Node_id.make Level.Hn2 (uuid "4b6a6f20-0000-0000-0000-00000000cccc") in
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
  Memory.run st (fun () ->
    Effects.put_node n2;
    match
      Hierarchy.add_node ~parent:c2 ~level:Level.Hn3 ~name:"P"
        ~metadata:(`Assoc []) ()
    with
    | Ok _ -> ()
    | Error e -> Alcotest.failf "%s" (Errors.message e));
  st, c2

let get_node_returns_node () =
  let st, c2 = seed_with_one_child () in
  let body =
    Memory.run st (fun () ->
      Api_query.dispatch ~action:"get_node"
        ~params:[ ("id", Node_id.to_string c2) ])
  in
  let json = Yojson.Safe.from_string body in
  let status = Yojson.Safe.Util.(json |> member "statusCode" |> to_int) in
  Alcotest.(check int) "status" 200 status

let list_children_returns_array () =
  let st, c2 = seed_with_one_child () in
  let body =
    Memory.run st (fun () ->
      Api_query.dispatch ~action:"list_children"
        ~params:[ ("parent", Node_id.to_string c2) ])
  in
  let json = Yojson.Safe.from_string body in
  let status = Yojson.Safe.Util.(json |> member "statusCode" |> to_int) in
  Alcotest.(check int) "status" 200 status;
  let inner =
    Yojson.Safe.Util.(json |> member "body" |> to_string |> Yojson.Safe.from_string)
  in
  let children =
    Yojson.Safe.Util.(inner |> member "children" |> to_list)
  in
  Alcotest.(check int) "one child" 1 (List.length children)

let unknown_action_is_bad_request () =
  let body =
    Memory.run (Memory.empty ()) (fun () ->
      Api_query.dispatch ~action:"does_not_exist" ~params:[])
  in
  let status =
    Yojson.Safe.Util.(Yojson.Safe.from_string body |> member "statusCode" |> to_int)
  in
  Alcotest.(check int) "status 400" 400 status

let list_sensors_empty () =
  let st = Memory.empty () in
  let parent =
    Node_id.make Level.Hn4
      (Uuidm.of_string "55555555-0000-4000-8000-000000000001" |> Option.get)
  in
  Memory.run st (fun () ->
    let resp =
      Api_query.dispatch ~action:"list_sensors"
        ~params:[ ("parent", Node_id.to_string parent) ]
    in
    let status =
      Yojson.Safe.Util.(Yojson.Safe.from_string resp |> member "statusCode" |> to_int)
    in
    Alcotest.(check int) "200" 200 status)

let get_user_happy () =
  let st = Memory.empty () in
  Memory.run st (fun () ->
    let _ =
      Api_command.dispatch
        ~body:{|{"action":"create_user","email":"carol@ex","name":"Carol","cognito_group":"admin"}|}
    in
    let resp =
      Api_query.dispatch ~action:"get_user"
        ~params:[ ("id", "U#carol@ex") ]
    in
    let status =
      Yojson.Safe.Util.(Yojson.Safe.from_string resp |> member "statusCode" |> to_int)
    in
    Alcotest.(check int) "status 200" 200 status)

let list_users_empty () =
  let resp =
    Memory.run (Memory.empty ()) (fun () ->
      Api_query.dispatch ~action:"list_users" ~params:[])
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
  let users =
    Yojson.Safe.Util.(inner |> member "users" |> to_list)
  in
  Alcotest.(check int) "empty list" 0 (List.length users)

let tests =
  [
    Alcotest.test_case "get_node" `Quick get_node_returns_node;
    Alcotest.test_case "list_children" `Quick list_children_returns_array;
    Alcotest.test_case "unknown action -> 400" `Quick unknown_action_is_bad_request;
    Alcotest.test_case "list_sensors empty" `Quick list_sensors_empty;
    Alcotest.test_case "get_user" `Quick get_user_happy;
    Alcotest.test_case "list_users empty" `Quick list_users_empty;
  ]
