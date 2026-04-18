open Ocaml_lambda_test

let uuid_of s = Uuidm.of_string s |> Option.get
let ptime_of s = Ptime.of_rfc3339 s |> Result.get_ok |> fun (t, _, _) -> t

let make_sample () : Sensor.t =
  {
    id = Sensor_id.make (uuid_of "11111111-2222-4333-8444-000000000001");
    created = ptime_of "2026-04-18T10:00:00Z";
    parent =
      Node_id.make Level.Hn5
        (uuid_of "22222222-2222-4333-8444-000000000002");
    daq_id = "daq:adeunis_pu_v1:123:0018b210000191c7:counter_a";
    hierarchy_path = "P1#C1#PR1#B2#A1";
    purpose = "Electricity";
    meter_type = Sensor.Counter;
    unit = Some "kWh";
    formula = Formula.Identity;
  }

let fields_preserved () =
  let s = make_sample () in
  Alcotest.(check string) "purpose"     "Electricity" s.purpose;
  Alcotest.(check string) "daq_id"
    "daq:adeunis_pu_v1:123:0018b210000191c7:counter_a" s.daq_id;
  Alcotest.(check bool) "counter meter" true
    (match s.meter_type with Sensor.Counter -> true | _ -> false);
  Alcotest.(check (option string)) "unit" (Some "kWh") s.unit

let meter_type_to_string () =
  Alcotest.(check string) "counter" "counter"
    (Sensor.meter_type_to_string Sensor.Counter);
  Alcotest.(check string) "gauge"   "gauge"
    (Sensor.meter_type_to_string Sensor.Gauge)

let meter_type_of_string () =
  Alcotest.(check bool) "counter parse" true
    (Sensor.meter_type_of_string "counter" = Ok Sensor.Counter);
  Alcotest.(check bool) "gauge parse"   true
    (Sensor.meter_type_of_string "gauge"   = Ok Sensor.Gauge);
  match Sensor.meter_type_of_string "wat" with
  | Ok _ -> Alcotest.fail "should reject unknown meter type"
  | Error _ -> ()

let tests =
  [
    Alcotest.test_case "fields preserved"      `Quick fields_preserved;
    Alcotest.test_case "meter_type_to_string"  `Quick meter_type_to_string;
    Alcotest.test_case "meter_type_of_string"  `Quick meter_type_of_string;
  ]
