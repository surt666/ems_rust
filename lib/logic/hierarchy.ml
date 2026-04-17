let ( let* ) = Result.bind

let add_node ~parent ~level ~name ~metadata =
  let* parent_node =
    match Effects.get_node parent with
    | None -> Error (Errors.Not_found parent)
    | Some n -> Ok n
  in
  let* () =
    if Level.depth (Node_id.level parent_node.Node.id) < Level.depth level
    then Ok ()
    else Error (Errors.Validation [ { Metadata.path = ""; message = "child depth must exceed parent depth" } ])
  in
  let* _host, schema = Schema_check.find_for parent in
  let* edge_spec =
    match Schema.allowed_child schema (Node_id.level parent_node.Node.id) level with
    | Some s -> Ok s
    | None ->
        Error (Errors.Validation [ {
          Metadata.path = "";
          message = Printf.sprintf "edge %s -> %s not allowed by schema"
            (Level.to_string (Node_id.level parent_node.Node.id))
            (Level.to_string level);
        } ])
  in
  let* () =
    let specs = Schema.metadata_for schema level in
    match Metadata.validate ~specs metadata with
    | Ok () -> Ok ()
    | Error errs -> Error (Errors.Validation errs)
  in
  let existing =
    Effects.list_children ~label:("has_" ^ edge_spec.Schema.label ^ "#") parent
  in
  let* () =
    match edge_spec.Schema.max with
    | Some m when List.length existing >= m ->
        Error (Errors.Validation [ {
          Metadata.path = "";
          message = Printf.sprintf "max %d %s per parent already reached" m edge_spec.Schema.label;
        } ])
    | _ -> Ok ()
  in
  let uuid = Effects.gen_uuid () in
  let created = Effects.now () in
  let child =
    Node.make ~uuid ~level ~name ~parent ~created ~metadata ~schema:None
  in
  Effects.put_node child;
  Effects.put_edge ~from_:parent ~to_:child.Node.id ~label:edge_spec.Schema.label;
  Ok child

let get_node id =
  match Effects.get_node id with
  | Some n -> Ok n
  | None -> Error (Errors.Not_found id)

let list_children ?label parent =
  let label_arg = Option.map (fun l -> "has_" ^ l ^ "#") label in
  Ok (Effects.list_children ?label:label_arg parent)

let delete_node id =
  match Effects.get_node id with
  | None -> Error (Errors.Not_found id)
  | Some _ ->
      Effects.delete_node id;
      Ok id
