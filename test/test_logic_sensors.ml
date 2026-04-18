open Ocaml_lambda_test

let uuid_of s = Uuidm.of_string s |> Option.get

let sample_schema () : Schema.t =
  Schema.{
    version = 1;
    edges = [
      (Level.Hn2, [ (Level.Hn3, [ { label = "building"; min = None; max = None } ]) ]);
    ];
    metadata = [];
    sensors = [
      (Level.Hn3, [
        Sensor_slot.{
          kind = "electricity"; min = None; max = Some 1;
          meter_type = Either; purposes = Some [ "Electricity" ];
        };
      ]);
    ];
  }

let seed_company st =
  let c2 = Node_id.make Level.Hn2 (uuid_of "4b6a6f20-0000-0000-0000-00000000cccc") in
  let n2 =
    Node.make ~uuid:(Node_id.uuid c2) ~level:Level.Hn2 ~name:"Acme"
      ~parent:Node_id.root ~created:Ptime.epoch
      ~metadata:(`Assoc []) ~schema:(Some (sample_schema ()))
  in
  Memory.run st (fun () -> Effects.put_node n2);
  c2

let seed_building st c2 =
  Memory.run st (fun () ->
    match
      Hierarchy.add_node
        ~parent:c2 ~level:Level.Hn3 ~name:"B" ~metadata:(`Assoc []) ()
    with
    | Ok b -> b.Node.id
    | Error e -> Alcotest.failf "seed: %s" (Errors.message e))

let attach_happy () =
  let st = Memory.empty () in
  let c2 = seed_company st in
  let bldg = seed_building st c2 in
  Memory.run st (fun () ->
    match
      Sensors.attach
        ~parent:bldg ~kind:"electricity" ~daq_address:"daq:1"
        ~purpose:"Electricity" ~meter_type:Sensor.Counter ~unit:"kWh"
        ~formula:Formula.Identity ()
    with
    | Error e -> Alcotest.failf "attach: %s" (Errors.message e)
    | Ok s ->
        Alcotest.(check string) "daq" "daq:1" s.Sensor.daq_address;
        Alcotest.(check string) "parent wired"
          (Node_id.to_string bldg) (Node_id.to_string s.Sensor.parent))

let rejects_missing_kind_slot () =
  let st = Memory.empty () in
  let c2 = seed_company st in
  let bldg = seed_building st c2 in
  Memory.run st (fun () ->
    match
      Sensors.attach
        ~parent:bldg ~kind:"water" ~daq_address:"daq:2"
        ~purpose:"Water" ~meter_type:Sensor.Counter ()
    with
    | Ok _ -> Alcotest.fail "expected Validation"
    | Error (Errors.Validation _) -> ()
    | Error e -> Alcotest.failf "wrong error: %s" (Errors.message e))

let rejects_max_exceeded () =
  let st = Memory.empty () in
  let c2 = seed_company st in
  let bldg = seed_building st c2 in
  Memory.run st (fun () ->
    let _ = Sensors.attach ~parent:bldg ~kind:"electricity"
              ~daq_address:"daq:a" ~purpose:"Electricity"
              ~meter_type:Sensor.Counter () in
    match
      Sensors.attach ~parent:bldg ~kind:"electricity"
        ~daq_address:"daq:b" ~purpose:"Electricity"
        ~meter_type:Sensor.Counter ()
    with
    | Ok _ -> Alcotest.fail "max=1 should have rejected second attach"
    | Error (Errors.Validation _) -> ()
    | Error e -> Alcotest.failf "wrong error: %s" (Errors.message e))

let rejects_purpose_not_in_whitelist () =
  let st = Memory.empty () in
  let c2 = seed_company st in
  let bldg = seed_building st c2 in
  Memory.run st (fun () ->
    match
      Sensors.attach ~parent:bldg ~kind:"electricity"
        ~daq_address:"daq:x" ~purpose:"Water"
        ~meter_type:Sensor.Counter ()
    with
    | Ok _ -> Alcotest.fail "purpose Water should be rejected"
    | Error (Errors.Validation _) -> ()
    | Error e -> Alcotest.failf "wrong error: %s" (Errors.message e))

