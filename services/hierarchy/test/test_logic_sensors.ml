open Ocaml_lambda_hierarchy

let uuid_of s = Uuidm.of_string s |> Option.get

let sample_schema () : Schema.t =
  Schema.{
    version = 1;
    edges = [
      (Level.Hn2, [ (Level.Hn3, [ { label = "building"; min = None; max = None } ]) ]);
    ];
    metadata = [];
    sensors = [ Level.Hn3 ];
  }

let seed_company st =
  let c2 = Node_id.make Level.Hn2 (uuid_of "4b6a6f20-0000-0000-0000-00000000cccc") in
  let n2 =
    Node.make ~uuid:(Node_id.uuid c2) ~level:Level.Hn2 ~name:"Acme"
      ~parent:Node_id.root ~parent_path:(Node_id.to_string Node_id.root) ~created:Ptime.epoch
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
        ~parent:bldg ~daq_id:"daq:1"
        ~purpose:"Electricity" ~meter_type:Sensor.Counter ~unit:"kWh"
        ~formula:Formula.Identity ()
    with
    | Error e -> Alcotest.failf "attach: %s" (Errors.message e)
    | Ok s ->
        Alcotest.(check string) "daq" "daq:1" s.Sensor.daq_id;
        Alcotest.(check string) "parent wired"
          (Node_id.to_string bldg) (Node_id.to_string s.Sensor.parent))

let rejects_level_not_allowed () =
  let st = Memory.empty () in
  let c2 = seed_company st in
  Memory.run st (fun () ->
    match
      Sensors.attach
        ~parent:c2 ~daq_id:"daq:2"
        ~purpose:"Electricity" ~meter_type:Sensor.Counter ()
    with
    | Ok _ -> Alcotest.fail "expected Validation at disallowed level"
    | Error (Errors.Validation _) -> ()
    | Error e -> Alcotest.failf "wrong error: %s" (Errors.message e))

let list_active_returns_attached () =
  let st = Memory.empty () in
  let c2 = seed_company st in
  let bldg = seed_building st c2 in
  Memory.run st (fun () ->
    let _ = Sensors.attach ~parent:bldg
              ~daq_id:"daq:1" ~purpose:"Electricity"
              ~meter_type:Sensor.Counter () in
    match Sensors.list_active ~parent:bldg with
    | Error e -> Alcotest.failf "list: %s" (Errors.message e)
    | Ok xs ->
        Alcotest.(check int) "one sensor" 1 (List.length xs);
        let s = List.hd xs in
        Alcotest.(check string) "daq" "daq:1" s.Sensor.daq_id)

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
        Sensors.attach ~parent:bldg
          ~daq_id:"daq:k" ~purpose:"Electricity"
          ~meter_type:Sensor.Counter ()
      with
      | Ok x -> x
      | Error e -> Alcotest.failf "attach: %s" (Errors.message e)
    in
    match Sensors.get_active s.Sensor.id with
    | Ok s2 -> Alcotest.(check string) "daq" "daq:k" s2.Sensor.daq_id
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
        Sensors.attach ~parent:bldg
          ~daq_id:"daq:old" ~purpose:"Electricity"
          ~meter_type:Sensor.Counter ()
      with
      | Ok x -> x
      | Error e -> Alcotest.failf "attach: %s" (Errors.message e)
    in
    match Sensors.replace_device ~sensor_id:s.Sensor.id ~new_daq_id:"daq:new" () with
    | Error e -> Alcotest.failf "replace: %s" (Errors.message e)
    | Ok s2 ->
        Alcotest.(check string) "new daq" "daq:new" s2.Sensor.daq_id;
        (match Sensors.get_active s.Sensor.id with
         | Ok cur ->
             Alcotest.(check string) "active is new" "daq:new" cur.Sensor.daq_id;
             Alcotest.(check bool) "created updated" true
               (not (Ptime.equal s.Sensor.created cur.Sensor.created))
         | Error e -> Alcotest.failf "get: %s" (Errors.message e)))

let replace_unknown_fails () =
  let st = Memory.empty () in
  Memory.run st (fun () ->
    let id = Sensor_id.make (uuid_of "ffffffff-0000-4000-8000-000000000002") in
    match Sensors.replace_device ~sensor_id:id ~new_daq_id:"x" () with
    | Ok _ -> Alcotest.fail "expected Not_found"
    | Error (Errors.Not_found _) -> ()
    | Error e -> Alcotest.failf "wrong error: %s" (Errors.message e))

let attach_detects_self_cycle () =
  let st = Memory.empty () in
  let c2 = seed_company st in
  let bldg = seed_building st c2 in
  Memory.run st (fun () ->
    let s =
      match
        Sensors.attach ~parent:bldg
          ~daq_id:"daq:1" ~purpose:"Electricity"
          ~meter_type:Sensor.Counter ()
      with
      | Ok x -> x
      | Error e -> Alcotest.failf "attach: %s" (Errors.message e)
    in
    let cyc =
      Formula.Expr {
        ast = Formula.Ref "self_again";
        refs = [ ("self_again", Sensor_id.uuid s.Sensor.id) ];
      }
    in
    match
      Sensors.set_formula ~sensor_id:s.Sensor.id ~formula:cyc ()
    with
    | Ok _ -> Alcotest.fail "should reject cycle"
    | Error (Errors.Validation _) -> ()
    | Error e -> Alcotest.failf "wrong error: %s" (Errors.message e))

