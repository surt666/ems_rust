open Ocaml_lambda_test

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

let fresh_root cfg =
  Dynamo.run cfg (fun () ->
    let u = Effects.gen_uuid () in
    let c2 = Node_id.make Level.Hn2 u in
    let sch : Schema.t =
      Schema.{
        version = 1;
        edges = [
          (Level.Hn2, [ (Level.Hn3, [ { label = "property"; min = None; max = None } ]) ]);
          (Level.Hn3, [ (Level.Hn4, [ { label = "building"; min = None; max = None } ]) ]);
        ];
        metadata = [
          (Level.Hn4, [
            ("lat", Metadata.{ typ = Number { min = Some (-90.); max = Some 90. }; required = true });
          ]);
        ];
      }
    in
    let n2 =
      Node.make ~uuid:u ~level:Level.Hn2 ~name:"IntegrationCo"
        ~parent:Node_id.root ~created:(Ptime_clock.now ())
        ~metadata:(`Assoc []) ~schema:(Some sch)
    in
    Effects.put_node n2;
    c2)

let cascade_clean cfg id =
  Dynamo.run cfg (fun () -> Effects.delete_node id)

let add_and_get cfg =
  let c2 = fresh_root cfg in
  Dynamo.run cfg (fun () ->
    match
      Hierarchy.add_node ~parent:c2 ~level:Level.Hn3 ~name:"P" ~metadata:(`Assoc []) ()
    with
    | Error e -> Alcotest.failf "add failed: %s" (Errors.message e)
    | Ok p ->
        match
          Hierarchy.add_node ~parent:p.Node.id ~level:Level.Hn4 ~name:"B"
            ~metadata:(`Assoc [ ("lat", `Float 55.0) ]) ()
        with
        | Error e -> Alcotest.failf "add child failed: %s" (Errors.message e)
        | Ok b ->
            match Hierarchy.get_node b.Node.id with
            | Ok b2 ->
                let lat =
                  Yojson.Safe.Util.(b2.Node.metadata |> member "lat" |> to_number)
                in
                Alcotest.(check (float 1e-9)) "lat preserved" 55.0 lat;
                (* teardown *)
                cascade_clean cfg c2
            | Error e -> Alcotest.failf "get failed: %s" (Errors.message e))

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
  }

let chargepoint_schema () : Schema.t =
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
  }

let seed_root_partner_companies cfg =
  Dynamo.run cfg (fun () ->
    let now = Ptime_clock.now () in
    Effects.put_node (Node.make_root ~created:now);
    let p_u = Effects.gen_uuid () in
    let partner_id = Node_id.make Level.Hn1 p_u in
    let partner =
      Node.make ~uuid:p_u ~level:Level.Hn1 ~name:"Acme Partner"
        ~parent:Node_id.root ~created:now
        ~metadata:(`Assoc []) ~schema:None
    in
    Effects.put_node partner;
    Effects.put_edge ~from_:Node_id.root ~to_:partner_id ~label:"partner";
    let mk_company ~name ~schema =
      let u = Effects.gen_uuid () in
      let n =
        Node.make ~uuid:u ~level:Level.Hn2 ~name
          ~parent:partner_id ~created:now
          ~metadata:(`Assoc []) ~schema:(Some schema)
      in
      Effects.put_node n;
      Effects.put_edge ~from_:partner_id ~to_:n.Node.id ~label:"company";
      n.Node.id
    in
    let realestate = mk_company ~name:"RealEstateCo" ~schema:(property_schema ()) in
    let chargeco   = mk_company ~name:"ChargeCo"     ~schema:(chargepoint_schema ()) in
    (partner_id, realestate, chargeco))

