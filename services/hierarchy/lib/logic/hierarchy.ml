let ( let* ) = Result.bind

let bad m = Errors.Validation [ { Metadata.path = ""; message = m } ]

let add_under_schema ?label ~parent ~parent_level ~level ~metadata () =
  let* _host, schema = Schema_check.find_for parent in
  let candidates = Schema.edges_between schema parent_level level in
  let* edge_spec =
    match label, candidates with
    | Some l, _ ->
        (match List.find_opt (fun (s : Schema.edge_spec) -> s.label = l) candidates with
         | Some s -> Ok s
         | None ->
             Error (bad (Printf.sprintf "edge %s -> %s (%s) not allowed by schema"
                           (Level.to_string parent_level) (Level.to_string level) l)))
    | None, [ single ] -> Ok single
    | None, [] ->
        Error (bad (Printf.sprintf "edge %s -> %s not allowed by schema"
                      (Level.to_string parent_level) (Level.to_string level)))
    | None, _many ->
        Error (bad (Printf.sprintf "edge %s -> %s is ambiguous; specify label"
                      (Level.to_string parent_level) (Level.to_string level)))
  in
  let* () =
    let specs = Schema.metadata_for schema level in
    match Metadata.validate ~specs metadata with
    | Ok () -> Ok ()
    | Error errs -> Error (Errors.Validation errs)
  in
  let existing =
    Effects.list_children
      ~kind:(Edge_kind.Has_label edge_spec.Schema.label) parent
  in
  let* () =
    match edge_spec.Schema.max with
    | Some m when List.length existing >= m ->
        Error (bad (Printf.sprintf "max %d %s per parent already reached"
                      m edge_spec.Schema.label))
    | _ -> Ok ()
  in
  Ok edge_spec.Schema.label

let resolve_child_level ?label ~parent ~parent_level () =
  match parent_level with
  | Level.Hn0 -> Ok Level.Hn1
  | Level.Hn1 -> Ok Level.Hn2
  | _ ->
      let* _host, schema = Schema_check.find_for parent in
      (match label with
       | Some l ->
           let matches =
             List.filter_map
               (fun (child, specs) ->
                 if List.exists (fun (s : Schema.edge_spec) -> s.label = l) specs
                 then Some child else None)
               (Schema.allowed_children schema parent_level)
           in
           (match matches with
            | [ c ] -> Ok c
            | [] ->
                Error (bad (Printf.sprintf "no edge from %s with label %S"
                              (Level.to_string parent_level) l))
            | _ ->
                Error (bad (Printf.sprintf
                              "label %S matches multiple target levels; specify level"
                              l)))
       | None ->
           (match Level.of_depth (Level.depth parent_level + 1) with
            | Some lv -> Ok lv
            | None ->
                Error (bad (Printf.sprintf "no default child level for %s"
                              (Level.to_string parent_level)))))

let add_node ?label ?schema ?level ~parent ~name ~metadata () =
  let* () =
    match level with
    | Some Level.Hn0 -> Error (Errors.Bad_request "cannot create root via add_node")
    | _ -> Ok ()
  in
  let* parent_node =
    match Effects.get_node parent with
    | None -> Error (Errors.Not_found parent)
    | Some n -> Ok n
  in
  let parent_level = Node_id.level parent_node.Node.id in
  let* level =
    match level with
    | Some lv -> Ok lv
    | None -> resolve_child_level ?label ~parent ~parent_level ()
  in
  let* () =
    if Level.depth parent_level < Level.depth level then Ok ()
    else Error (bad "child depth must exceed parent depth")
  in
  let* edge_label, node_schema =
    match parent_level, level with
    | Level.Hn0, Level.Hn1 ->
        let* () =
          if schema <> None
          then Error (Errors.Bad_request "schema only allowed on hn2 nodes")
          else Ok ()
        in
        Ok (Option.value label ~default:"partner", None)
    | Level.Hn0, _ ->
        Error (bad "root can only contain hn1 (partner) nodes")
    | Level.Hn1, Level.Hn2 ->
        let* sch =
          match schema with
          | None -> Error (Errors.Bad_request "schema is required when creating an hn2 node")
          | Some s ->
              (match Schema.validate s with
               | Ok () -> Ok s
               | Error msg -> Error (Errors.Bad_request (Printf.sprintf "invalid schema: %s" msg)))
        in
        Ok (Option.value label ~default:"company", Some sch)
    | Level.Hn1, _ ->
        Error (bad "partner can only contain hn2 (company) nodes")
    | _ ->
        let* () =
          if schema <> None
          then Error (Errors.Bad_request "schema only allowed on hn2 nodes")
          else Ok ()
        in
        let* lbl = add_under_schema ?label ~parent ~parent_level ~level ~metadata () in
        Ok (lbl, None)
  in
  let created = Effects.now () in
  Effects.add_node ~level ~build:(fun ~id ->
    let child =
      Node.make ~id ~level ~name ~parent
        ~parent_path:parent_node.Node.path
        ~created ~metadata ~schema:node_schema
    in
    let edge =
      { Effects.from_ = Node_id.to_string parent;
        to_ = Node_id.to_string child.Node.id;
        kind = Edge_kind.Has_label edge_label;
        name = child.Node.name;
        created;
        self_path = Some child.Node.path; }
    in
    (child, edge))

let get_node id =
  match Effects.get_node id with
  | Some n -> Ok n
  | None -> Error (Errors.Not_found id)

let list_children ?label parent =
  let kind_arg = Option.map (fun l -> Edge_kind.Has_label l) label in
  Ok (Effects.list_children ?kind:kind_arg parent)

let list_child_refs ?label parent =
  let kind_arg = Option.map (fun l -> Edge_kind.Has_label l) label in
  Ok (Effects.list_child_refs ?kind:kind_arg parent)

let delete_node id =
  match Effects.get_node id with
  | None -> Error (Errors.Not_found id)
  | Some _ ->
      let blockers = Effects.list_blocked_users id in
      List.iter
        (fun user_id ->
          Effects.delete_edge
            ~from_:(User_id.to_string user_id)
            ~to_:(Node_id.to_string id)
            ~kind:Edge_kind.Blocked)
        blockers;
      Effects.delete_node id;
      Ok id
