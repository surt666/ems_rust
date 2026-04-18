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
  let* name     = require_string json "name"      in
  let metadata =
    match field json "metadata" with
    | Some v -> v
    | None -> `Assoc []
  in
  let label =
    match field json "label" with
    | Some (`String s) -> Some s
    | _ -> None
  in
  let* level =
    match field json "level" with
    | None -> Ok None
    | Some (`String s) ->
        (match Level.of_string s with
         | Ok lv -> Ok (Some lv)
         | Error e -> Error e)
    | Some _ -> Error "non-string field \"level\""
  in
  let* schema =
    match field json "schema" with
    | None -> Ok None
    | Some v ->
        (match Api_json.schema_of_json v with
         | Ok s -> Ok (Some s)
         | Error msg -> Error (Printf.sprintf "invalid schema: %s" msg))
  in
  let* parent = Node_id.of_string parent_s in
  match Hierarchy.add_node ?label ?schema ?level ~parent ~name ~metadata () with
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

let run_attach_sensor json =
  let* parent_s = require_string json "parent_id" in
  let* daq      = require_string json "daq_id" in
  let* purpose  = require_string json "purpose"   in
  let* mt_s     = require_string json "meter_type" in
  let unit =
    match field json "unit" with
    | Some (`String u) -> Some u
    | _ -> None
  in
  let* parent     = Node_id.of_string parent_s in
  let* meter_type = Sensor.meter_type_of_string mt_s in
  match Sensors.attach ~parent ~daq_id:daq ~purpose ~meter_type ?unit () with
  | Ok s -> Ok (Api_json.ok_response (Api_json.sensor_to_json s))
  | Error e -> Ok (Api_json.error_response e)

let run_replace_sensor_device json =
  let* id_s = require_string json "sensor_id" in
  let* daq  = require_string json "daq_id" in
  let* id   = Sensor_id.of_string id_s in
  match Sensors.replace_device ~sensor_id:id ~new_daq_id:daq () with
  | Ok s -> Ok (Api_json.ok_response (Api_json.sensor_to_json s))
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
       | Ok "attach_sensor" ->
           (match run_attach_sensor json with
            | Ok resp -> resp
            | Error m -> err_bad_request m)
       | Ok "replace_sensor_device" ->
           (match run_replace_sensor_device json with
            | Ok resp -> resp
            | Error m -> err_bad_request m)
       | Ok other ->
           err_bad_request (Printf.sprintf "unknown command action %S" other))