let full_tree cfg =
  let _partner_id, realestate_id, chargeco_id = seed_root_partner_companies cfg in
  let bail ctx e = Alcotest.failf "%s: %s" ctx (Errors.message e) in
  Dynamo.run cfg (fun () ->
    let add ?label ~parent ~level ~name ~metadata () =
      match Hierarchy.add_node ?label ~parent ~level ~name ~metadata () with
      | Error e -> bail ("add " ^ name) e
      | Ok n -> n
    in

    (* RealEstateCo: property/group -> building -> area *)
    let bldg_md =
      `Assoc [ ("lat", `Float 55.6761); ("lng", `Float 12.5683) ]
    in
    let prop1 = add ~label:"property" ~parent:realestate_id ~level:Level.Hn3
      ~name:"HQ Property"        ~metadata:(`Assoc []) () in
    let prop2 = add ~label:"property" ~parent:realestate_id ~level:Level.Hn3
      ~name:"Warehouse Property" ~metadata:(`Assoc []) () in
    let _grp  = add ~label:"group"    ~parent:realestate_id ~level:Level.Hn3
      ~name:"Region Group"       ~metadata:(`Assoc []) () in
    let b1_a = add ~parent:prop1.Node.id ~level:Level.Hn4
      ~name:"HQ Building A"        ~metadata:bldg_md () in
    let _b1_b = add ~parent:prop1.Node.id ~level:Level.Hn4
      ~name:"HQ Building B"        ~metadata:bldg_md () in
    let _b2_a = add ~parent:prop2.Node.id ~level:Level.Hn4
      ~name:"Warehouse Building A" ~metadata:bldg_md () in
    let _b2_b = add ~parent:prop2.Node.id ~level:Level.Hn4
      ~name:"Warehouse Building B" ~metadata:bldg_md () in
    let _area = add ~parent:b1_a.Node.id ~level:Level.Hn5
      ~name:"HQ Parking A" ~metadata:(`Assoc []) () in

    (* ChargeCo: parkinglot -> chargingpool -> charger -> plug *)
    let charger_md kw conn =
      `Assoc [ ("power_kw", `Float kw); ("connector", `String conn) ]
    in
    let lot = add ~parent:chargeco_id ~level:Level.Hn3
      ~name:"Parking Lot North" ~metadata:(`Assoc []) () in
    let pool = add ~parent:lot.Node.id ~level:Level.Hn4
      ~name:"Pool A" ~metadata:(`Assoc []) () in
    let charger1 = add ~parent:pool.Node.id ~level:Level.Hn5
      ~name:"CP-01" ~metadata:(charger_md 150. "ccs") () in
    let charger2 = add ~parent:pool.Node.id ~level:Level.Hn5
      ~name:"CP-02" ~metadata:(charger_md 50. "type2") () in
    let _p1_1 = add ~parent:charger1.Node.id ~level:Level.Hn6
      ~name:"Plug 01-A" ~metadata:(`Assoc []) () in
    let _p1_2 = add ~parent:charger1.Node.id ~level:Level.Hn6
      ~name:"Plug 01-B" ~metadata:(`Assoc []) () in
    let _p2_1 = add ~parent:charger2.Node.id ~level:Level.Hn6
      ~name:"Plug 02-A" ~metadata:(`Assoc []) () in

    (* Reject cross-schema moves: property label is not allowed on ChargeCo. *)
    (match Hierarchy.add_node ~label:"property" ~parent:chargeco_id
             ~level:Level.Hn3 ~name:"nope" ~metadata:(`Assoc []) ()
     with
     | Ok _ -> Alcotest.fail "ChargeCo should not accept a 'property' child"
     | Error (Errors.Validation _) -> ()
     | Error e -> Alcotest.failf "wrong error: %s" (Errors.message e));

    Alcotest.(check int) "RealEstateCo has 3 children (2 properties + 1 group)"
      3 (List.length (Effects.list_children realestate_id));
    Alcotest.(check int) "ChargeCo has 1 parkinglot"
      1 (List.length (Effects.list_children chargeco_id));
    Alcotest.(check int) "Pool A has 2 chargers"
      2 (List.length (Effects.list_children pool.Node.id));
    Alcotest.(check int) "CP-01 has 2 plugs"
      2 (List.length (Effects.list_children charger1.Node.id)))
  (* No cleanup: both trees persist in DynamoDB for inspection. *)

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
        Alcotest.test_case "two companies, distinct sub-hierarchies" `Quick (fun () -> full_tree cfg);
      ]);
    ]
