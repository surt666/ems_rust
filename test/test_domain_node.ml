open Ocaml_lambda_test

let make_root () =
  let now = Ptime_clock.now () in
  let n = Node.make_root ~created:now in
  Alcotest.(check string) "id" "HN0#root" (Node_id.to_string n.Node.id);
  Alcotest.(check (option string)) "no parent" None
    (Option.map Node_id.to_string n.Node.parent)

let make_child () =
  let uuid = Uuidm.of_string "4b6a6f20-0000-0000-0000-000000000001" |> Option.get in
  let parent = Node_id.root in
  let now = Ptime_clock.now () in
  let n =
    Node.make ~uuid ~level:Level.Hn1 ~name:"Acme" ~parent ~created:now
      ~metadata:(`Assoc []) ~schema:None
  in
  Alcotest.(check string) "id has level prefix" "HN1#4b6a6f20-0000-0000-0000-000000000001"
    (Node_id.to_string n.Node.id);
  Alcotest.(check string) "parent set"
    "HN0#root" (Option.map Node_id.to_string n.Node.parent |> Option.get)

let tests =
  [
    Alcotest.test_case "make_root" `Quick make_root;
    Alcotest.test_case "make child" `Quick make_child;
  ]
