type sensor_row = Active of Sensor.t | History of Sensor.t

type edge = {
  from_ : string;
  to_   : string;
  kind  : Edge_kind.t;
  name  : string;
}

type counter = { mutable n : int; mutable live : int }

type state = {
  nodes    : (string, Node.t) Hashtbl.t;
  edges    : edge list ref;
  sensors  : (string, sensor_row list) Hashtbl.t;
  users    : (string, User.t) Hashtbl.t;
  counters : (string, counter) Hashtbl.t;
  clock    : unit -> Ptime.t;
}

let empty ?(seed = 42) ?(clock = Ptime_clock.now) () =
  let _ = seed in
  {
    nodes = Hashtbl.create 32;
    edges = ref [];
    sensors = Hashtbl.create 32;
    users = Hashtbl.create 32;
    counters = Hashtbl.create 16;
    clock;
  }

let counter_for st key =
  match Hashtbl.find_opt st.counters key with
  | Some c -> c
  | None ->
      let c = { n = 10000; live = 0 } in
      Hashtbl.replace st.counters key c;
      c

let allocate_node_id st level =
  let key = Codec.counter_pk_node level in
  let c = counter_for st key in
  c.n <- c.n + 1;
  c.live <- c.live + 1;
  c.n

let allocate_sensor_id st =
  let c = counter_for st Codec.counter_pk_sensor in
  c.n <- c.n + 1;
  c.live <- c.live + 1;
  c.n

let release_node_count st level =
  let key = Codec.counter_pk_node level in
  let c = counter_for st key in
  c.live <- c.live - 1

let release_sensor_count st =
  let c = counter_for st Codec.counter_pk_sensor in
  c.live <- c.live - 1

let find_node st id =
  Hashtbl.find_opt st.nodes (Node_id.to_string id)

let sensor_rows st id =
  Hashtbl.find_opt st.sensors (Sensor_id.to_string id) |> Option.value ~default:[]

let set_sensor_rows st id rows =
  Hashtbl.replace st.sensors (Sensor_id.to_string id) rows

let active_of_rows rows =
  List.find_map (function Active s -> Some s | History _ -> None) rows

let edge_matches st parent kind_opt =
  List.filter_map
    (fun e ->
      match Node_id.of_string e.from_ with
      | Error _ -> None
      | Ok p when not (Node_id.equal p parent) -> None
      | Ok _ ->
          let keep =
            match kind_opt with
            | None -> true
            | Some k -> e.kind = k
          in
          if not keep then None
          else
            match Node_id.of_string e.to_ with
            | Ok c -> Some (e, c)
            | Error _ -> None)
    !(st.edges)

let push_edge st (es : Effects.edge_spec) =
  st.edges := { from_ = es.from_; to_ = es.to_; kind = es.kind; name = es.name }
              :: !(st.edges)

