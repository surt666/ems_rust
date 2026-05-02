open Ocaml_lambda_hierarchy

let sample_schema : Schema.t =
  Schema.{
    version = 1;
    edges = [
      (Level.Hn2, [ (Level.Hn3, [ { label = "property"; min = None; max = Some 2 } ]) ]);
      (Level.Hn3, [ (Level.Hn4, [ { label = "building"; min = Some 1; max = None } ]) ]);
    ];
    metadata = [
      (Level.Hn4, [
        ("lat", Metadata.{ typ = Number { min = Some (-90.); max = Some 90. }; required = true });
      ]);
    ];
    sensors = [];
  }

let parent_path_for_hn2 () =
  Node_id.to_string Node_id.root ^ "|"
  ^ Node_id.to_string (Node_id.make Level.Hn1 10001)

let seed_company st =
  let c2 = Node_id.make Level.Hn2 10002 in
  let n2 =
    Node.make ~id:10002 ~level:Level.Hn2 ~name:"Acme"
      ~parent:Node_id.root ~parent_path:(parent_path_for_hn2 ()) ~created:Ptime.epoch
      ~metadata:(`Assoc []) ~schema:(Some sample_schema)
  in
  Memory.run st (fun () -> Effects.put_node n2);
  c2

let add_property_and_building () =
  let st = Memory.empty () in
  let c2 = seed_company st in
  Memory.run st (fun () ->
    match
      Hierarchy.add_node
        ~parent:c2 ~level:Level.Hn3 ~name:"Ostergade" ~metadata:(`Assoc []) ()
    with
    | Error e -> Alcotest.failf "%s" (Errors.message e)
    | Ok prop ->
        (match
          Hierarchy.add_node
            ~parent:prop.Node.id ~level:Level.Hn4 ~name:"B1"
            ~metadata:(`Assoc [ ("lat", `Float 55.) ]) ()
        with
        | Error e -> Alcotest.failf "%s" (Errors.message e)
        | Ok b1 ->
            Alcotest.(check string) "parent wired"
              (Node_id.to_string prop.Node.id)
              (Option.map Node_id.to_string b1.Node.parent |> Option.get)))

let rejects_disallowed_edge () =
  let st = Memory.empty () in
  let c2 = seed_company st in
  Memory.run st (fun () ->
    match
      Hierarchy.add_node
        ~parent:c2 ~level:Level.Hn5 ~name:"bad" ~metadata:(`Assoc []) ()
    with
    | Ok _ -> Alcotest.fail "expected Validation"
    | Error (Errors.Validation _) -> ()
    | Error e -> Alcotest.failf "wrong error: %s" (Errors.message e))

let rejects_bad_metadata () =
  let st = Memory.empty () in
  let c2 = seed_company st in
  Memory.run st (fun () ->
    let (let*) = Result.bind in
    match
      let* prop =
        Hierarchy.add_node
          ~parent:c2 ~level:Level.Hn3 ~name:"P" ~metadata:(`Assoc []) ()
      in
      Hierarchy.add_node
        ~parent:prop.Node.id ~level:Level.Hn4 ~name:"B"
        ~metadata:(`Assoc [ ("lat", `Float 200.) ]) ()
    with
    | Ok _ -> Alcotest.fail "expected Validation"
    | Error (Errors.Validation errs) ->
        Alcotest.(check bool) "has a failure on lat" true
          (List.exists (fun e -> e.Metadata.path = "lat") errs)
    | Error e -> Alcotest.failf "wrong error: %s" (Errors.message e))

let enforces_cardinality_max () =
  let st = Memory.empty () in
  let c2 = seed_company st in
  Memory.run st (fun () ->
    let _ = Hierarchy.add_node ~parent:c2 ~level:Level.Hn3 ~name:"P1" ~metadata:(`Assoc []) () in
    let _ = Hierarchy.add_node ~parent:c2 ~level:Level.Hn3 ~name:"P2" ~metadata:(`Assoc []) () in
    match Hierarchy.add_node ~parent:c2 ~level:Level.Hn3 ~name:"P3" ~metadata:(`Assoc []) () with
    | Ok _ -> Alcotest.fail "expected Validation on max=2 exceeded"
    | Error (Errors.Validation _) -> ()
    | Error e -> Alcotest.failf "wrong error: %s" (Errors.message e))

let seed_root st =
  Memory.run st (fun () ->
    Effects.put_node (Node.make_root ~created:Ptime.epoch))

let creates_partner_under_root () =
  let st = Memory.empty () in
  seed_root st;
  Memory.run st (fun () ->
    match
      Hierarchy.add_node ~parent:Node_id.root ~level:Level.Hn1
        ~name:"Acme Group" ~metadata:(`Assoc []) ()
    with
    | Error e -> Alcotest.failf "%s" (Errors.message e)
    | Ok n ->
        Alcotest.(check bool) "is hn1" true (Node_id.level n.Node.id = Level.Hn1);
        Alcotest.(check bool) "no schema on partner" true (n.Node.schema = None))

