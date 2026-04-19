open Ocaml_lambda_test

let user_id_rt () =
  let id = User_id.of_email "alice@example.com" in
  Alcotest.(check string) "render" "U#alice@example.com" (User_id.to_string id);
  match User_id.of_string "U#alice@example.com" with
  | Ok id' -> Alcotest.(check string) "rt" "alice@example.com" (User_id.email id')
  | Error e -> Alcotest.failf "of_string: %s" e

let user_id_rejects_bad () =
  match User_id.of_string "alice@example.com" with
  | Ok _ -> Alcotest.fail "missing U# prefix should fail"
  | Error _ -> ()

let cognito_group_parses () =
  Alcotest.(check bool) "reader" true
    (Cognito_group.of_string "reader" = Ok Cognito_group.Reader);
  Alcotest.(check bool) "writer" true
    (Cognito_group.of_string "writer" = Ok Cognito_group.Writer);
  Alcotest.(check bool) "admin"  true
    (Cognito_group.of_string "admin"  = Ok Cognito_group.Admin)

let tests =
  [ Alcotest.test_case "user_id roundtrip"   `Quick user_id_rt
  ; Alcotest.test_case "user_id rejects bad" `Quick user_id_rejects_bad
  ; Alcotest.test_case "cognito_group parse" `Quick cognito_group_parses
  ]
