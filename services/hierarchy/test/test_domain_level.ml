open Ocaml_lambda_hierarchy

let roundtrip () =
  for i = 0 to 9 do
    let s = Printf.sprintf "hn%d" i in
    match Level.of_string s with
    | Error e -> Alcotest.failf "of_string %s -> %s" s e
    | Ok lvl ->
        Alcotest.(check int) "depth matches" i (Level.depth lvl);
        Alcotest.(check string) "render matches" s (Level.to_string lvl)
  done

let rejects_bad_input () =
  List.iter
    (fun s ->
      match Level.of_string s with
      | Ok _ -> Alcotest.failf "expected Error on %S" s
      | Error _ -> ())
    [ ""; "hn"; "hn10"; "hn-1"; "HN0"; "sensor" ]

let tests =
  [
    Alcotest.test_case "hn0..hn9 roundtrip" `Quick roundtrip;
    Alcotest.test_case "rejects malformed" `Quick rejects_bad_input;
  ]
