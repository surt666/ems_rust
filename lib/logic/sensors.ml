let ( let* ) = Result.bind

let validation_err msg =
  Errors.Validation [ { Metadata.path = ""; message = msg } ]

let rec walk_refs visited uuid =
  if List.exists (Uuidm.equal uuid) visited then `Cycle
  else
    let id = Sensor_id.make uuid in
    match Effects.get_active_sensor id with
    | None -> `Ok
    | Some s ->
        let next_uuids = Formula.referenced_uuids s.Sensor.formula in
        let visited' = uuid :: visited in
        List.fold_left
          (fun acc u ->
            match acc with
            | `Cycle -> `Cycle
            | `Ok -> walk_refs visited' u)
          `Ok next_uuids

let has_cycle ~self_uuid formula =
  let next = Formula.referenced_uuids formula in
  List.exists
    (fun u ->
      Uuidm.equal u self_uuid
      || walk_refs [ self_uuid ] u = `Cycle)
    next

let attach ?(formula = Formula.Identity) ?unit ~parent
    ~daq_id ~purpose ~meter_type () =
  let* parent_node =
    match Effects.get_node parent with
    | None -> Error (Errors.Not_found parent)
    | Some n -> Ok n
  in
  let parent_level = Node_id.level parent_node.Node.id in
  let* _host, schema = Schema_check.find_for parent in
  let* () =
    if Schema.allows_sensors schema parent_level then Ok ()
    else Error (validation_err
                  (Printf.sprintf "sensors not allowed at %s"
                     (Level.to_string parent_level)))
  in
  let uuid = Effects.gen_uuid () in
  let* () =
    if has_cycle ~self_uuid:uuid formula
    then Error (validation_err "formula refs form a cycle")
    else Ok ()
  in
  let now = Effects.now () in
  let sensor : Sensor.t =
    {
      id = Sensor_id.make uuid;
      created = now;
      parent;
      daq_id;
      hierarchy_path = Node_id.to_string parent;
      purpose;
      meter_type;
      unit;
      formula;
    }
  in
  Effects.put_sensor ~sensor ~parent;
  Ok sensor

let list_active ~parent =
  let ids = Effects.list_sensor_ids parent in
  let xs =
    List.filter_map
      (fun id -> Effects.get_active_sensor id)
      ids
  in
  Ok xs

let get_active id =
  match Effects.get_active_sensor id with
  | Some s -> Ok s
  | None ->
      (* Reuse Not_found by synthesizing a pseudo node-id from the sensor's uuid *)
      Error (Errors.Not_found
               (Node_id.make Level.Hn9 (Sensor_id.uuid id)))

let replace_device ~sensor_id ~new_daq_id () =
  let* old =
    match Effects.get_active_sensor sensor_id with
    | Some s -> Ok s
    | None ->
        Error (Errors.Not_found
                 (Node_id.make Level.Hn9 (Sensor_id.uuid sensor_id)))
  in
  let now = Effects.now () in
  let new_sensor =
    { old with
      Sensor.created = now;
      daq_id = new_daq_id;
    }
  in
  Effects.replace_sensor_device
    ~old_created:old.Sensor.created ~new_sensor;
  Ok new_sensor

let set_formula ~sensor_id ~formula () =
  let* old = get_active sensor_id in
  let self_uuid = Sensor_id.uuid sensor_id in
  let* () =
    if has_cycle ~self_uuid formula
    then Error (validation_err "formula refs form a cycle")
    else Ok ()
  in
  let now = Effects.now () in
  let new_sensor =
    { old with Sensor.created = now; formula }
  in
  Effects.replace_sensor_device
    ~old_created:old.Sensor.created ~new_sensor;
  Ok new_sensor

let rec evaluate id =
  let* s = get_active id in
  let* self_reading =
    match Effects.get_sensor_reading id with
    | Some v -> Ok v
    | None -> Error (Errors.Not_found (Node_id.make Level.Hn9 (Sensor_id.uuid id)))
  in
  match s.Sensor.formula with
  | Formula.Identity -> Ok self_reading
  | Formula.Expr { refs; _ } as f ->
      let rec resolve_all acc = function
        | [] -> Ok (List.rev acc)
        | (alias, uuid) :: rest ->
            let* v = evaluate (Sensor_id.make uuid) in
            resolve_all ((alias, v) :: acc) rest
      in
      let* resolved = resolve_all [] refs in
      let resolve_alias alias =
        match List.assoc_opt alias resolved with
        | Some v -> v
        | None -> raise (Formula.Unknown_ref alias)
      in
      Ok (Formula.eval ~self:self_reading ~resolve:resolve_alias f)
