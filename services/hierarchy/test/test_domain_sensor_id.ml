open Ocaml_lambda_hierarchy

let round_trip () =
  let id = Sensor_id.make 10042 in
  let s = Sensor_id.to_string id in
  Alcotest.(check string) "prefixed" "S#10042" s;
  match Sensor_id.of_string s with
  | Ok id2 ->
      Alcotest.(check bool) "equal" true (Sensor_id.equal id id2);
      Alcotest.(check int) "id preserved" (Sensor_id.id id) (Sensor_id.id id2)
  | Error e -> Alcotest.failf "parse failed: %s" e

let rejects_missing_prefix () =
  match Sensor_id.of_string "10042" with
  | Ok _ -> Alcotest.fail "should reject missing S#"
  | Error _ -> ()

let rejects_wrong_prefix () =
  match Sensor_id.of_string "HN4#10042" with
  | Ok _ -> Alcotest.fail "should reject HN4# prefix"
  | Error _ -> ()

let rejects_bad_id () =
  match Sensor_id.of_string "S#not-an-int" with
  | Ok _ -> Alcotest.fail "should reject bad id"
  | Error _ -> ()

let tests =
  [
    Alcotest.test_case "round trip"            `Quick round_trip;
    Alcotest.test_case "rejects missing prefix" `Quick rejects_missing_prefix;
    Alcotest.test_case "rejects wrong prefix"   `Quick rejects_wrong_prefix;
    Alcotest.test_case "rejects bad id"         `Quick rejects_bad_id;
  ]
