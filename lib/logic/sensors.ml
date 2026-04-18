let ( let* ) = Result.bind

let validation_err msg =
  Errors.Validation [ { Metadata.path = ""; message = msg } ]

let attach ?(formula = Formula.Identity) ?unit ~parent ~kind
    ~daq_address ~purpose ~meter_type () =
  let* parent_node =
    match Effects.get_node parent with
    | None -> Error (Errors.Not_found parent)
    | Some n -> Ok n
  in
  let parent_level = Node_id.level parent_node.Node.id in
  let* _host, schema = Schema_check.find_for parent in
  let slots = Schema.sensors_for schema parent_level in
  let* slot =
    match List.find_opt (fun (s : Sensor_slot.t) -> s.kind = kind) slots with
    | Some s -> Ok s
    | None ->
        Error (validation_err
                 (Printf.sprintf "no sensor slot %S at %s"
                    kind (Level.to_string parent_level)))
  in
  let* () =
    if Sensor_slot.allows_meter_type slot meter_type then Ok ()
    else Error (validation_err
                  (Printf.sprintf "meter type not allowed by slot %S" kind))
  in
  let* () =
    if Sensor_slot.allows_purpose slot purpose then Ok ()
    else Error (validation_err
                  (Printf.sprintf "purpose %S not allowed by slot %S"
                     purpose kind))
  in
  let attached = Effects.list_sensor_ids parent in
  let same_kind_count =
    List.fold_left
      (fun acc id ->
        match Effects.get_active_sensor id with
        | Some s when s.Sensor.purpose = purpose || s.Sensor.daq_address = daq_address ->
            acc + 1
        | _ -> acc)
      0 attached
  in
  let* () =
    match slot.max with
    | Some m when same_kind_count >= m ->
        Error (validation_err
                 (Printf.sprintf "max %d sensors of kind %S per parent"
                    m kind))
    | _ -> Ok ()
  in
  let uuid = Effects.gen_uuid () in
  let now = Effects.now () in
  let sensor : Sensor.t =
    {
      id = Sensor_id.make uuid;
      active_from = now;
      parent;
      daq_address;
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
