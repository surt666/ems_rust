type t =
  | Has_label of string
  | Has_sensor
  | Blocked

let sk_verb = function
  | Has_label l -> "has_" ^ l
  | Has_sensor  -> "has_sensor"
  | Blocked     -> "blocked"

let gsi_verb = function
  | Has_label _ -> "parent_of"
  | Has_sensor  -> "sensor_of"
  | Blocked     -> "blocks"

let to_string = function
  | Has_label l -> "has_label:" ^ l
  | Has_sensor  -> "has_sensor"
  | Blocked     -> "blocked"

let of_string s =
  if s = "has_sensor" then Ok Has_sensor
  else if s = "blocked" then Ok Blocked
  else match String.split_on_char ':' s with
    | [ "has_label"; l ] when l <> "" -> Ok (Has_label l)
    | _ -> Error (Printf.sprintf "bad edge_kind %S" s)
