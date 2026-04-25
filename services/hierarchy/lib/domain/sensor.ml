type meter_type = Counter | Gauge

type t = {
  id : Sensor_id.t;
  created : Ptime.t;
  parent : Node_id.t;
  daq_id : string;
  hierarchy_path : string;
  purpose : string;
  meter_type : meter_type;
  unit : string option;
  formula : Formula.t;
}

let meter_type_to_string = function
  | Counter -> "counter"
  | Gauge   -> "gauge"

let meter_type_of_string = function
  | "counter" -> Ok Counter
  | "gauge"   -> Ok Gauge
  | other     -> Error (Printf.sprintf "unknown meter type %S" other)
