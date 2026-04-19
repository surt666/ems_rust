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

let tests =
  [ Alcotest.test_case "user put/get roundtrip" `Quick put_get_roundtrip
  ; Alcotest.test_case "user list and delete"   `Quick list_and_delete
  ]
