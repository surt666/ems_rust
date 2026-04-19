open Ocaml_lambda_test

let uuid_of s = Uuidm.of_string s |> Option.get

let sample_schema () : Schema.t =
  Schema.{
    version = 1;
    edges = [
      (Level.Hn2, [ (Level.Hn3, [ { label = "building"; min = None; max = None } ]) ]);
    ];
    metadata = [];
    sensors = [];
  }

let seed () =
  let st = Memory.empty () in
  let c2 = Node_id.make Level.Hn2
             (uuid_of "4b6a6f20-0000-0000-0000-0000000000c0") in
  let n2 =
    Node.make ~uuid:(Node_id.uuid c2) ~level:Level.Hn2 ~name:"Acme"
      ~parent:Node_id.root ~created:Ptime.epoch
      ~metadata:(`Assoc []) ~schema:(Some (sample_schema ()))
  in
  Memory.run st (fun () -> Effects.put_node n2);
  let bldg =
    Memory.run st (fun () ->
      match
        Hierarchy.add_node ~parent:c2 ~level:Level.Hn3
          ~name:"B" ~metadata:(`Assoc []) ()
      with
      | Ok b -> b.Node.id
      | Error e -> Alcotest.failf "seed: %s" (Errors.message e))
  in
  let u =
    User.make ~email:"alice@ex" ~name:"Alice"
      ~cognito_group:Cognito_group.Writer ~created:Ptime.epoch ()
  in
  Memory.run st (fun () -> Effects.put_user u);
  (st, c2, bldg, u.User.id)

let block_then_list () =
  let st, c2, _bldg, uid = seed () in
  Memory.run st (fun () ->
    match Access.block ~user_id:uid ~node_id:c2 () with
    | Error e -> Alcotest.failf "block: %s" (Errors.message e)
    | Ok () ->
        (match Access.list_blocked_nodes ~user_id:uid with
         | Error e -> Alcotest.failf "list: %s" (Errors.message e)
         | Ok xs ->
             Alcotest.(check int) "one" 1 (List.length xs);
             Alcotest.(check string) "node"
               (Node_id.to_string c2)
               (Node_id.to_string (List.hd xs))))

let block_unknown_user_fails () =
  let st = Memory.empty () in
  let c2 = Node_id.make Level.Hn2
             (uuid_of "4b6a6f20-0000-0000-0000-0000000000c1") in
  let n2 =
    Node.make ~uuid:(Node_id.uuid c2) ~level:Level.Hn2 ~name:"Acme"
      ~parent:Node_id.root ~created:Ptime.epoch
      ~metadata:(`Assoc []) ~schema:(Some (sample_schema ()))
  in
  Memory.run st (fun () -> Effects.put_node n2);
  Memory.run st (fun () ->
    match
      Access.block
        ~user_id:(User_id.of_email "ghost@ex") ~node_id:c2 ()
    with
    | Ok () -> Alcotest.fail "expected Not_found_user"
    | Error (Errors.Not_found_user _) -> ()
    | Error e -> Alcotest.failf "wrong error: %s" (Errors.message e))

let effective_permission_flows () =
  let st, c2, bldg, uid = seed () in
  Memory.run st (fun () ->
    (* baseline: writer capability *)
    (match Access.effective_permission ~user_id:uid ~node_id:bldg with
     | Ok (Some g) ->
         Alcotest.(check string) "writer" "writer"
           (Cognito_group.to_string g)
     | Ok None -> Alcotest.fail "expected writer, got None"
     | Error e -> Alcotest.failf "eff: %s" (Errors.message e));
    (* block on ancestor c2 *)
    (match Access.block ~user_id:uid ~node_id:c2 () with
     | Error e -> Alcotest.failf "block: %s" (Errors.message e)
     | Ok () -> ());
    (* now blocked at ancestor, so inherited at child *)
    (match Access.effective_permission ~user_id:uid ~node_id:c2 with
     | Ok None -> ()
     | Ok (Some _) -> Alcotest.fail "expected blocked at c2"
     | Error e -> Alcotest.failf "eff: %s" (Errors.message e));
    (match Access.effective_permission ~user_id:uid ~node_id:bldg with
     | Ok None -> ()
     | Ok (Some _) -> Alcotest.fail "expected inherited block at bldg"
     | Error e -> Alcotest.failf "eff: %s" (Errors.message e));
    (* unblock restores *)
    (match Access.unblock ~user_id:uid ~node_id:c2 () with
     | Error e -> Alcotest.failf "unblock: %s" (Errors.message e)
     | Ok () -> ());
    match Access.effective_permission ~user_id:uid ~node_id:bldg with
    | Ok (Some _) -> ()
    | Ok None -> Alcotest.fail "expected restored"
    | Error e -> Alcotest.failf "eff: %s" (Errors.message e))

let list_blocked_users_reverse () =
  let st, c2, _bldg, uid = seed () in
  Memory.run st (fun () ->
    match Access.block ~user_id:uid ~node_id:c2 () with
    | Error e -> Alcotest.failf "block: %s" (Errors.message e)
    | Ok () ->
        (match Access.list_blocked_users ~node_id:c2 with
         | Ok xs ->
             Alcotest.(check int) "one" 1 (List.length xs);
             Alcotest.(check string) "user"
               (User_id.to_string uid)
               (User_id.to_string (List.hd xs))
         | Error e -> Alcotest.failf "list: %s" (Errors.message e)))

let tests =
  [ Alcotest.test_case "block then list" `Quick block_then_list
  ; Alcotest.test_case "block unknown user fails" `Quick block_unknown_user_fails
  ; Alcotest.test_case "effective_permission + inheritance" `Quick effective_permission_flows
  ; Alcotest.test_case "list_blocked_users reverse" `Quick list_blocked_users_reverse
  ]
