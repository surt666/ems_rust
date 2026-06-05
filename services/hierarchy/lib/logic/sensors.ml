let ( let* ) = Result.bind

let validation_err msg =
  Errors.Validation [ { Metadata.path = ""; message = msg } ]

let rec walk_refs visited (id : Sensor_id.t) =
  if List.exists (Sensor_id.equal id) visited then `Cycle
  else
    match Effects.get_active_sensor id with
    | None -> `Ok
    | Some s ->
        let next = Formula.referenced_ids s.Sensor.formula in
        let visited' = id :: visited in
        List.fold_left
          (fun acc u ->
            match acc with
            | `Cycle -> `Cycle
            | `Ok -> walk_refs visited' u)
          `Ok next

let has_cycle ~self_id formula =
  let next = Formula.referenced_ids formula in
  List.exists
    (fun u ->
      Sensor_id.equal u self_id
      || walk_refs [ self_id ] u = `Cycle)
    next

(* Construct a Node_id whose level/depth slot is filled by the sensor's
   numeric id. Used as a pseudo-target for Errors.Not_found when a sensor
   is missing — the API path renders S#<id> equivalently. *)
let sensor_not_found (id : Sensor_id.t) =
  Errors.Not_found (Node_id.make Level.Hn9 (Sensor_id.id id))

let attach ?(formula = Formula.Identity) ?unit ?binning ~parent
    ~daq_id ~purpose ~meter_type () =
  let* parent_node =
    match Effects.get_node parent with
    | None -> Error (Errors.Not_found parent)
    | Some n -> Ok n
  in
  let* () =
    match binning with
    | Some b when b <= 0 ->
        Error (validation_err "binning must be > 0 minutes")
    | _ -> Ok ()
  in
  let parent_level = Node_id.level parent_node.Node.id in
  let* _host, schema = Schema_check.find_for parent in
  let* () =
    if Schema.allows_sensors schema parent_level then Ok ()
    else Error (validation_err
                  (Printf.sprintf "sensors not allowed at %s"
                     (Level.to_string parent_level)))
  in
  let now = Effects.now () in
  Effects.add_sensor ~build:(fun ~id ->
    let sid = Sensor_id.make id in
    let path =
      Sensor.child_path
        ~parent_path:parent_node.Node.path
        ~sensor_id_str:(Sensor_id.to_string sid)
    in
    let sensor : Sensor.t =
      {
        id = sid;
        created = now;
        daq_id;
        path;
        purpose;
        meter_type;
        unit;
        formula;
        binning;
      }
    in
    let edge =
      { Effects.from_ = Node_id.to_string parent;
        to_ = Sensor_id.to_string sid;
        kind = Edge_kind.Has_sensor;
        name = "";
        created = now;
        self_path = Some path; }
    in
    (sensor, edge))
  |> Result.map (fun s ->
       (* Sensor cycle check happens after allocation: reject formulas
          that include the freshly-allocated id. The atomic add already
          succeeded, but we run the check post-hoc and roll back via
          delete if it fails. *)
       if has_cycle ~self_id:s.Sensor.id formula then begin
         Effects.delete_sensor ~sensor_id:s.Sensor.id ~parent;
         Error (validation_err "formula refs form a cycle")
       end else Ok s)
  |> Result.join

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
  | None -> Error (sensor_not_found id)

let replace_device ~sensor_id ~new_daq_id () =
  let* old =
    match Effects.get_active_sensor sensor_id with
    | Some s -> Ok s
    | None -> Error (sensor_not_found sensor_id)
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
  let* () =
    if has_cycle ~self_id:sensor_id formula
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
  let reading () =
    match Effects.get_sensor_reading id with
    | Some v -> Ok v
    | None -> Error (sensor_not_found id)
  in
  match s.Sensor.formula with
  | Formula.Zero     -> Ok 0.
  | Formula.Identity -> reading ()
  | Formula.Expr { refs; _ } as f ->
      let* self_reading = reading () in
      let rec resolve_all acc = function
        | [] -> Ok (List.rev acc)
        | (alias, sid) :: rest ->
            let* v = evaluate sid in
            resolve_all ((alias, v) :: acc) rest
      in
      let* resolved = resolve_all [] refs in
      let resolve_alias alias =
        match List.assoc_opt alias resolved with
        | Some v -> v
        | None -> raise (Formula.Unknown_ref alias)
      in
      Ok (Formula.eval ~self:self_reading ~resolve:resolve_alias f)
