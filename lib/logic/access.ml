let ( let* ) = Result.bind

let block ~user_id ~node_id () =
  let* _ =
    match Effects.get_user user_id with
    | Some u -> Ok u
    | None -> Error (Errors.Not_found_user user_id)
  in
  let* _ =
    match Effects.get_node node_id with
    | Some n -> Ok n
    | None -> Error (Errors.Not_found node_id)
  in
  let created = Effects.now () in
  Effects.put_edge
    ~from_:(User_id.to_string user_id)
    ~to_:(Node_id.to_string node_id)
    ~kind:Edge_kind.Blocked ~name:"" ~created;
  Ok ()

let unblock ~user_id ~node_id () =
  Effects.delete_edge
    ~from_:(User_id.to_string user_id)
    ~to_:(Node_id.to_string node_id)
    ~kind:Edge_kind.Blocked;
  Ok ()

let ancestors_of node_id =
  let rec go acc id =
    match Effects.get_node id with
    | None -> List.rev acc
    | Some n ->
        (match n.Node.parent with
         | Some p -> go (p :: acc) p
         | None -> List.rev acc)
  in
  go [] node_id

let effective_permission ~user_id ~node_id =
  match Effects.get_user user_id with
  | None -> Error (Errors.Not_found_user user_id)
  | Some u ->
      let blocked = Effects.list_blocked_nodes user_id in
      let chain = node_id :: ancestors_of node_id in
      let is_blocked =
        List.exists
          (fun n -> List.exists (fun b -> Node_id.equal b n) blocked)
          chain
      in
      if is_blocked then Ok None
      else Ok (Some u.User.cognito_group)

let list_blocked_nodes ~user_id = Ok (Effects.list_blocked_nodes user_id)
let list_blocked_users ~node_id = Ok (Effects.list_blocked_users node_id)
