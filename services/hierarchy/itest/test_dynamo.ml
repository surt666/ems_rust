open Ocaml_lambda_hierarchy

let require_env name =
  match Sys.getenv_opt name with
  | Some v when v <> "" -> v
  | _ ->
      prerr_endline
        (Printf.sprintf
           "itest: %s must be set (smaws reads AWS_DEFAULT_REGION, not AWS_REGION)"
           name);
      exit 2

let table () = require_env "ITEST_DYNAMO_TABLE"

(* Create root + a partner, then add a company under partner. Returns the
   company id. Uses atomic add_node so ids come from counters. *)
let fresh_partner_and_company cfg ~company_name ~schema =
  Dynamo.run cfg (fun () ->
    let now = Ptime_clock.now () in
    Effects.put_node (Node.make_root ~created:now);
    match
      Hierarchy.add_node ~parent:Node_id.root ~level:Level.Hn1
        ~name:"itest partner" ~metadata:(`Assoc []) ()
    with
    | Error e -> Alcotest.failf "add partner: %s" (Errors.message e)
    | Ok partner ->
        match
          Hierarchy.add_node ~parent:partner.Node.id ~level:Level.Hn2
            ~schema ~name:company_name ~metadata:(`Assoc []) ()
        with
        | Error e -> Alcotest.failf "add company: %s" (Errors.message e)
        | Ok company -> (partner.Node.id, company.Node.id))

let cascade_clean cfg id =
  Dynamo.run cfg (fun () -> Effects.delete_node id)

let property_schema () : Schema.t =
  Schema.{
    version = 1;
    edges = [
      (Level.Hn2, [ (Level.Hn3, [
        { label = "property"; min = None; max = None };
        { label = "group";    min = None; max = None };
      ]) ]);
      (Level.Hn3, [ (Level.Hn4, [ { label = "building"; min = None; max = None } ]) ]);
      (Level.Hn4, [ (Level.Hn5, [ { label = "area";     min = None; max = None } ]) ]);
    ];
    metadata = [
      (Level.Hn4, [
        ("lat", Metadata.{ typ = Number { min = Some (-90.);  max = Some 90. };  required = true });
        ("lng", Metadata.{ typ = Number { min = Some (-180.); max = Some 180. }; required = true });
      ]);
    ];
    sensors = [];
  }

let _chargepoint_schema () : Schema.t =
  Schema.{
    version = 1;
    edges = [
      (Level.Hn2, [ (Level.Hn3, [ { label = "parkinglot";   min = None; max = None } ]) ]);
      (Level.Hn3, [ (Level.Hn4, [ { label = "chargingpool"; min = None; max = None } ]) ]);
      (Level.Hn4, [ (Level.Hn5, [ { label = "charger";      min = None; max = None } ]) ]);
      (Level.Hn5, [ (Level.Hn6, [ { label = "plug";         min = None; max = None } ]) ]);
    ];
    metadata = [
      (Level.Hn5, [
        ("power_kw",  Metadata.{ typ = Number { min = Some 0.; max = None }; required = true });
        ("connector", Metadata.{
          typ = Enum { one_of = [ "ccs"; "type2"; "chademo" ] };
          required = true;
        });
      ]);
    ];
    sensors = [];
  }

let add_and_get cfg =
  let _, c2 = fresh_partner_and_company cfg
                ~company_name:"IntegrationCo" ~schema:(property_schema ()) in
  Dynamo.run cfg (fun () ->
    match
      Hierarchy.add_node ~parent:c2 ~level:Level.Hn3
        ~label:"property" ~name:"P" ~metadata:(`Assoc []) ()
    with
    | Error e -> Alcotest.failf "add failed: %s" (Errors.message e)
    | Ok p ->
        match
          Hierarchy.add_node ~parent:p.Node.id ~level:Level.Hn4 ~name:"B"
            ~metadata:(`Assoc [ ("lat", `Float 55.0); ("lng", `Float 12.5) ]) ()
        with
        | Error e -> Alcotest.failf "add child failed: %s" (Errors.message e)
        | Ok b ->
            match Hierarchy.get_node b.Node.id with
            | Ok b2 ->
                let lat =
                  Yojson.Safe.Util.(b2.Node.metadata |> member "lat" |> to_number)
                in
                Alcotest.(check (float 1e-9)) "lat preserved" 55.0 lat;
                cascade_clean cfg c2
            | Error e -> Alcotest.failf "get failed: %s" (Errors.message e))

