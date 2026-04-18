open Ocaml_lambda_test

let validates_ok () =
  let s : Sensor_slot.t =
    { kind = "electricity"; min = None; max = Some 2;
      meter_type = Sensor_slot.Either; purposes = Some [ "Electricity" ] }
  in
  match Sensor_slot.validate s with
  | Ok () -> ()
  | Error e -> Alcotest.failf "unexpected: %s" e

let rejects_empty_kind () =
  let s : Sensor_slot.t =
    { kind = ""; min = None; max = None;
      meter_type = Sensor_slot.Counter; purposes = None }
  in
  match Sensor_slot.validate s with
  | Ok () -> Alcotest.fail "expected error on empty kind"
  | Error _ -> ()

let rejects_min_greater_than_max () =
  let s : Sensor_slot.t =
    { kind = "heat"; min = Some 3; max = Some 1;
      meter_type = Sensor_slot.Gauge; purposes = None }
  in
  match Sensor_slot.validate s with
  | Ok () -> Alcotest.fail "expected error on min>max"
  | Error _ -> ()

let allows_purpose_check () =
  let s : Sensor_slot.t =
    { kind = "heat"; min = None; max = None;
      meter_type = Sensor_slot.Either; purposes = Some [ "Heat"; "Electricity" ] }
  in
  Alcotest.(check bool) "Heat ok"        true  (Sensor_slot.allows_purpose s "Heat");
  Alcotest.(check bool) "Water rejected" false (Sensor_slot.allows_purpose s "Water")

let allows_purpose_unrestricted () =
  let s : Sensor_slot.t =
    { kind = "anything"; min = None; max = None;
      meter_type = Sensor_slot.Either; purposes = None }
  in
  Alcotest.(check bool) "any purpose ok" true (Sensor_slot.allows_purpose s "Anything")

let tests =
  [
    Alcotest.test_case "validates ok"            `Quick validates_ok;
    Alcotest.test_case "rejects empty kind"      `Quick rejects_empty_kind;
    Alcotest.test_case "rejects min>max"         `Quick rejects_min_greater_than_max;
    Alcotest.test_case "purpose whitelist"       `Quick allows_purpose_check;
    Alcotest.test_case "purpose unrestricted"    `Quick allows_purpose_unrestricted;
  ]
