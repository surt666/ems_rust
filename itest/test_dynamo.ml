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
          (Level.Hn2, [ (Level.Hn3, { label = "property"; min = None; max = None }) ]);
          (Level.Hn3, [ (Level.Hn4, { label = "building"; min = None; max = None }) ]);
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
      Hierarchy.add_node ~parent:c2 ~level:Level.Hn3 ~name:"P" ~metadata:(`Assoc [])
    with
    | Error e -> Alcotest.failf "add failed: %s" (Errors.message e)
    | Ok p ->
        match
          Hierarchy.add_node ~parent:p.Node.id ~level:Level.Hn4 ~name:"B"
            ~metadata:(`Assoc [ ("lat", `Float 55.0) ])
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

let charge_point_schema () : Schema.t =
  Schema.{
    version = 1;
    edges = [
      (Level.Hn2, [ (Level.Hn3, { label = "property";  min = None;   max = None }) ]);
      (Level.Hn3, [ (Level.Hn4, { label = "building";  min = Some 1; max = None }) ]);
      (Level.Hn4, [ (Level.Hn5, { label = "area";      min = None;   max = None }) ]);
      (Level.Hn5, [ (Level.Hn6, { label = "charger";   min = None;   max = None }) ]);
      (Level.Hn6, [ (Level.Hn7, { label = "plug";      min = None;   max = None }) ]);
    ];
    metadata = [
      (Level.Hn4, [
        ("lat", Metadata.{ typ = Number { min = Some (-90.);  max = Some 90. };  required = true });
        ("lng", Metadata.{ typ = Number { min = Some (-180.); max = Some 180. }; required = true });
      ]);
      (Level.Hn6, [
        ("power_kw",  Metadata.{ typ = Number { min = Some 0.; max = None }; required = true });
        ("connector", Metadata.{
          typ = Enum { one_of = [ "ccs"; "type2"; "chademo" ] };
          required = true;
        });
      ]);
    ];
  }

let seed_root_partner_company cfg =
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
    let c_u = Effects.gen_uuid () in
    let company =
      Node.make ~uuid:c_u ~level:Level.Hn2 ~name:"ChargeCo"
        ~parent:partner_id ~created:now
        ~metadata:(`Assoc []) ~schema:(Some (charge_point_schema ()))
    in
    Effects.put_node company;
    Effects.put_edge ~from_:partner_id ~to_:company.Node.id ~label:"company";
    (partner_id, company.Node.id))

let full_tree cfg =
  let partner_id, company_id = seed_root_partner_company cfg in
  (* Root is a singleton — seed but leave it behind. *)
  let created = ref [ company_id; partner_id ] in
  let bail ctx e = Alcotest.failf "%s: %s" ctx (Errors.message e) in
  Dynamo.run cfg (fun () ->
    let add ~parent ~level ~name ~metadata =
      match Hierarchy.add_node ~parent ~level ~name ~metadata with
      | Error e -> bail ("add " ^ name) e
      | Ok n -> created := n.Node.id :: !created; n
    in
    let property = add ~parent:company_id        ~level:Level.Hn3
      ~name:"HQ Property"   ~metadata:(`Assoc []) in
    let building = add ~parent:property.Node.id  ~level:Level.Hn4
      ~name:"Main Building" ~metadata:(`Assoc [ ("lat", `Float 55.6761); ("lng", `Float 12.5683) ]) in
    let area = add ~parent:building.Node.id  ~level:Level.Hn5
      ~name:"Parking A"     ~metadata:(`Assoc []) in
    let charger = add ~parent:area.Node.id      ~level:Level.Hn6
      ~name:"Charger 01"    ~metadata:(`Assoc [ ("power_kw", `Float 150.); ("connector", `String "ccs") ]) in
    let plug = add ~parent:charger.Node.id   ~level:Level.Hn7
      ~name:"Plug A"        ~metadata:(`Assoc []) in

    let has_child ctx parent child =
      let cs = Effects.list_children parent in
      if not (List.exists (fun (n : Node.t) -> Node_id.equal n.id child) cs)
      then Alcotest.failf "%s: child %s not in %d children"
        ctx (Node_id.to_string child) (List.length cs)
    in
    has_child "root->partner"      Node_id.root        partner_id;
    has_child "partner->company"   partner_id          company_id;
    has_child "company->property"  company_id          property.Node.id;
    has_child "property->building" property.Node.id    building.Node.id;
    has_child "building->area"     building.Node.id    area.Node.id;
    has_child "area->charger"      area.Node.id        charger.Node.id;
    has_child "charger->plug"      charger.Node.id     plug.Node.id;

    (match Hierarchy.get_node charger.Node.id with
     | Error e -> bail "get charger" e
     | Ok c ->
         let pow = Yojson.Safe.Util.(c.Node.metadata |> member "power_kw"  |> to_number) in
         let conn = Yojson.Safe.Util.(c.Node.metadata |> member "connector" |> to_string) in
         Alcotest.(check (float 1e-9)) "power_kw preserved" 150. pow;
         Alcotest.(check string)       "connector preserved" "ccs" conn);

    List.iter (fun id -> Effects.delete_node id) !created)

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
        Alcotest.test_case "full tree partner->plug" `Quick (fun () -> full_tree cfg);
      ]);
    ]