let sensor_schema () : Schema.t =
  Schema.{
    version = 1;
    edges = [
      (Level.Hn2, [ (Level.Hn3, [ { label = "property"; min = None; max = None } ]) ]);
      (Level.Hn3, [ (Level.Hn4, [ { label = "building"; min = None; max = None } ]) ]);
      (Level.Hn4, [ (Level.Hn5, [ { label = "area";     min = None; max = None } ]) ]);
    ];
    metadata = [
      (Level.Hn4, [
        ("lat", Metadata.{ typ = Number { min = Some (-90.); max = Some 90. }; required = true });
      ]);
    ];
    sensors = [ Level.Hn4; Level.Hn5 ];
  }

let attach_list_replace cfg =
  let _, c2 = fresh_partner_and_company cfg
                ~company_name:"SensorCo" ~schema:(sensor_schema ()) in
  Dynamo.run cfg (fun () ->
    let bail tag e = Alcotest.failf "%s: %s" tag (Errors.message e) in
    let prop =
      match Hierarchy.add_node ~parent:c2 ~level:Level.Hn3
              ~name:"P" ~metadata:(`Assoc []) () with
      | Ok n -> n | Error e -> bail "add prop" e
    in
    let bldg =
      match Hierarchy.add_node ~parent:prop.Node.id ~level:Level.Hn4
              ~name:"B" ~metadata:(`Assoc [ ("lat", `Float 55.) ]) () with
      | Ok n -> n | Error e -> bail "add bldg" e
    in
    let s =
      match Sensors.attach ~parent:bldg.Node.id
              ~daq_id:"daq:itest:old" ~purpose:"Electricity"
              ~meter_type:Sensor.Counter ~unit:"kWh" ~resample_minutes:15 () with
      | Ok s -> s | Error e -> bail "attach" e
    in
    (match Sensors.list_active ~parent:bldg.Node.id with
     | Ok xs -> Alcotest.(check int) "one sensor attached" 1 (List.length xs)
     | Error e -> bail "list" e);
    (match Sensors.replace_device ~sensor_id:s.Sensor.id
             ~new_daq_id:"daq:itest:new" () with
     | Ok s2 ->
         Alcotest.(check string) "new daq active"
           "daq:itest:new" s2.Sensor.daq_id
     | Error e -> bail "replace" e);
    (match Sensors.get_active s.Sensor.id with
     | Ok s3 ->
         Alcotest.(check string) "read back new"
           "daq:itest:new" s3.Sensor.daq_id
     | Error e -> bail "get" e);
    Effects.delete_sensor ~sensor_id:s.Sensor.id ~parent:bldg.Node.id;
    Effects.delete_node c2)

let () =
  let _ = require_env "AWS_DEFAULT_REGION" in
  let _ = require_env "ITEST_DYNAMO_TABLE" in
  Eio_main.run @@ fun env ->
  Eio.Switch.run @@ fun sw ->
  let ctx = Smaws_Lib.Context.make ~sw env in
  let cfg = Dynamo.{ ctx; table = table () } in
  Alcotest.run "itest.dynamo"
    [
      ("hierarchy", [
        Alcotest.test_case "add + get roundtrip" `Quick (fun () -> add_and_get cfg);
      ]);
      ("sensors", [
        Alcotest.test_case "attach + list + replace" `Quick (fun () -> attach_list_replace cfg);
      ]);
    ]
