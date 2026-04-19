type t =
  | Not_found      of Node_id.t
  | Not_found_user of User_id.t
  | Bad_request    of string
  | Validation     of Metadata.error list
  | Schema_missing of Node_id.t
  | Conflict       of string
  | Internal       of string

let to_code = function
  | Not_found _      -> "not_found"
  | Not_found_user _ -> "not_found"
  | Bad_request _    -> "bad_request"
  | Validation _     -> "validation_failed"
  | Schema_missing _ -> "schema_missing"
  | Conflict _       -> "conflict"
  | Internal _       -> "internal"

let http_status = function
  | Bad_request _    -> 400
  | Not_found _      -> 404
  | Not_found_user _ -> 404
  | Validation _     -> 422
  | Schema_missing _ -> 409
  | Conflict _       -> 409
  | Internal _       -> 500

let message = function
  | Not_found id      -> Printf.sprintf "node %s not found" (Node_id.to_string id)
  | Not_found_user id -> Printf.sprintf "user %s not found" (User_id.email id)
  | Bad_request m     -> m
  | Validation _      -> "validation failed"
  | Schema_missing id -> Printf.sprintf "no hn2 schema found above %s" (Node_id.to_string id)
  | Conflict m        -> m
  | Internal m        -> m

let details = function
  | Validation errs ->
      let to_json e =
        `Assoc [
          ("path", `String e.Metadata.path);
          ("message", `String e.Metadata.message);
        ]
      in
      Some (`Assoc [ ("failures", `List (List.map to_json errs)) ])
  | _ -> None
