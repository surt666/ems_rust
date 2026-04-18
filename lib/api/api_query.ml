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
           let full =
             match param params "full" with
             | Some ("true" | "1") -> true
             | _ -> false
           in
           if full then
             (match Hierarchy.list_children ?label p with
              | Error err -> Api_json.error_response err
              | Ok ns ->
                  Api_json.ok_response
                    (`Assoc [ ("children", `List (List.map Api_json.node_to_json ns)) ]))
           else
             (match Hierarchy.list_child_refs ?label p with
              | Error err -> Api_json.error_response err
              | Ok refs ->
                  Api_json.ok_response
                    (`Assoc [ ("children",
                               `List (List.map Api_json.node_ref_to_json refs)) ])))

let list_sensors ~params =
  match param params "parent" with
  | None -> err_bad_request "missing parent"
  | Some pid ->
      (match Node_id.of_string pid with
       | Error e -> err_bad_request e
       | Ok p ->
           (match Sensors.list_active ~parent:p with
            | Error err -> Api_json.error_response err
            | Ok xs ->
                Api_json.ok_response
                  (`Assoc [ ("sensors", `List (List.map Api_json.sensor_to_json xs)) ])))

let get_sensor ~params =
  match param params "id" with
  | None -> err_bad_request "missing id"
  | Some sid ->
      (match Sensor_id.of_string sid with
       | Error e -> err_bad_request e
       | Ok id ->
           (match Sensors.get_active id with
            | Ok s -> Api_json.ok_response (Api_json.sensor_to_json s)
            | Error err -> Api_json.error_response err))

let dispatch ~action ~params =
  match action with
  | "get_node" -> get_node ~params
  | "list_children" -> list_children ~params
  | "list_sensors" -> list_sensors ~params
  | "get_sensor"   -> get_sensor   ~params
  | other -> err_bad_request (Printf.sprintf "unknown query action %S" other)
