let err_bad_request m = Api_json.error_response (Errors.Bad_request m)

let field j k =
  match Yojson.Safe.Util.member k j with
  | `Null -> None
  | v -> Some v

let require_string j k =
  match field j k with
  | Some (`String s) -> Ok s
  | _ -> Error (Printf.sprintf "missing or non-string field %S" k)

let ( let* ) = Result.bind

let run_add_node json =
  let* parent_s = require_string json "parent_id" in
  let* level_s  = require_string json "level"     in
  let* name     = require_string json "name"      in
  let metadata =
    match field json "metadata" with
    | Some v -> v
    | None -> `Assoc []
  in
  let* parent = Node_id.of_string parent_s in
  let* level  = Level.of_string level_s     in
  match Hierarchy.add_node ~parent ~level ~name ~metadata () with
  | Ok n -> Ok (Api_json.ok_response (Api_json.node_to_json n))
  | Error e -> Ok (Api_json.error_response e)

let run_delete_node json =
  let* id_s = require_string json "id" in
  let* id = Node_id.of_string id_s in
  match Hierarchy.delete_node id with
  | Ok _ ->
      Ok (Api_json.ok_response
            (`Assoc [ ("deleted", `String (Node_id.to_string id)) ]))
  | Error e -> Ok (Api_json.error_response e)

let dispatch ~body =
  match Yojson.Safe.from_string body with
  | exception Yojson.Json_error msg ->
      err_bad_request (Printf.sprintf "invalid JSON: %s" msg)
  | json ->
      (match require_string json "action" with
       | Error m -> err_bad_request m
       | Ok "add_node" ->
           (match run_add_node json with
            | Ok resp -> resp
            | Error m -> err_bad_request m)
       | Ok "delete_node" ->
           (match run_delete_node json with
            | Ok resp -> resp
            | Error m -> err_bad_request m)
       | Ok other ->
           err_bad_request (Printf.sprintf "unknown command action %S" other))
