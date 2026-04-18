type meter_kind = Counter | Gauge | Either

type t = {
  kind : string;
  min : int option;
  max : int option;
  meter_type : meter_kind;
  purposes : string list option;
}

let validate t =
  if t.kind = "" then Error "sensor slot kind must be non-empty"
  else
    match t.min, t.max with
    | Some a, Some b when a > b ->
        Error (Printf.sprintf "sensor slot %S has min > max" t.kind)
    | _ -> Ok ()

let allows_meter_type slot = function
  | Sensor.Counter -> (match slot.meter_type with Counter | Either -> true | _ -> false)
  | Sensor.Gauge   -> (match slot.meter_type with Gauge   | Either -> true | _ -> false)

let allows_purpose slot purpose =
  match slot.purposes with
  | None -> true
  | Some xs -> List.mem purpose xs
