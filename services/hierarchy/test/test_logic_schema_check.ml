open Ocaml_lambda_hierarchy

let sample_schema : Schema.t =
  Schema.{
    version = 1;
    edges = [
      (Level.Hn2, [ (Level.Hn3, [ { label = "property"; min = None; max = None } ]) ]);
      (Level.Hn3, [ (Level.Hn4, [ { label = "building"; min = None; max = None } ]) ]);
    ];
    metadata = [];
    sensors = [];
  }

let parent_path_for_hn2 () =
  Node_id.to_string Node_id.root ^ "|"
  ^ Node_id.to_string (Node_id.make Level.Hn1 10001)

let find_schema_from_self () =
  let st = Memory.empty () in
  let c2 = Node_id.make Level.Hn2 10010 in
  let n =
    Node.make ~id:10010 ~level:Level.Hn2 ~name:"Acme"
      ~parent:Node_id.root ~parent_path:(parent_path_for_hn2 ()) ~created:Ptime.epoch
      ~metadata:(`Assoc []) ~schema:(Some sample_schema)
  in
  Memory.run st (fun () ->
    Effects.put_node n;
    match Schema_check.find_for c2 with
    | Ok (host, s) ->
        Alcotest.(check string) "host" (Node_id.to_string c2) (Node_id.to_string host);
        Alcotest.(check int) "version" 1 s.Schema.version
    | Error e -> Alcotest.failf "%s" (Errors.message e))

let find_schema_by_walking_up () =
  let st = Memory.empty () in
  let c2 = Node_id.make Level.Hn2 10020 in
  let n2 =
    Node.make ~id:10020 ~level:Level.Hn2 ~name:"Acme"
      ~parent:Node_id.root ~parent_path:(parent_path_for_hn2 ()) ~created:Ptime.epoch
      ~metadata:(`Assoc []) ~schema:(Some sample_schema)
  in
  let c3 = Node_id.make Level.Hn3 10021 in
  let n3 =
    Node.make ~id:10021 ~level:Level.Hn3 ~name:"Ostergade"
      ~parent:c2 ~parent_path:n2.Node.path ~created:Ptime.epoch
      ~metadata:(`Assoc []) ~schema:None
  in
  Memory.run st (fun () ->
    Effects.put_node n2;
    Effects.put_node n3;
    match Schema_check.find_for c3 with
    | Ok (host, _) ->
        Alcotest.(check string) "host is hn2 ancestor"
          (Node_id.to_string c2) (Node_id.to_string host)
    | Error _ -> Alcotest.fail "expected Ok")

let schema_missing_when_no_hn2 () =
  let st = Memory.empty () in
  let c3 = Node_id.make Level.Hn3 10030 in
  let n3 =
    Node.make ~id:10030 ~level:Level.Hn3 ~name:"orphan"
      ~parent:Node_id.root ~parent_path:(Node_id.to_string Node_id.root) ~created:Ptime.epoch
      ~metadata:(`Assoc []) ~schema:None
  in
  Memory.run st (fun () ->
    Effects.put_node n3;
    match Schema_check.find_for c3 with
    | Ok _ -> Alcotest.fail "expected Schema_missing"
    | Error (Errors.Schema_missing _) -> ()
    | Error e -> Alcotest.failf "wrong error: %s" (Errors.message e))

let tests =
  [
    Alcotest.test_case "find from self (hn2)" `Quick find_schema_from_self;
    Alcotest.test_case "walk up to hn2" `Quick find_schema_by_walking_up;
    Alcotest.test_case "schema missing" `Quick schema_missing_when_no_hn2;
  ]
