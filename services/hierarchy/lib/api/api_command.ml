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
  (* resample_minutes is optional. JSON callers send a number; the HTML form
     posts it as a string ("15"), and an empty number input posts "". Accept
     both and treat empty/absent as unset. Out-of-range values (<= 0) are
     rejected downstream in Sensors.attach. *)
  let resample_minutes =
    match field json "resample_minutes" with
    | Some (`Int i) -> Some i
    | Some (`String s) -> int_of_string_opt (String.trim s)
    | _ -> None
  in
  let* formula = Api_json.formula_of_json (field json "formula") in
  let* parent     = Node_id.of_string parent_s in
  let* meter_type = Sensor.meter_type_of_string mt_s in
  match Sensors.attach ~parent ~daq_id:daq ~purpose ~meter_type ?resample_minutes
          ~formula ?unit () with
  | Ok s -> Ok (Api_json.ok_response (Api_json.sensor_to_json s))
  | Error e -> Ok (Api_json.error_response e)

let run_replace_sensor_device json =
  let* id_s = require_string json "sensor_id" in
  let* daq  = require_string json "daq_id" in
  let* id   = Sensor_id.of_string id_s in
  match Sensors.replace_device ~sensor_id:id ~new_daq_id:daq () with
  | Ok s -> Ok (Api_json.ok_response (Api_json.sensor_to_json s))
  | Error e -> Ok (Api_json.error_response e)

let run_create_user json =
  let* email = require_string json "email" in
  let* name  = require_string json "name" in
  let* profile_s = require_string json "profile" in
  let* profile = Profile.of_string profile_s in
  let cognito_group = Profile.to_cognito_group profile in
  let* language =
    match field json "language" with
    | None -> Ok None
    | Some (`String s) ->
        (match Language.of_string s with
         | Ok l -> Ok (Some l)
         | Error e -> Error e)
    | Some _ -> Error "non-string field \"language\""
  in
  let* currency =
    match field json "currency" with
    | None -> Ok None
    | Some (`String s) ->
        (match Currency.of_string s with
         | Ok c -> Ok (Some c)
         | Error e -> Error e)
    | Some _ -> Error "non-string field \"currency\""
  in
  match
    Users.create ~email ~name ~cognito_group ?language ?currency ()
  with
  | Ok u -> Ok (Api_json.ok_response (Api_json.user_to_json u))
  | Error e -> Ok (Api_json.error_response e)

let run_update_user json =
  let* id_s = require_string json "id" in
  let* id = User_id.of_string id_s in
  let* name =
    match field json "name" with
    | None -> Ok None
    | Some (`String s) -> Ok (Some s)
    | Some _ -> Error "non-string field \"name\""
  in
  let* cognito_group =
    match field json "cognito_group" with
    | None -> Ok None
    | Some (`String s) ->
        (match Cognito_group.of_string s with
         | Ok g -> Ok (Some g)
         | Error e -> Error e)
    | Some _ -> Error "non-string field \"cognito_group\""
  in
  let* language =
    match field json "language" with
    | None -> Ok None
    | Some (`String s) ->
        (match Language.of_string s with
         | Ok l -> Ok (Some l)
         | Error e -> Error e)
    | Some _ -> Error "non-string field \"language\""
  in
  let* currency =
    match field json "currency" with
    | None -> Ok None
    | Some (`String s) ->
        (match Currency.of_string s with
         | Ok c -> Ok (Some c)
         | Error e -> Error e)
    | Some _ -> Error "non-string field \"currency\""
  in
  match
    Users.update ~id ?name ?cognito_group ?language ?currency ()
  with
  | Ok u -> Ok (Api_json.ok_response (Api_json.user_to_json u))
  | Error e -> Ok (Api_json.error_response e)

let run_delete_user json =
  let* id_s = require_string json "id" in
  let* id = User_id.of_string id_s in
  match Users.delete id with
  | Ok _ ->
      Ok (Api_json.ok_response
            (`Assoc [ ("deleted", `String (User_id.to_string id)) ]))
  | Error e -> Ok (Api_json.error_response e)

let run_block_user json =
  let* user_s = require_string json "user_id" in
  let* node_s = require_string json "node_id" in
  let* user_id = User_id.of_string user_s in
  let* node_id = Node_id.of_string node_s in
  match Access.block ~user_id ~node_id () with
  | Ok () -> Ok (Api_json.ok_response (`Assoc [ ("ok", `Bool true) ]))
  | Error e -> Ok (Api_json.error_response e)

let run_unblock_user json =
  let* user_s = require_string json "user_id" in
  let* node_s = require_string json "node_id" in
  let* user_id = User_id.of_string user_s in
  let* node_id = Node_id.of_string node_s in
  match Access.unblock ~user_id ~node_id () with
  | Ok () -> Ok (Api_json.ok_response (`Assoc [ ("ok", `Bool true) ]))
  | Error e -> Ok (Api_json.error_response e)

let run_grant_administrates json =
  let* user_s = require_string json "user_id" in
  let* node_s = require_string json "node_id" in
  let* user_id = User_id.of_string user_s in
  let* node_id = Node_id.of_string node_s in
  match Access.grant_administrates ~user_id ~node_id () with
  | Ok () -> Ok (Api_json.ok_response (`Assoc [ ("ok", `Bool true) ]))
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
       | Ok "create_user" ->
           (match run_create_user json with
            | Ok resp -> resp
            | Error m -> err_bad_request m)
       | Ok "update_user" ->
           (match run_update_user json with
            | Ok resp -> resp
            | Error m -> err_bad_request m)
       | Ok "delete_user" ->
           (match run_delete_user json with
            | Ok resp -> resp
            | Error m -> err_bad_request m)
       | Ok "block_user" ->
           (match run_block_user json with
            | Ok resp -> resp
            | Error m -> err_bad_request m)
       | Ok "unblock_user" ->
           (match run_unblock_user json with
            | Ok resp -> resp
            | Error m -> err_bad_request m)
       | Ok "grant_administrates" ->
           (match run_grant_administrates json with
            | Ok resp -> resp
            | Error m -> err_bad_request m)
       | Ok other ->
           err_bad_request (Printf.sprintf "unknown command action %S" other))
