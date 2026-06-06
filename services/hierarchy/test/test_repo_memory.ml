open Ocaml_lambda_hierarchy

let runs_pure_fn_and_returns_now () =
  let result =
    Memory.run (Memory.empty ())
      (fun () ->
        let t = Effects.now () in
        Ptime.to_rfc3339 t)
  in
  Alcotest.(check bool) "now is non-empty" true (String.length result > 0)

let get_missing_returns_none () =
  let id =
    Node_id.of_string "HN4#10042" |> Result.get_ok
  in
  let result = Memory.run (Memory.empty ()) (fun () -> Effects.get_node id) in
  Alcotest.(check bool) "missing node -> None" true (Option.is_none result)

let put_then_get () =
  let st = Memory.empty () in
  let id = Node_id.make Level.Hn1 10001 in
  let created = Ptime.epoch in
  let n =
    Node.make ~id:10001 ~level:Level.Hn1 ~name:"Acme"
      ~parent:Node_id.root ~parent_path:(Node_id.to_string Node_id.root) ~created ~metadata:(`Assoc []) ~schema:None
  in
  Memory.run st (fun () -> Effects.put_node n);
  let got = Memory.run st (fun () -> Effects.get_node id) in
  Alcotest.(check bool) "found" true (Option.is_some got)

let list_children_filters_by_label () =
  let st = Memory.empty () in
  let p = Node_id.make Level.Hn3 10003 in
  let mk id name =
    Node.make ~id ~level:Level.Hn4 ~name
      ~parent:p ~parent_path:(Node_id.to_string Node_id.root)
      ~created:Ptime.epoch ~metadata:(`Assoc []) ~schema:None
  in
  let c_a = (mk 10004 "A").Node.id in
  let c_b = (mk 10005 "B").Node.id in
  Memory.run st (fun () ->
    Effects.put_node (mk 10004 "A");
    Effects.put_node (mk 10005 "B");
    Effects.put_edge
      ~from_:(Node_id.to_string p) ~to_:(Node_id.to_string c_a)
      ~kind:(Edge_kind.Has_label "building") ~name:"A"
      ~created:Ptime.epoch ();
    Effects.put_edge
      ~from_:(Node_id.to_string p) ~to_:(Node_id.to_string c_b)
      ~kind:(Edge_kind.Has_label "area") ~name:"B"
      ~created:Ptime.epoch ());
  let all = Memory.run st (fun () -> Effects.list_children p) in
  Alcotest.(check int) "no filter -> 2" 2 (List.length all);
  let bldgs =
    Memory.run st (fun () ->
      Hierarchy.list_children ~label:"building" p |> Result.get_ok)
  in
  Alcotest.(check int) "building -> 1" 1 (List.length bldgs);
  let name = (List.hd bldgs).Node.name in
  Alcotest.(check string) "got the right one" "A" name

let delete_node_removes_edges () =
  let st = Memory.empty () in
  let p = Node_id.make Level.Hn3 10006 in
  let c = Node_id.make Level.Hn4 10007 in
  let node =
    Node.make ~id:10007 ~level:Level.Hn4 ~name:"X"
      ~parent:p ~parent_path:(Node_id.to_string Node_id.root) ~created:Ptime.epoch ~metadata:(`Assoc []) ~schema:None
  in
  Memory.run st (fun () ->
    Effects.put_node node;
    Effects.put_edge
      ~from_:(Node_id.to_string p) ~to_:(Node_id.to_string c)
      ~kind:(Edge_kind.Has_label "building") ~name:"X"
      ~created:Ptime.epoch ();
    Effects.delete_node c);
  let remaining = Memory.run st (fun () -> Effects.list_children p) in
  Alcotest.(check int) "no children after delete" 0 (List.length remaining);
  let got = Memory.run st (fun () -> Effects.get_node c) in
  Alcotest.(check bool) "node gone" true (Option.is_none got)

let tests =
  [
    Alcotest.test_case "now works" `Quick runs_pure_fn_and_returns_now;
    Alcotest.test_case "get_node missing -> None" `Quick get_missing_returns_none;
    Alcotest.test_case "put then get" `Quick put_then_get;
    Alcotest.test_case "list_children label filter" `Quick list_children_filters_by_label;
    Alcotest.test_case "delete removes edges" `Quick delete_node_removes_edges;
  ]

let ptime_of s = Ptime.of_rfc3339 s |> Result.get_ok |> fun (t, _, _) -> t

let mk_sensor ~sensor_id ~parent ~created ~daq : Sensor.t =
  let id = Sensor_id.make sensor_id in
  let path =
    Node_id.to_string Node_id.root
    ^ "|" ^ Node_id.to_string parent
    ^ "|" ^ Sensor_id.to_string id
  in
  {
    id;
    created;
    daq_id = daq;
    path;
    purpose = "Electricity";
    meter_type = Sensor.Counter;
    unit = Some "kWh";
    formula = Formula.Identity;
    resample_minutes = Some 15;
  }

