open Ocaml_lambda_hierarchy

let sample_time () =
  Ptime.of_rfc3339 "2026-04-18T10:00:00Z" |> Result.get_ok |> fun (t, _, _) -> t

let active_encodes_with_prefix () =
  let t = sample_time () in
  Alcotest.(check string) "active format"
    "active#2026-04-18T10:00:00Z"
    (Sensor_sk.to_string (Sensor_sk.Active t))

let history_encodes_without_prefix () =
  let t = sample_time () in
  Alcotest.(check string) "history format"
    "2026-04-18T10:00:00Z"
    (Sensor_sk.to_string (Sensor_sk.History t))

let parse_active () =
  match Sensor_sk.of_string "active#2026-04-18T10:00:00Z" with
  | Ok (Sensor_sk.Active t) ->
      Alcotest.(check string) "ts" "2026-04-18T10:00:00Z"
        (Ptime.to_rfc3339 ~tz_offset_s:0 t)
  | _ -> Alcotest.fail "expected Active"

let parse_history () =
  match Sensor_sk.of_string "2026-03-01T00:00:00Z" with
  | Ok (Sensor_sk.History t) ->
      Alcotest.(check string) "ts" "2026-03-01T00:00:00Z"
        (Ptime.to_rfc3339 ~tz_offset_s:0 t)
  | _ -> Alcotest.fail "expected History"

let rejects_garbage () =
  match Sensor_sk.of_string "garbage" with
  | Ok _ -> Alcotest.fail "should reject"
  | Error _ -> ()

let tests =
  [
    Alcotest.test_case "active format"   `Quick active_encodes_with_prefix;
    Alcotest.test_case "history format"  `Quick history_encodes_without_prefix;
    Alcotest.test_case "parse active"    `Quick parse_active;
    Alcotest.test_case "parse history"   `Quick parse_history;
    Alcotest.test_case "rejects garbage" `Quick rejects_garbage;
  ]
