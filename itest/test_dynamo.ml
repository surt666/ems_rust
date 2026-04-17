open Ocaml_lambda_test

let table () =
  match Sys.getenv_opt "ITEST_DYNAMO_TABLE" with
  | Some t -> t
  | None -> Alcotest.fail "ITEST_DYNAMO_TABLE env must be set"

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
                Alcotest.(check string) "lat preserved"
                  "55" (Yojson.Safe.Util.(b2.Node.metadata |> member "lat" |> to_string));
                (* teardown *)
                cascade_clean cfg c2
            | Error e -> Alcotest.failf "get failed: %s" (Errors.message e))

let () =
  Eio_main.run @@ fun env ->
  Eio.Switch.run @@ fun sw ->
  let ctx = Smaws_Lib.Context.make ~sw env in
  let cfg = Dynamo.{ ctx; table = table () } in
  Alcotest.run "itest.dynamo"
    [
      ("hierarchy", [
        Alcotest.test_case "add + get roundtrip" `Quick (fun () -> add_and_get cfg);
      ]);
    ]