(* For low-level tests we synthesize sensor rows directly into the in-memory
   state rather than going through the atomic alloc, since these tests assert
   storage-shape invariants. *)
let put_sensor_raw st (s : Sensor.t) parent =
  let rows = Memory.sensor_rows st s.Sensor.id in
  Memory.set_sensor_rows st s.Sensor.id (Memory.Active s :: rows);
  Memory.push_edge st
    { Effects.from_ = Node_id.to_string parent;
      to_ = Sensor_id.to_string s.Sensor.id;
      kind = Edge_kind.Has_sensor;
      name = "";
      created = s.created;
      self_path = Some s.path }

let memory_put_and_get_active_sensor () =
  let st = Memory.empty () in
  let parent = Node_id.make Level.Hn4 10042 in
  let s =
    mk_sensor ~sensor_id:20001 ~parent
      ~created:(ptime_of "2026-04-18T10:00:00Z")
      ~daq:"daq:x"
  in
  put_sensor_raw st s parent;
  Memory.run st (fun () ->
    match Effects.get_active_sensor s.Sensor.id with
    | Some s2 ->
        Alcotest.(check string) "daq preserved" "daq:x" s2.Sensor.daq_id
    | None -> Alcotest.fail "expected active")

let memory_list_sensor_ids_returns_attached () =
  let st = Memory.empty () in
  let parent = Node_id.make Level.Hn4 10043 in
  let s1 =
    mk_sensor ~sensor_id:20010 ~parent
      ~created:(ptime_of "2026-04-18T10:00:00Z") ~daq:"daq:a"
  in
  let s2 =
    mk_sensor ~sensor_id:20011 ~parent
      ~created:(ptime_of "2026-04-18T11:00:00Z") ~daq:"daq:b"
  in
  put_sensor_raw st s1 parent;
  put_sensor_raw st s2 parent;
  Memory.run st (fun () ->
    let ids = Effects.list_sensor_ids parent in
    Alcotest.(check int) "two sensors attached" 2 (List.length ids))

let memory_replace_device_demotes_old_and_promotes_new () =
  let st = Memory.empty () in
  let parent = Node_id.make Level.Hn4 10044 in
  let old_t = ptime_of "2026-03-01T00:00:00Z" in
  let new_t = ptime_of "2026-04-18T10:00:00Z" in
  let old_s = mk_sensor ~sensor_id:20020 ~parent ~created:old_t ~daq:"daq:old" in
  let new_s = mk_sensor ~sensor_id:20020 ~parent ~created:new_t ~daq:"daq:new" in
  put_sensor_raw st old_s parent;
  Memory.run st (fun () ->
    Effects.replace_sensor_device ~old_created:old_t ~new_sensor:new_s;
    match Effects.get_active_sensor old_s.Sensor.id with
    | Some s ->
        Alcotest.(check string) "active daq is the new one"
          "daq:new" s.Sensor.daq_id
    | None -> Alcotest.fail "expected active after replace")

let tests =
  tests @
  [
    Alcotest.test_case "put + get_active"         `Quick memory_put_and_get_active_sensor;
    Alcotest.test_case "list_sensor_ids"          `Quick memory_list_sensor_ids_returns_attached;
    Alcotest.test_case "replace_device"           `Quick memory_replace_device_demotes_old_and_promotes_new;
  ]

let blocked_list_and_delete () =
  let st = Memory.empty () in
  let node_id = Node_id.make Level.Hn4 10100 in
  let user_id = User_id.of_email "alice@example.com" in
  Memory.run st (fun () ->
    Effects.put_edge
      ~from_:(User_id.to_string user_id)
      ~to_:(Node_id.to_string node_id)
      ~kind:Edge_kind.Blocked
      ~name:""
      ~created:Ptime.epoch ();
    let blocked = Effects.list_blocked_nodes user_id in
    Alcotest.(check int) "one block" 1 (List.length blocked);
    Alcotest.(check string) "right node"
      (Node_id.to_string node_id)
      (Node_id.to_string (List.hd blocked));
    let blockers = Effects.list_blocked_users node_id in
    Alcotest.(check int) "one blocker" 1 (List.length blockers);
    Effects.delete_edge
      ~from_:(User_id.to_string user_id)
      ~to_:(Node_id.to_string node_id)
      ~kind:Edge_kind.Blocked;
    Alcotest.(check int) "cleared"
      0 (List.length (Effects.list_blocked_nodes user_id)))

let tests =
  tests @
  [
    Alcotest.test_case "blocked list and delete" `Quick blocked_list_and_delete;
  ]
