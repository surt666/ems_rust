open Ocaml_lambda_test

let uuid s = Uuidm.of_string s |> Option.get

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

let seed_company st =
  let c2 = Node_id.make Level.Hn2 (uuid "4b6a6f20-0000-0000-0000-00000000aaaa") in
  let n2 =
    Node.make ~uuid:(Node_id.uuid c2) ~level:Level.Hn2 ~name:"Acme"
      ~parent:Node_id.root ~created:Ptime.epoch
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

let tests =
  [
    Alcotest.test_case "add property + building" `Quick add_property_and_building;
    Alcotest.test_case "rejects disallowed edge" `Quick rejects_disallowed_edge;
    Alcotest.test_case "rejects bad metadata" `Quick rejects_bad_metadata;
    Alcotest.test_case "enforces max cardinality" `Quick enforces_cardinality_max;
  ]