let run_with_readings st readings f =
  Memory.run st (fun () ->
    let open Effect.Deep in
    try_with f ()
      {
        effc =
          (fun (type a) (eff : a Effect.t) ->
            match eff with
            | Effects.Get_sensor_reading id ->
                let v = List.assoc_opt (Sensor_id.to_string id) readings in
                Some (fun (k : (a, _) continuation) -> continue k v)
            | _ -> None);
      })

let evaluate_identity () =
  let st = Memory.empty () in
  let c2 = seed_company st in
  let bldg = seed_building st c2 in
  let s =
    Memory.run st (fun () ->
      match
        Sensors.attach ~parent:bldg
          ~daq_id:"daq:1" ~purpose:"Electricity"
          ~meter_type:Sensor.Counter ()
      with
      | Ok x -> x
      | Error e -> Alcotest.failf "attach: %s" (Errors.message e))
  in
  let readings = [ (Sensor_id.to_string s.Sensor.id, 42.0) ] in
  run_with_readings st readings (fun () ->
    match Sensors.evaluate s.Sensor.id with
    | Ok v -> Alcotest.(check (float 1e-9)) "raw passed through" 42.0 v
    | Error e -> Alcotest.failf "eval: %s" (Errors.message e))

let evaluate_composite () =
  let st = Memory.empty () in
  let c2 = seed_company st in
  let bldg = seed_building st c2 in
  let s4, s3 =
    Memory.run st (fun () ->
      let s4 =
        match
          Sensors.attach ~parent:bldg
            ~daq_id:"daq:4" ~purpose:"Electricity"
            ~meter_type:Sensor.Counter ()
        with
        | Ok x -> x
        | Error e -> Alcotest.failf "attach s4: %s" (Errors.message e)
      in
      let formula_s3 =
        Formula.Expr {
          ast = Formula.Abs (Formula.Sub (Formula.Self, Formula.Ref "s4"));
          refs = [ ("s4", Sensor_id.uuid s4.Sensor.id) ];
        }
      in
      let s3 =
        match
          Sensors.attach ~parent:bldg
            ~daq_id:"daq:3" ~purpose:"Electricity"
            ~meter_type:Sensor.Counter ~formula:formula_s3 ()
        with
        | Ok x -> x
        | Error e -> Alcotest.failf "attach s3: %s" (Errors.message e)
      in
      (s4, s3))
  in
  let readings =
    [
      (Sensor_id.to_string s4.Sensor.id, 3.0);
      (Sensor_id.to_string s3.Sensor.id, 10.0);
    ]
  in
  run_with_readings st readings (fun () ->
    match Sensors.evaluate s3.Sensor.id with
    | Ok v ->
        Alcotest.(check (float 1e-9)) "|10 - 3| = 7" 7.0 v
    | Error e -> Alcotest.failf "eval: %s" (Errors.message e))

let evaluate_zero_short_circuits_reading () =
  let st = Memory.empty () in
  let c2 = seed_company st in
  let bldg = seed_building st c2 in
  let s =
    Memory.run st (fun () ->
      match
        Sensors.attach ~parent:bldg
          ~daq_id:"daq:z" ~purpose:"Electricity"
          ~meter_type:Sensor.Counter ~formula:Formula.Zero ()
      with
      | Ok x -> x
      | Error e -> Alcotest.failf "attach: %s" (Errors.message e))
  in
  (* no readings registered — Zero must not try to fetch one *)
  run_with_readings st [] (fun () ->
    match Sensors.evaluate s.Sensor.id with
    | Ok v -> Alcotest.(check (float 0.0)) "0" 0.0 v
    | Error e -> Alcotest.failf "eval: %s" (Errors.message e))

let tests =
  [
    Alcotest.test_case "attach happy path"       `Quick attach_happy;
    Alcotest.test_case "rejects disallowed level" `Quick rejects_level_not_allowed;
    Alcotest.test_case "list_active returns attached" `Quick list_active_returns_attached;
    Alcotest.test_case "list_active empty"            `Quick list_active_empty_when_none;
    Alcotest.test_case "get_active happy"   `Quick get_active_happy;
    Alcotest.test_case "get_active unknown" `Quick get_active_unknown;
    Alcotest.test_case "replace_device promotes new" `Quick replace_device_promotes_new;
    Alcotest.test_case "replace_device unknown"      `Quick replace_unknown_fails;
    Alcotest.test_case "attach rejects cycle" `Quick attach_detects_self_cycle;
    Alcotest.test_case "evaluate identity"  `Quick evaluate_identity;
    Alcotest.test_case "evaluate composite" `Quick evaluate_composite;
    Alcotest.test_case "evaluate Zero short-circuits reading" `Quick
      evaluate_zero_short_circuits_reading;
  ]
