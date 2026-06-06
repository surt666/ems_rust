open Ocaml_lambda_hierarchy

let ptime_of s = Ptime.of_rfc3339 s |> Result.get_ok |> fun (t, _, _) -> t

let make_sample () : Sensor.t =
  let parent = Node_id.make Level.Hn5 10042 in
  let id = Sensor_id.make 20001 in
  let parent_path =
    Node_id.to_string Node_id.root ^ "|" ^ Node_id.to_string parent
  in
  {
    id;
    created = ptime_of "2026-04-18T10:00:00Z";
    daq_id = "daq:adeunis_pu_v1:123:0018b210000191c7:counter_a";
    path = parent_path ^ "|" ^ Sensor_id.to_string id;
    purpose = "Electricity";
    meter_type = Sensor.Counter;
    unit = Some "kWh";
    formula = Formula.Identity;
    resample_minutes = Some 15;
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

let parent_id_extracts_last_node_segment () =
  let s = make_sample () in
  let expected = Node_id.make Level.Hn5 10042 in
  Alcotest.(check string) "parent id from path"
    (Node_id.to_string expected)
    (Node_id.to_string (Sensor.parent_id s))

let tests =
  [
    Alcotest.test_case "fields preserved"      `Quick fields_preserved;
    Alcotest.test_case "meter_type_to_string"  `Quick meter_type_to_string;
    Alcotest.test_case "meter_type_of_string"  `Quick meter_type_of_string;
    Alcotest.test_case "parent_id from path"   `Quick parent_id_extracts_last_node_segment;
  ]
