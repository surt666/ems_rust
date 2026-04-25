(* Locate the HN2 ancestor of [id] and return (hn2_id, schema).
   Reads the starting node's [path] and picks the HN2 segment in one hop,
   then a single get_node on that HN2. *)
let find_for id =
  if Node_id.is_root id then Error (Errors.Schema_missing id)
  else
    match Effects.get_node id with
    | None -> Error (Errors.Not_found id)
    | Some node when Node_id.level node.Node.id = Level.Hn2 ->
        (match node.Node.schema with
         | Some s -> Ok (node.Node.id, s)
         | None -> Error (Errors.Schema_missing id))
    | Some node ->
        (match Node.segment_at_level ~path:node.Node.path ~lvl:Level.Hn2 with
         | None -> Error (Errors.Schema_missing id)
         | Some seg ->
             (match Node_id.of_string seg with
              | Error _ -> Error (Errors.Schema_missing id)
              | Ok hn2_id ->
                  (match Effects.get_node hn2_id with
                   | None -> Error (Errors.Not_found hn2_id)
                   | Some hn2 ->
                       (match hn2.Node.schema with
                        | Some s -> Ok (hn2.Node.id, s)
                        | None -> Error (Errors.Schema_missing hn2_id)))))
