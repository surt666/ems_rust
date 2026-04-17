open Ocaml_lambda_test

let runs_pure_fn_and_returns_uuid () =
  let result =
    Memory.run (Memory.empty ())
      (fun () ->
        let u = Effects.gen_uuid () in
        let t = Effects.now () in
        ignore t;
        Uuidm.to_string u)
  in
  Alcotest.(check bool) "uuid is non-empty" true (String.length result > 0)

let get_missing_returns_none () =
  let id =
    Node_id.of_string "HN4#4b6a6f20-0000-0000-0000-000000000001" |> Result.get_ok
  in
  let result = Memory.run (Memory.empty ()) (fun () -> Effects.get_node id) in
  Alcotest.(check bool) "missing node -> None" true (Option.is_none result)

let uuid s = Uuidm.of_string s |> Option.get

let put_then_get () =
  let st = Memory.empty () in
  let id = Node_id.make Level.Hn1 (uuid "4b6a6f20-0000-0000-0000-000000000001") in
  let created = Ptime.epoch in
  let n =
    Node.make ~uuid:(Node_id.uuid id) ~level:Level.Hn1 ~name:"Acme"
      ~parent:Node_id.root ~created ~metadata:(`Assoc []) ~schema:None
  in
  Memory.run st (fun () -> Effects.put_node n);
  let got = Memory.run st (fun () -> Effects.get_node id) in
  Alcotest.(check bool) "found" true (Option.is_some got)

let list_children_filters_by_label () =
  let st = Memory.empty () in
  let p = Node_id.make Level.Hn3 (uuid "4b6a6f20-0000-0000-0000-000000000002") in
  let c_a = Node_id.make Level.Hn4 (uuid "4b6a6f20-0000-0000-0000-000000000003") in
  let c_b = Node_id.make Level.Hn4 (uuid "4b6a6f20-0000-0000-0000-000000000004") in
  let mk id name =
    Node.make ~uuid:(Node_id.uuid id) ~level:Level.Hn4 ~name
      ~parent:p ~created:Ptime.epoch ~metadata:(`Assoc []) ~schema:None
  in
  Memory.run st (fun () ->
    Effects.put_node (mk c_a "A");
    Effects.put_node (mk c_b "B");
    Effects.put_edge ~from_:p ~to_:c_a ~label:"building";
    Effects.put_edge ~from_:p ~to_:c_b ~label:"area");
  let all = Memory.run st (fun () -> Effects.list_children p) in
  Alcotest.(check int) "no filter -> 2" 2 (List.length all);
  let bldgs = Memory.run st (fun () -> Effects.list_children ~label:"building" p) in
  Alcotest.(check int) "building -> 1" 1 (List.length bldgs);
  let name = (List.hd bldgs).Node.name in
  Alcotest.(check string) "got the right one" "A" name

let delete_node_removes_edges () =
  let st = Memory.empty () in
  let p = Node_id.make Level.Hn3 (uuid "4b6a6f20-0000-0000-0000-000000000005") in
  let c = Node_id.make Level.Hn4 (uuid "4b6a6f20-0000-0000-0000-000000000006") in
  let node =
    Node.make ~uuid:(Node_id.uuid c) ~level:Level.Hn4 ~name:"X"
      ~parent:p ~created:Ptime.epoch ~metadata:(`Assoc []) ~schema:None
  in
  Memory.run st (fun () ->
    Effects.put_node node;
    Effects.put_edge ~from_:p ~to_:c ~label:"building";
    Effects.delete_node c);
  let remaining = Memory.run st (fun () -> Effects.list_children p) in
  Alcotest.(check int) "no children after delete" 0 (List.length remaining);
  let got = Memory.run st (fun () -> Effects.get_node c) in
  Alcotest.(check bool) "node gone" true (Option.is_none got)

let tests =
  [
    Alcotest.test_case "gen_uuid works" `Quick runs_pure_fn_and_returns_uuid;
    Alcotest.test_case "get_node missing -> None" `Quick get_missing_returns_none;
    Alcotest.test_case "put then get" `Quick put_then_get;
    Alcotest.test_case "list_children label filter" `Quick list_children_filters_by_label;
    Alcotest.test_case "delete removes edges" `Quick delete_node_removes_edges;
  ]
