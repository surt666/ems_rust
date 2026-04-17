open Ocaml_lambda_test

let runs_pure_fn_and_returns_uuid () =
  let result =
    Memory.run (Memory.empty ())
      (fun () ->
        let u = Effects.gen_uuid () in
        let t = Effects.now () in
        ignore t;
        Uuidm.to_string u)
  in
  Alcotest.(check bool) "uuid is non-empty" true (String.length result > 0)

let get_missing_returns_none () =
  let id =
    Node_id.of_string "HN4#4b6a6f20-0000-0000-0000-000000000001" |> Result.get_ok
  in
  let result = Memory.run (Memory.empty ()) (fun () -> Effects.get_node id) in
  Alcotest.(check bool) "missing node -> None" true (Option.is_none result)

let tests =
  [
    Alcotest.test_case "runs pure fn, produces uuid" `Quick runs_pure_fn_and_returns_uuid;
    Alcotest.test_case "get_node missing returns None" `Quick get_missing_returns_none;
  ]
