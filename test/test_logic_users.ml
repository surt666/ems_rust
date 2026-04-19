open Ocaml_lambda_test

let put_get_roundtrip () =
  let st = Memory.empty () in
  Memory.run st (fun () ->
    let u =
      User.make ~email:"a@x" ~name:"A"
        ~cognito_group:Cognito_group.Reader
        ~created:Ptime.epoch ()
    in
    Effects.put_user u;
    match Effects.get_user u.User.id with
    | Some u' -> Alcotest.(check string) "name" "A" u'.User.name
    | None -> Alcotest.fail "put then get returned None")

let list_and_delete () =
  let st = Memory.empty () in
  Memory.run st (fun () ->
    let u1 =
      User.make ~email:"a@x" ~name:"A"
        ~cognito_group:Cognito_group.Reader
        ~created:Ptime.epoch ()
    in
    let u2 =
      User.make ~email:"b@x" ~name:"B"
        ~cognito_group:Cognito_group.Writer
        ~created:Ptime.epoch ()
    in
    Effects.put_user u1;
    Effects.put_user u2;
    let all = Effects.list_users () in
    Alcotest.(check int) "two users" 2 (List.length all);
    Effects.delete_user u1.User.id;
    let remaining = Effects.list_users () in
    Alcotest.(check int) "one left" 1 (List.length remaining);
    match Effects.get_user u1.User.id with
    | None -> ()
    | Some _ -> Alcotest.fail "u1 should be gone")

(* Task 7 — logic-layer tests *)

let create_happy () =
  let st = Memory.empty () in
  Memory.run st (fun () ->
    match
      Users.create ~email:"a@x" ~name:"A"
        ~cognito_group:Cognito_group.Reader ()
    with
    | Ok u ->
        Alcotest.(check string) "name" "A" u.User.name;
        Alcotest.(check string) "email" "a@x" (User_id.email u.User.id)
    | Error e -> Alcotest.failf "create: %s" (Errors.message e))

let create_duplicate_conflicts () =
  let st = Memory.empty () in
  Memory.run st (fun () ->
    let _ =
      Users.create ~email:"a@x" ~name:"A"
        ~cognito_group:Cognito_group.Reader ()
    in
    match
      Users.create ~email:"a@x" ~name:"A again"
        ~cognito_group:Cognito_group.Reader ()
    with
    | Ok _ -> Alcotest.fail "expected Conflict"
    | Error (Errors.Conflict _) -> ()
    | Error e -> Alcotest.failf "wrong error: %s" (Errors.message e))

let get_unknown_is_not_found () =
  let st = Memory.empty () in
  Memory.run st (fun () ->
    match Users.get (User_id.of_email "ghost@x") with
    | Ok _ -> Alcotest.fail "expected error"
    | Error (Errors.Not_found _) -> ()
    | Error e -> Alcotest.failf "wrong error: %s" (Errors.message e))

let update_changes_name () =
  let st = Memory.empty () in
  Memory.run st (fun () ->
    let u =
      match
        Users.create ~email:"a@x" ~name:"A"
          ~cognito_group:Cognito_group.Reader ()
      with
      | Ok u -> u
      | Error e -> Alcotest.failf "create: %s" (Errors.message e)
    in
    match Users.update ~id:u.User.id ~name:"Alice" () with
    | Ok u' -> Alcotest.(check string) "name" "Alice" u'.User.name
    | Error e -> Alcotest.failf "update: %s" (Errors.message e))

let list_and_delete_via_logic () =
  let st = Memory.empty () in
  Memory.run st (fun () ->
    let u1 =
      match
        Users.create ~email:"a@x" ~name:"A"
          ~cognito_group:Cognito_group.Reader ()
      with
      | Ok u -> u
      | Error e -> Alcotest.failf "create: %s" (Errors.message e)
    in
    let _ =
      Users.create ~email:"b@x" ~name:"B"
        ~cognito_group:Cognito_group.Writer ()
    in
    (match Users.list () with
     | Ok xs -> Alcotest.(check int) "two" 2 (List.length xs)
     | Error e -> Alcotest.failf "list: %s" (Errors.message e));
    (match Users.delete u1.User.id with
     | Ok _ -> ()
     | Error e -> Alcotest.failf "delete: %s" (Errors.message e));
    match Users.list () with
    | Ok xs -> Alcotest.(check int) "one left" 1 (List.length xs)
    | Error e -> Alcotest.failf "list: %s" (Errors.message e))

let delete_unknown_errors () =
  let st = Memory.empty () in
  Memory.run st (fun () ->
    match Users.delete (User_id.of_email "ghost@x") with
    | Ok _ -> Alcotest.fail "expected error"
    | Error _ -> ())

let tests =
  [ Alcotest.test_case "user put/get roundtrip" `Quick put_get_roundtrip
  ; Alcotest.test_case "user list and delete"   `Quick list_and_delete
  ; Alcotest.test_case "create happy"           `Quick create_happy
  ; Alcotest.test_case "create duplicate conflicts" `Quick create_duplicate_conflicts
  ; Alcotest.test_case "get unknown is Not_found" `Quick get_unknown_is_not_found
  ; Alcotest.test_case "update changes name"    `Quick update_changes_name
  ; Alcotest.test_case "list and delete via logic" `Quick list_and_delete_via_logic
  ; Alcotest.test_case "delete unknown errors"  `Quick delete_unknown_errors
  ]
