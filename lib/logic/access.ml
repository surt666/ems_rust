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

(* Return all ancestor ids of [node_id], root first, self excluded. Reads the
   path from DynamoDB once (1 query) rather than walking parent by parent. *)
let ancestors_of node_id =
  if Node_id.is_root node_id then []
  else
    match Effects.get_node node_id with
    | None -> []
    | Some n ->
        let parts =
          String.split_on_char '|' n.Node.path
          |> List.filter (fun s -> s <> "")
        in
        let own = Node_id.to_string node_id in
        parts
        |> List.filter (fun s -> s <> own)
        |> List.filter_map (fun s ->
             match Node_id.of_string s with
             | Ok id -> Some id
             | Error _ -> None)

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

(* ---- grant edges (Administrates) ---- *)

let grant_administrates ~user_id ~node_id () =
  let* _ =
    match Effects.get_user user_id with
    | Some u -> Ok u
    | None -> Error (Errors.Not_found_user user_id)
  in
  let* _ =
    if Node_id.is_root node_id then Ok ()
    else
      match Effects.get_node node_id with
      | Some _ -> Ok ()
      | None -> Error (Errors.Not_found node_id)
  in
  let created = Effects.now () in
  Effects.put_edge
    ~from_:(User_id.to_string user_id)
    ~to_:(Node_id.to_string node_id)
    ~kind:Edge_kind.Administrates ~name:"" ~created;
  Ok ()

let list_administrated_nodes ~user_id =
  Ok (Effects.list_administrated_nodes user_id)

(* True if the user has an administrates edge to `node_id` or any ancestor
   (including root). *)
let has_admin_access ~user_id ~node_id =
  let grants = Effects.list_administrated_nodes user_id in
  if List.exists (fun g -> Node_id.equal g node_id) grants then true
  else
    List.exists
      (fun a -> List.exists (fun g -> Node_id.equal g a) grants)
      (ancestors_of node_id)
