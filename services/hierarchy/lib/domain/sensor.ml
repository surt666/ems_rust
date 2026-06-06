type meter_type = Counter | Gauge

type t = {
  id : Sensor_id.t;
  created : Ptime.t;
  daq_id : string;
  (* Pipe-separated list of ancestor ids from root down to and including
     self. The trailing segment is the sensor id (S#<n>); the segment
     before it is the direct parent node-id (HN<n>#<n>). In storage this
     string is also the gsi1sk attribute. *)
  path : string;
  purpose : string;
  meter_type : meter_type;
  unit : string option;
  formula : Formula.t;
  (* Resample interval in minutes. Typical values: 5, 15, 60. When set,
     must be > 0; the downstream timeseries store uses it to know how to
     resample raw readings onto a fixed time grid. None = no resampling
     configured for this sensor. *)
  resample_minutes : int option;
}

let path_sep = "|"

let child_path ~parent_path ~sensor_id_str =
  parent_path ^ path_sep ^ sensor_id_str

(* Walk the path from the back and return the last HN<n>#<uuid> segment
   as a Node_id. Sensors must always have a node parent, so this raises
   if the path is malformed. *)
let parent_id (t : t) : Node_id.t =
  let parts = String.split_on_char '|' t.path |> List.filter (fun s -> s <> "") in
  let rec last_hn = function
    | [] -> None
    | seg :: rest ->
        (match Node_id.of_string seg with
         | Ok id -> (match last_hn rest with Some x -> Some x | None -> Some id)
         | Error _ -> last_hn rest)
  in
  match last_hn parts with
  | Some id -> id
  | None ->
      failwith (Printf.sprintf "Sensor.parent_id: no node segment in path %S" t.path)

let meter_type_to_string = function
  | Counter -> "counter"
  | Gauge   -> "gauge"

let meter_type_of_string = function
  | "counter" -> Ok Counter
  | "gauge"   -> Ok Gauge
  | other     -> Error (Printf.sprintf "unknown meter type %S" other)
