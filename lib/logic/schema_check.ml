let rec climb_to_hn2 id =
  match Effects.get_node id with
  | None -> Error (Errors.Not_found id)
  | Some node when Node_id.level node.Node.id = Level.Hn2 ->
      (match node.Node.schema with
       | Some s -> Ok (node.Node.id, s)
       | None -> Error (Errors.Schema_missing id))
  | Some node ->
      (match node.Node.parent with
       | None -> Error (Errors.Schema_missing id)
       | Some p -> climb_to_hn2 p)

let find_for = climb_to_hn2
