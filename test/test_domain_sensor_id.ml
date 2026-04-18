open Ocaml_lambda_test

let sample = Uuidm.of_string "11111111-2222-4333-8444-555555555555" |> Option.get

let round_trip () =
  let id = Sensor_id.make sample in
  let s = Sensor_id.to_string id in
  Alcotest.(check string) "prefixed" "S#11111111-2222-4333-8444-555555555555" s;
  match Sensor_id.of_string s with
  | Ok id2 ->
      Alcotest.(check bool) "equal" true (Sensor_id.equal id id2);
      Alcotest.(check bool) "uuid preserved" true
        (Uuidm.equal (Sensor_id.uuid id2) sample)
  | Error e -> Alcotest.failf "parse failed: %s" e

let rejects_missing_prefix () =
  match Sensor_id.of_string "11111111-2222-4333-8444-555555555555" with
  | Ok _ -> Alcotest.fail "should reject missing S#"
  | Error _ -> ()

let rejects_wrong_prefix () =
  match Sensor_id.of_string "HN4#11111111-2222-4333-8444-555555555555" with
  | Ok _ -> Alcotest.fail "should reject HN4# prefix"
  | Error _ -> ()

let rejects_bad_uuid () =
  match Sensor_id.of_string "S#not-a-uuid" with
  | Ok _ -> Alcotest.fail "should reject bad uuid"
  | Error _ -> ()

let tests =
  [
    Alcotest.test_case "round trip"            `Quick round_trip;
    Alcotest.test_case "rejects missing prefix" `Quick rejects_missing_prefix;
    Alcotest.test_case "rejects wrong prefix"   `Quick rejects_wrong_prefix;
    Alcotest.test_case "rejects bad uuid"       `Quick rejects_bad_uuid;
  ]