let rejects_root_creation () =
  let st = Memory.empty () in
  seed_root st;
  Memory.run st (fun () ->
    match
      Hierarchy.add_node ~parent:Node_id.root ~level:Level.Hn0
        ~name:"root2" ~metadata:(`Assoc []) ()
    with
    | Ok _ -> Alcotest.fail "expected Bad_request"
    | Error (Errors.Bad_request _) -> ()
    | Error e -> Alcotest.failf "wrong error: %s" (Errors.message e))

let creates_company_with_schema () =
  let st = Memory.empty () in
  seed_root st;
  Memory.run st (fun () ->
    let partner =
      match
        Hierarchy.add_node ~parent:Node_id.root ~level:Level.Hn1
          ~name:"Group" ~metadata:(`Assoc []) ()
      with
      | Ok n -> n.Node.id
      | Error e -> Alcotest.failf "partner: %s" (Errors.message e)
    in
    match
      Hierarchy.add_node ~parent:partner ~level:Level.Hn2
        ~name:"Acme" ~metadata:(`Assoc []) ~schema:sample_schema ()
    with
    | Error e -> Alcotest.failf "%s" (Errors.message e)
    | Ok n ->
        Alcotest.(check bool) "has schema" true (n.Node.schema <> None))

let company_without_schema_fails () =
  let st = Memory.empty () in
  seed_root st;
  Memory.run st (fun () ->
    let partner =
      match
        Hierarchy.add_node ~parent:Node_id.root ~level:Level.Hn1
          ~name:"Group" ~metadata:(`Assoc []) ()
      with
      | Ok n -> n.Node.id
      | Error e -> Alcotest.failf "partner: %s" (Errors.message e)
    in
    match
      Hierarchy.add_node ~parent:partner ~level:Level.Hn2
        ~name:"Acme" ~metadata:(`Assoc []) ()
    with
    | Ok _ -> Alcotest.fail "expected Bad_request"
    | Error (Errors.Bad_request _) -> ()
    | Error e -> Alcotest.failf "wrong error: %s" (Errors.message e))

let infers_level_default_parent_plus_one () =
  let st = Memory.empty () in
  let c2 = seed_company st in
  Memory.run st (fun () ->
    match
      Hierarchy.add_node ~parent:c2 ~name:"P" ~metadata:(`Assoc []) ()
    with
    | Error e -> Alcotest.failf "%s" (Errors.message e)
    | Ok n ->
        Alcotest.(check bool) "resolved to hn3" true
          (Node_id.level n.Node.id = Level.Hn3))

let infers_level_from_label () =
  let st = Memory.empty () in
  let c2 = seed_company st in
  Memory.run st (fun () ->
    match
      Hierarchy.add_node ~parent:c2 ~label:"property"
        ~name:"P" ~metadata:(`Assoc []) ()
    with
    | Error e -> Alcotest.failf "%s" (Errors.message e)
    | Ok n ->
        Alcotest.(check bool) "resolved to hn3 via label" true
          (Node_id.level n.Node.id = Level.Hn3))

let cross_level_label_schema : Schema.t =
  Schema.{
    version = 1;
    edges = [
      (Level.Hn2, [
        (Level.Hn3, [ { label = "floor";  min = None; max = None } ]);
        (Level.Hn4, [ { label = "zone";   min = None; max = None } ]);
      ]);
    ];
    metadata = [];
    sensors = [];
  }

let infers_level_from_cross_level_label () =
  let st = Memory.empty () in
  let c2 = Node_id.make Level.Hn2 10003 in
  let n2 =
    Node.make ~id:10003 ~level:Level.Hn2 ~name:"X"
      ~parent:Node_id.root ~parent_path:(parent_path_for_hn2 ()) ~created:Ptime.epoch
      ~metadata:(`Assoc []) ~schema:(Some cross_level_label_schema)
  in
  Memory.run st (fun () -> Effects.put_node n2);
  Memory.run st (fun () ->
    match
      Hierarchy.add_node ~parent:c2 ~label:"zone"
        ~name:"Z" ~metadata:(`Assoc []) ()
    with
    | Error e -> Alcotest.failf "%s" (Errors.message e)
    | Ok n ->
        Alcotest.(check bool) "label=zone resolved to hn4" true
          (Node_id.level n.Node.id = Level.Hn4))

let tests =
  [
    Alcotest.test_case "add property + building" `Quick add_property_and_building;
    Alcotest.test_case "rejects disallowed edge" `Quick rejects_disallowed_edge;
    Alcotest.test_case "rejects bad metadata" `Quick rejects_bad_metadata;
    Alcotest.test_case "enforces max cardinality" `Quick enforces_cardinality_max;
    Alcotest.test_case "creates partner under root" `Quick creates_partner_under_root;
    Alcotest.test_case "rejects creating root" `Quick rejects_root_creation;
    Alcotest.test_case "creates company with schema" `Quick creates_company_with_schema;
    Alcotest.test_case "company without schema fails" `Quick company_without_schema_fails;
    Alcotest.test_case "infers level = parent+1 by default" `Quick
      infers_level_default_parent_plus_one;
    Alcotest.test_case "infers level from label" `Quick infers_level_from_label;
    Alcotest.test_case "infers level from cross-level label" `Quick
      infers_level_from_cross_level_label;
  ]
