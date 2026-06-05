type t =
  | Active of Ptime.t
  | History of Ptime.t

let active_prefix = "active#"

let ptime_to_s t = Ptime.to_rfc3339 ~tz_offset_s:0 t

let to_string = function
  | Active t  -> active_prefix ^ ptime_to_s t
  | History t -> ptime_to_s t

let parse_ts s =
  match Ptime.of_rfc3339 s with
  | Ok (t, _, _) -> Ok t
  | Error _ -> Error (Printf.sprintf "bad timestamp %S" s)

let of_string s =
  let plen = String.length active_prefix in
  if String.starts_with ~prefix:active_prefix s then
    let rest = String.sub s plen (String.length s - plen) in
    match parse_ts rest with
    | Ok t -> Ok (Active t)
    | Error e -> Error e
  else
    match parse_ts s with
    | Ok t -> Ok (History t)
    | Error e -> Error e
