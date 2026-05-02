open Ocaml_lambda_hierarchy

let seed_with_one_child () =
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
  let parent = Node_id.make Level.Hn4 10042 in
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

let seed_user_and_blocked_node () =
  let st, c2 = seed_with_one_child () in
  Memory.run st (fun () ->
    let _ =
      Api_command.dispatch
        ~body:{|{"action":"create_user","email":"alice@ex","name":"Alice","cognito_group":"writer"}|}
    in
    let body =
      Printf.sprintf
        {|{"action":"block_user","user_id":"U#alice@ex","node_id":%S}|}
        (Node_id.to_string c2)
    in
    let _ = Api_command.dispatch ~body in
    ());
  (st, c2)

let list_blocked_nodes_returns_nodes () =
  let st, c2 = seed_user_and_blocked_node () in
  let resp =
    Memory.run st (fun () ->
      Api_query.dispatch ~action:"list_blocked_nodes"
        ~params:[ ("user", "U#alice@ex") ])
  in
  let json = Yojson.Safe.from_string resp in
  let status = Yojson.Safe.Util.(json |> member "statusCode" |> to_int) in
  Alcotest.(check int) "status 200" 200 status;
  let inner =
    Yojson.Safe.Util.(json |> member "body" |> to_string |> Yojson.Safe.from_string)
  in
  let nodes =
    Yojson.Safe.Util.(inner |> member "nodes" |> to_list
                      |> List.map to_string)
  in
  Alcotest.(check int) "one node" 1 (List.length nodes);
  Alcotest.(check string) "node id" (Node_id.to_string c2) (List.hd nodes)

let list_blocked_users_returns_users () =
  let st, c2 = seed_user_and_blocked_node () in
  let resp =
    Memory.run st (fun () ->
      Api_query.dispatch ~action:"list_blocked_users"
        ~params:[ ("node", Node_id.to_string c2) ])
  in
  let json = Yojson.Safe.from_string resp in
  let status = Yojson.Safe.Util.(json |> member "statusCode" |> to_int) in
  Alcotest.(check int) "status 200" 200 status;
  let inner =
    Yojson.Safe.Util.(json |> member "body" |> to_string |> Yojson.Safe.from_string)
  in
  let users =
    Yojson.Safe.Util.(inner |> member "users" |> to_list
                      |> List.map to_string)
  in
  Alcotest.(check int) "one user" 1 (List.length users);
  Alcotest.(check string) "user id" "U#alice@ex" (List.hd users)

let effective_permission_before_and_after_block () =
  let st, c2 = seed_with_one_child () in
  Memory.run st (fun () ->
    let _ =
      Api_command.dispatch
        ~body:{|{"action":"create_user","email":"frank@ex","name":"Frank","cognito_group":"writer"}|}
    in
    (* before block: writer capability *)
    let resp =
      Api_query.dispatch ~action:"effective_permission"
        ~params:[ ("user", "U#frank@ex");
                  ("node", Node_id.to_string c2) ]
    in
    let json = Yojson.Safe.from_string resp in
    let status = Yojson.Safe.Util.(json |> member "statusCode" |> to_int) in
    Alcotest.(check int) "status 200" 200 status;
    let inner =
      Yojson.Safe.Util.(json |> member "body" |> to_string |> Yojson.Safe.from_string)
    in
    let cap = Yojson.Safe.Util.(inner |> member "capability" |> to_string) in
    Alcotest.(check string) "writer" "writer" cap;
    (* block and re-check *)
    let block =
      Printf.sprintf
        {|{"action":"block_user","user_id":"U#frank@ex","node_id":%S}|}
        (Node_id.to_string c2)
    in
    let _ = Api_command.dispatch ~body:block in
    let resp =
      Api_query.dispatch ~action:"effective_permission"
        ~params:[ ("user", "U#frank@ex");
                  ("node", Node_id.to_string c2) ]
    in
    let json = Yojson.Safe.from_string resp in
    let status = Yojson.Safe.Util.(json |> member "statusCode" |> to_int) in
    Alcotest.(check int) "status 200" 200 status;
    let inner =
      Yojson.Safe.Util.(json |> member "body" |> to_string |> Yojson.Safe.from_string)
    in
    let cap_null =
      match Yojson.Safe.Util.member "capability" inner with
      | `Null -> true
      | _ -> false
    in
    Alcotest.(check bool) "capability null" true cap_null;
    let reason = Yojson.Safe.Util.(inner |> member "reason" |> to_string) in
    Alcotest.(check string) "reason blocked" "blocked" reason)

let tests =
  [
    Alcotest.test_case "get_node" `Quick get_node_returns_node;
    Alcotest.test_case "list_children" `Quick list_children_returns_array;
    Alcotest.test_case "unknown action -> 400" `Quick unknown_action_is_bad_request;
    Alcotest.test_case "list_sensors empty" `Quick list_sensors_empty;
    Alcotest.test_case "get_user" `Quick get_user_happy;
    Alcotest.test_case "list_users empty" `Quick list_users_empty;
    Alcotest.test_case "list_blocked_nodes" `Quick list_blocked_nodes_returns_nodes;
    Alcotest.test_case "list_blocked_users" `Quick list_blocked_users_returns_users;
    Alcotest.test_case "effective_permission before/after block" `Quick
      effective_permission_before_and_after_block;
  ]
