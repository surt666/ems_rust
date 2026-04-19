open Ocaml_lambda_test

let has_label_verbs () =
  let k = Edge_kind.Has_label "building" in
  Alcotest.(check string) "sk verb" "has_building" (Edge_kind.sk_verb k);
  Alcotest.(check string) "gsi verb" "parent_of"  (Edge_kind.gsi_verb k)

let has_sensor_verbs () =
  let k = Edge_kind.Has_sensor in
  Alcotest.(check string) "sk verb"  "has_sensor"   (Edge_kind.sk_verb k);
  Alcotest.(check string) "gsi verb" "sensor_of"    (Edge_kind.gsi_verb k)

let blocked_verbs () =
  let k = Edge_kind.Blocked in
  Alcotest.(check string) "sk verb"  "blocked" (Edge_kind.sk_verb k);
  Alcotest.(check string) "gsi verb" "blocks"  (Edge_kind.gsi_verb k)

let roundtrip_to_string () =
  let xs = [ Edge_kind.Has_label "b"; Edge_kind.Has_sensor; Edge_kind.Blocked ] in
  List.iter (fun k ->
    let s = Edge_kind.to_string k in
    match Edge_kind.of_string s with
    | Ok k' -> Alcotest.(check bool) (Printf.sprintf "rt %s" s) true (k = k')
    | Error e -> Alcotest.failf "of_string %S: %s" s e) xs

let tests =
  [ Alcotest.test_case "Has_label verbs"  `Quick has_label_verbs
  ; Alcotest.test_case "Has_sensor verbs" `Quick has_sensor_verbs
  ; Alcotest.test_case "Blocked verbs"    `Quick blocked_verbs
  ; Alcotest.test_case "to/of_string rt"  `Quick roundtrip_to_string
  ]
