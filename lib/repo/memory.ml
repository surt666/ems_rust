type sensor_row = Active of Sensor.t | History of Sensor.t

type edge = {
  from_ : string;
  to_   : string;
  kind  : Edge_kind.t;
  name  : string;
}

type state = {
  nodes    : (string, Node.t) Hashtbl.t;
  edges    : edge list ref;
  sensors  : (string, sensor_row list) Hashtbl.t;
  users    : (string, User.t) Hashtbl.t;
  rng      : Random.State.t;
  clock    : unit -> Ptime.t;
}

let empty ?(seed = 42) ?(clock = Ptime_clock.now) () =
  {
    nodes = Hashtbl.create 32;
    edges = ref [];
    sensors = Hashtbl.create 32;
    users = Hashtbl.create 32;
    rng = Random.State.make [| seed |];
    clock;
  }

let fresh_uuid rng =
  let bytes = Bytes.create 16 in
  for i = 0 to 15 do Bytes.set_uint8 bytes i (Random.State.int rng 256) done;
  Bytes.set_uint8 bytes 6 (0x40 lor (Bytes.get_uint8 bytes 6 land 0x0f));
  Bytes.set_uint8 bytes 8 (0x80 lor (Bytes.get_uint8 bytes 8 land 0x3f));
  Uuidm.unsafe_of_binary_string (Bytes.unsafe_to_string bytes)

let find_node st id =
  Hashtbl.find_opt st.nodes (Node_id.to_string id)

let sensor_rows st id =
  Hashtbl.find_opt st.sensors (Sensor_id.to_string id) |> Option.value ~default:[]

let set_sensor_rows st id rows =
  Hashtbl.replace st.sensors (Sensor_id.to_string id) rows

let active_of_rows rows =
  List.find_map (function Active s -> Some s | History _ -> None) rows

(* For each edge whose from_ parses to the given parent node, optionally
   filter by Edge_kind. If kind_opt is None, return all child edges whose
   to_ parses as a Node_id (i.e. node-child edges, skipping sensor/other
   non-node targets). If kind_opt is Some k, match edges whose e.kind = k. *)
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

let run (st : state) (f : unit -> 'a) : 'a =
  let open Effect.Deep in
  try_with f ()
    {
      effc =
        (fun (type a) (eff : a Effect.t) ->
          match eff with
          | Effects.Gen_uuid () ->
              Some (fun (k : (a, _) continuation) -> continue k (fresh_uuid st.rng))
          | Effects.Now () ->
              Some (fun k -> continue k (st.clock ()))
          | Effects.Get_node id ->
              Some (fun k -> continue k (find_node st id))
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
          | Effects.Put_node n ->
              Hashtbl.replace st.nodes (Node_id.to_string n.Node.id) n;
              Some (fun k -> continue k ())
          | Effects.Put_edge { from_; to_; kind; name; created = _ } ->
              st.edges := { from_; to_; kind; name } :: !(st.edges);
              Some (fun k -> continue k ())
          | Effects.Delete_node id ->
              let id_s = Node_id.to_string id in
              Hashtbl.remove st.nodes id_s;
              st.edges :=
                List.filter
                  (fun e -> e.from_ <> id_s && e.to_ <> id_s)
                  !(st.edges);
              Some (fun k -> continue k ())
          | Effects.Put_sensor { sensor; parent } ->
              let rows = sensor_rows st sensor.Sensor.id in
              set_sensor_rows st sensor.Sensor.id (Active sensor :: rows);
              st.edges :=
                {
                  from_ = Node_id.to_string parent;
                  to_   = Sensor_id.to_string sensor.Sensor.id;
                  kind  = Edge_kind.Has_sensor;
                  name  = "";
                }
                :: !(st.edges);
              Some (fun k -> continue k ())
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
          | _ -> None);
    }