let rejects_meter_type_mismatch () =
  let st = Memory.empty () in
  let c2 =
    Memory.run st (fun () ->
      let u = uuid_of "4b6a6f20-0000-0000-0000-00000000dddd" in
      let sch : Schema.t =
        { (sample_schema ()) with
          sensors = [
            (Level.Hn3, [
              Sensor_slot.{
                kind = "electricity"; min = None; max = None;
                meter_type = Counter; purposes = None;
              };
            ]);
          ];
        }
      in
      let n = Node.make ~uuid:u ~level:Level.Hn2 ~name:"C"
                ~parent:Node_id.root ~created:Ptime.epoch
                ~metadata:(`Assoc []) ~schema:(Some sch)
      in
      Effects.put_node n;
      Node_id.make Level.Hn2 u)
  in
  let bldg = seed_building st c2 in
  Memory.run st (fun () ->
    match
      Sensors.attach ~parent:bldg ~kind:"electricity"
        ~daq_address:"daq:y" ~purpose:"Electricity"
        ~meter_type:Sensor.Gauge ()
    with
    | Ok _ -> Alcotest.fail "gauge should be rejected in Counter slot"
    | Error (Errors.Validation _) -> ()
    | Error e -> Alcotest.failf "wrong error: %s" (Errors.message e))

let list_active_returns_attached () =
  let st = Memory.empty () in
  let c2 = seed_company st in
  let bldg = seed_building st c2 in
  Memory.run st (fun () ->
    let _ = Sensors.attach ~parent:bldg ~kind:"electricity"
              ~daq_address:"daq:1" ~purpose:"Electricity"
              ~meter_type:Sensor.Counter () in
    match Sensors.list_active ~parent:bldg with
    | Error e -> Alcotest.failf "list: %s" (Errors.message e)
    | Ok xs ->
        Alcotest.(check int) "one sensor" 1 (List.length xs);
        let s = List.hd xs in
        Alcotest.(check string) "daq" "daq:1" s.Sensor.daq_address)

let list_active_empty_when_none () =
  let st = Memory.empty () in
  let c2 = seed_company st in
  let bldg = seed_building st c2 in
  Memory.run st (fun () ->
    match Sensors.list_active ~parent:bldg with
    | Error e -> Alcotest.failf "list: %s" (Errors.message e)
    | Ok xs -> Alcotest.(check int) "zero" 0 (List.length xs))

let get_active_happy () =
  let st = Memory.empty () in
  let c2 = seed_company st in
  let bldg = seed_building st c2 in
  Memory.run st (fun () ->
    let s =
      match
        Sensors.attach ~parent:bldg ~kind:"electricity"
          ~daq_address:"daq:k" ~purpose:"Electricity"
          ~meter_type:Sensor.Counter ()
      with
      | Ok x -> x
      | Error e -> Alcotest.failf "attach: %s" (Errors.message e)
    in
    match Sensors.get_active s.Sensor.id with
    | Ok s2 -> Alcotest.(check string) "daq" "daq:k" s2.Sensor.daq_address
    | Error e -> Alcotest.failf "get: %s" (Errors.message e))

let get_active_unknown () =
  let st = Memory.empty () in
  Memory.run st (fun () ->
    let id = Sensor_id.make (uuid_of "ffffffff-0000-4000-8000-000000000001") in
    match Sensors.get_active id with
    | Ok _ -> Alcotest.fail "expected not found"
    | Error (Errors.Not_found _) -> ()
    | Error e -> Alcotest.failf "wrong error: %s" (Errors.message e))

let replace_device_promotes_new () =
  let st = Memory.empty () in
  let c2 = seed_company st in
  let bldg = seed_building st c2 in
  Memory.run st (fun () ->
    let s =
      match
        Sensors.attach ~parent:bldg ~kind:"electricity"
          ~daq_address:"daq:old" ~purpose:"Electricity"
          ~meter_type:Sensor.Counter ()
      with
      | Ok x -> x
      | Error e -> Alcotest.failf "attach: %s" (Errors.message e)
    in
    match Sensors.replace_device ~sensor_id:s.Sensor.id ~new_daq_address:"daq:new" () with
    | Error e -> Alcotest.failf "replace: %s" (Errors.message e)
    | Ok s2 ->
        Alcotest.(check string) "new daq" "daq:new" s2.Sensor.daq_address;
        (match Sensors.get_active s.Sensor.id with
         | Ok cur ->
             Alcotest.(check string) "active is new" "daq:new" cur.Sensor.daq_address;
             Alcotest.(check bool) "active_from updated" true
               (not (Ptime.equal s.Sensor.active_from cur.Sensor.active_from))
         | Error e -> Alcotest.failf "get: %s" (Errors.message e)))

let replace_unknown_fails () =
  let st = Memory.empty () in
  Memory.run st (fun () ->
    let id = Sensor_id.make (uuid_of "ffffffff-0000-4000-8000-000000000002") in
    match Sensors.replace_device ~sensor_id:id ~new_daq_address:"x" () with
    | Ok _ -> Alcotest.fail "expected Not_found"
    | Error (Errors.Not_found _) -> ()
    | Error e -> Alcotest.failf "wrong error: %s" (Errors.message e))

let tests =
  [
    Alcotest.test_case "attach happy path"       `Quick attach_happy;
    Alcotest.test_case "rejects missing kind"    `Quick rejects_missing_kind_slot;
    Alcotest.test_case "rejects max exceeded"    `Quick rejects_max_exceeded;
    Alcotest.test_case "rejects bad purpose"     `Quick rejects_purpose_not_in_whitelist;
    Alcotest.test_case "rejects meter mismatch"  `Quick rejects_meter_type_mismatch;
    Alcotest.test_case "list_active returns attached" `Quick list_active_returns_attached;
    Alcotest.test_case "list_active empty"            `Quick list_active_empty_when_none;
    Alcotest.test_case "get_active happy"   `Quick get_active_happy;
    Alcotest.test_case "get_active unknown" `Quick get_active_unknown;
    Alcotest.test_case "replace_device promotes new" `Quick replace_device_promotes_new;
    Alcotest.test_case "replace_device unknown"      `Quick replace_unknown_fails;
  ]
