open Ocaml_lambda_hierarchy

let roundtrip () =
  let id = Node_id.make Level.Hn4 10042 in
  let s = Node_id.to_string id in
  Alcotest.(check string) "render" "HN4#10042" s;
  match Node_id.of_string s with
  | Error e -> Alcotest.failf "of_string %s -> %s" s e
  | Ok id2 ->
      Alcotest.(check int) "same level" (Level.depth (Node_id.level id))
        (Level.depth (Node_id.level id2));
      Alcotest.(check int) "same id" (Node_id.id id) (Node_id.id id2)

let rejects_bad () =
  List.iter
    (fun s ->
      match Node_id.of_string s with
      | Ok _ -> Alcotest.failf "expected Error on %S" s
      | Error _ -> ())
    [ ""; "HN4"; "HN4#"; "hn4#10"; "HN10#42"; "HN4#not-an-int" ]

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
