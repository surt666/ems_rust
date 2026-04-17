open Ocaml_lambda_test

let uuid_sample = Uuidm.of_string "4b6a6f20-0000-0000-0000-000000000001" |> Option.get

let roundtrip () =
  let id = Node_id.make Level.Hn4 uuid_sample in
  let s = Node_id.to_string id in
  Alcotest.(check string) "render" "HN4#4b6a6f20-0000-0000-0000-000000000001" s;
  match Node_id.of_string s with
  | Error e -> Alcotest.failf "of_string %s -> %s" s e
  | Ok id2 ->
      Alcotest.(check int) "same level" (Level.depth (Node_id.level id))
        (Level.depth (Node_id.level id2));
      Alcotest.(check string) "same uuid" (Uuidm.to_string (Node_id.uuid id))
        (Uuidm.to_string (Node_id.uuid id2))

let rejects_bad () =
  List.iter
    (fun s ->
      match Node_id.of_string s with
      | Ok _ -> Alcotest.failf "expected Error on %S" s
      | Error _ -> ())
    [ ""; "HN4"; "HN4#"; "hn4#abc"; "HN10#4b6a6f20-0000-0000-0000-000000000001";
      "HN4#not-a-uuid" ]

let root_constant () =
  Alcotest.(check string) "root id"
    "HN0#root" (Node_id.to_string Node_id.root);
  Alcotest.(check bool) "root is_root" true (Node_id.is_root Node_id.root)

let tests =
  [
    Alcotest.test_case "roundtrip" `Quick roundtrip;
    Alcotest.test_case "rejects bad" `Quick rejects_bad;
    Alcotest.test_case "root constant" `Quick root_constant;
  ]
