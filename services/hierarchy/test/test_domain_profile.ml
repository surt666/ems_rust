open Ocaml_lambda_hierarchy

let group_of profile =
  match Profile.of_string profile with
  | Ok p -> Profile.to_cognito_group p
  | Error e -> Alcotest.failf "of_string %S: %s" profile e

let maps_to_groups () =
  Alcotest.(check bool) "SysAdm->Admin"      true (group_of "SysAdm" = Cognito_group.Admin);
  Alcotest.(check bool) "Developer->Writer"  true (group_of "Developer" = Cognito_group.Writer);
  Alcotest.(check bool) "Standard->Writer"   true (group_of "Standard" = Cognito_group.Writer);
  Alcotest.(check bool) "Technician->Reader" true (group_of "Technician" = Cognito_group.Reader);
  Alcotest.(check bool) "Reader->Reader"     true (group_of "Reader" = Cognito_group.Reader)

let rejects_unknown () =
  match Profile.of_string "Nope" with
  | Ok _ -> Alcotest.fail "unknown profile should fail"
  | Error _ -> ()

let all_roundtrip () =
  List.iter
    (fun p ->
      match Profile.of_string (Profile.to_string p) with
      | Ok p' -> Alcotest.(check bool) "rt" true (p' = p)
      | Error e -> Alcotest.failf "rt: %s" e)
    Profile.all

let tests =
  [ Alcotest.test_case "profile -> cognito group" `Quick maps_to_groups
  ; Alcotest.test_case "rejects unknown profile" `Quick rejects_unknown
  ; Alcotest.test_case "to_string/of_string roundtrip" `Quick all_roundtrip
  ]
