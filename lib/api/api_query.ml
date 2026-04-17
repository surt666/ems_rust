let param ps k = List.assoc_opt k ps

let err_bad_request m = Api_json.error_response (Errors.Bad_request m)

let get_node ~params =
  match param params "id" with
  | None -> err_bad_request "missing id"
  | Some id ->
      (match Node_id.of_string id with
       | Error e -> err_bad_request e
       | Ok nid ->
           (match Hierarchy.get_node nid with
            | Ok n -> Api_json.ok_response (Api_json.node_to_json n)
            | Error err -> Api_json.error_response err))

let list_children ~params =
  match param params "parent" with
  | None -> err_bad_request "missing parent"
  | Some pid ->
      (match Node_id.of_string pid with
       | Error e -> err_bad_request e
       | Ok p ->
           let label = param params "label" in
           (match Hierarchy.list_children ?label p with
            | Error err -> Api_json.error_response err
            | Ok ns ->
                let body =
                  `Assoc [ ("children", `List (List.map Api_json.node_to_json ns)) ]
                in
                Api_json.ok_response body))

let dispatch ~action ~params =
  match action with
  | "get_node" -> get_node ~params
  | "list_children" -> list_children ~params
  | other -> err_bad_request (Printf.sprintf "unknown query action %S" other)