let run (st : state) (f : unit -> 'a) : 'a =
  let open Effect.Deep in
  try_with f ()
    {
      effc =
        (fun (type a) (eff : a Effect.t) ->
          match eff with
          | Effects.Get_node id ->
              Some (fun (k : (a, _) continuation) ->
                continue k (find_node st id))
          | Effects.Now () ->
              Some (fun (k : (a, _) continuation) ->
                continue k (st.clock ()))
          | Effects.Get_schema id ->
              let schema = Option.bind (find_node st id) (fun n -> n.Node.schema) in
              Some (fun k -> continue k schema)
          | Effects.List_children (parent, kind_opt) ->
              let matches = edge_matches st parent kind_opt in
              let children =
                List.filter_map
                  (fun (_e, c) -> find_node st c)
                  matches
              in
              Some (fun k -> continue k children)
          | Effects.List_child_refs (parent, kind_opt) ->
              let matches = edge_matches st parent kind_opt in
              let refs = List.map (fun (e, c) -> (c, e.name)) matches in
              Some (fun k -> continue k refs)
          | Effects.Add_node { level; build } ->
              let id = allocate_node_id st level in
              let (node, edge) = build ~id in
              Hashtbl.replace st.nodes (Node_id.to_string node.Node.id) node;
              push_edge st edge;
              Some (fun k -> continue k (Ok node))
          | Effects.Put_node n ->
              Hashtbl.replace st.nodes (Node_id.to_string n.Node.id) n;
              Some (fun k -> continue k ())
          | Effects.Put_edge es ->
              push_edge st es;
              Some (fun k -> continue k ())
          | Effects.Delete_node id ->
              let id_s = Node_id.to_string id in
              (match find_node st id with
               | Some n -> release_node_count st (Node.level n)
               | None -> ());
              Hashtbl.remove st.nodes id_s;
              st.edges :=
                List.filter
                  (fun e -> e.from_ <> id_s && e.to_ <> id_s)
                  !(st.edges);
              Some (fun k -> continue k ())
          | Effects.Add_sensor { build } ->
              let id = allocate_sensor_id st in
              let (sensor, edge) = build ~id in
              let rows = sensor_rows st sensor.Sensor.id in
              set_sensor_rows st sensor.Sensor.id (Active sensor :: rows);
              push_edge st edge;
              Some (fun k -> continue k (Ok sensor))
          | Effects.Get_active_sensor id ->
              let v = active_of_rows (sensor_rows st id) in
              Some (fun k -> continue k v)
          | Effects.List_sensor_ids parent ->
              let parent_s = Node_id.to_string parent in
              let ids =
                List.filter_map
                  (fun e ->
                    if e.from_ = parent_s && e.kind = Edge_kind.Has_sensor
                    then
                      match Sensor_id.of_string e.to_ with
                      | Ok id -> Some id
                      | Error _ -> None
                    else None)
                  !(st.edges)
              in
              Some (fun k -> continue k ids)
          | Effects.Replace_sensor_device { old_created; new_sensor } ->
              let id = new_sensor.Sensor.id in
              let rows = sensor_rows st id in
              let demote = function
                | Active s when Ptime.equal s.Sensor.created old_created ->
                    History s
                | other -> other
              in
              let demoted = List.map demote rows in
              set_sensor_rows st id (Active new_sensor :: demoted);
              Some (fun k -> continue k ())
          | Effects.Delete_sensor { sensor_id; parent } ->
              release_sensor_count st;
              Hashtbl.remove st.sensors (Sensor_id.to_string sensor_id);
              let target_s = Sensor_id.to_string sensor_id in
              let parent_s = Node_id.to_string parent in
              st.edges :=
                List.filter
                  (fun e ->
                    not
                      (e.from_ = parent_s
                       && e.kind = Edge_kind.Has_sensor
                       && e.to_ = target_s))
                  !(st.edges);
              Some (fun k -> continue k ())
          | Effects.Get_sensor_reading _id ->
              Some (fun k -> continue k None)
          | Effects.Put_user u ->
              Hashtbl.replace st.users (User_id.to_string u.User.id) u;
              Some (fun k -> continue k ())
          | Effects.Get_user id ->
              let v = Hashtbl.find_opt st.users (User_id.to_string id) in
              Some (fun k -> continue k v)
          | Effects.List_users () ->
              let xs =
                Hashtbl.fold (fun _ u acc -> u :: acc) st.users []
              in
              Some (fun k -> continue k xs)
          | Effects.Delete_user id ->
              Hashtbl.remove st.users (User_id.to_string id);
              Some (fun k -> continue k ())
          | Effects.List_blocked_nodes user_id ->
              let user_s = User_id.to_string user_id in
              let ids =
                List.filter_map
                  (fun e ->
                    match e.kind with
                    | Edge_kind.Blocked when e.from_ = user_s ->
                        (match Node_id.of_string e.to_ with
                         | Ok id -> Some id
                         | Error _ -> None)
                    | _ -> None)
                  !(st.edges)
              in
              Some (fun k -> continue k ids)
          | Effects.List_blocked_users node_id ->
              let node_s = Node_id.to_string node_id in
              let ids =
                List.filter_map
                  (fun e ->
                    match e.kind with
                    | Edge_kind.Blocked when e.to_ = node_s ->
                        (match User_id.of_string e.from_ with
                         | Ok id -> Some id
                         | Error _ -> None)
                    | _ -> None)
                  !(st.edges)
              in
              Some (fun k -> continue k ids)
          | Effects.List_administrated_nodes user_id ->
              let user_s = User_id.to_string user_id in
              let ids =
                List.filter_map
                  (fun e ->
                    match e.kind with
                    | Edge_kind.Administrates when e.from_ = user_s ->
                        (match Node_id.of_string e.to_ with
                         | Ok id -> Some id
                         | Error _ -> None)
                    | _ -> None)
                  !(st.edges)
              in
              Some (fun k -> continue k ids)
          | Effects.Delete_edge { from_; to_; kind } ->
              st.edges :=
                List.filter
                  (fun e ->
                    not (e.from_ = from_ && e.to_ = to_ && e.kind = kind))
                  !(st.edges);
              Some (fun k -> continue k ())
          | _ -> None);
    }
