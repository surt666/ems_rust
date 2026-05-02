let ( let* ) = Result.bind

let schema_to_json (sch : Schema.t) : Yojson.Safe.t =
  let edge_spec_body (spec : Schema.edge_spec) =
    let kvs =
      (match spec.min with Some m -> [ ("min", `Int m) ] | None -> [])
      @ (match spec.max with Some m -> [ ("max", `Int m) ] | None -> [])
    in
    `Assoc kvs
  in
  let edges_j =
    `Assoc (List.map
      (fun (lvl, cs) ->
        let inner =
          List.map
            (fun (child, specs) ->
              let label_m =
                List.map (fun (sp : Schema.edge_spec) -> (sp.label, edge_spec_body sp)) specs
              in
              (Level.to_string child, `Assoc label_m))
            cs
        in
        (Level.to_string lvl, `Assoc inner))
      sch.edges)
  in
  let field_type_json = function
    | Metadata.String { min_len; max_len } ->
        let base = [ ("type", `String "string") ] in
        let base = match min_len with Some v -> ("min_len", `Int v) :: base | _ -> base in
        let base = match max_len with Some v -> ("max_len", `Int v) :: base | _ -> base in
        base
    | Metadata.Number { min; max } ->
        let base = [ ("type", `String "number") ] in
        let base = match min with Some v -> ("min", `Float v) :: base | _ -> base in
        let base = match max with Some v -> ("max", `Float v) :: base | _ -> base in
        base
    | Metadata.Integer { min; max } ->
        let base = [ ("type", `String "integer") ] in
        let base = match min with Some v -> ("min", `Intlit (Int64.to_string v)) :: base | _ -> base in
        let base = match max with Some v -> ("max", `Intlit (Int64.to_string v)) :: base | _ -> base in
        base
    | Metadata.Boolean -> [ ("type", `String "boolean") ]
    | Metadata.Timestamp -> [ ("type", `String "timestamp") ]
    | Metadata.Enum { one_of } ->
        [ ("type", `String "enum"); ("one_of", `List (List.map (fun s -> `String s) one_of)) ]
  in
  let spec_j (fs : Metadata.field_spec) =
    `Assoc (("required", `Bool fs.required) :: field_type_json fs.typ)
  in
  let metadata_j =
    `Assoc (List.map
      (fun (lvl, fields) ->
        let inner = List.map (fun (name, fs) -> (name, spec_j fs)) fields in
        (Level.to_string lvl, `Assoc inner))
      sch.metadata)
  in
  let sensors_j =
    `List (List.map (fun lvl -> `String (Level.to_string lvl)) sch.sensors)
  in
  `Assoc [
    ("version", `Int sch.version);
    ("edges", edges_j);
    ("metadata", metadata_j);
    ("sensors", sensors_j);
  ]

let opt_int_of_json = function
  | `Int i -> Some i
  | `Intlit s -> int_of_string_opt s
  | _ -> None

let opt_float_of_json = function
  | `Float f -> Some f
  | `Int i -> Some (float_of_int i)
  | `Intlit s -> float_of_string_opt s
  | _ -> None

let opt_int64_of_json = function
  | `Int i -> Some (Int64.of_int i)
  | `Intlit s -> (try Some (Int64.of_string s) with _ -> None)
  | _ -> None

let as_assoc = function
  | `Assoc kvs -> Ok kvs
  | _ -> Error "expected object"

let as_list = function
  | `List xs -> Ok xs
  | _ -> Error "expected array"

let as_string = function
  | `String s -> Ok s
  | _ -> Error "expected string"

let decode_field_spec (v : Yojson.Safe.t) : (Metadata.field_spec, string) result =
  let* kvs = as_assoc v in
  let* typ_s =
    match List.assoc_opt "type" kvs with
    | Some (`String s) -> Ok s
    | _ -> Error "missing or non-string field \"type\""
  in
  let required =
    match List.assoc_opt "required" kvs with
    | Some (`Bool b) -> b
    | _ -> false
  in
  let get k = List.assoc_opt k kvs in
  let* typ =
    match typ_s with
    | "string" ->
        Ok (Metadata.String {
          min_len = Option.bind (get "min_len") opt_int_of_json;
          max_len = Option.bind (get "max_len") opt_int_of_json;
        })
    | "number" ->
        Ok (Metadata.Number {
          min = Option.bind (get "min") opt_float_of_json;
          max = Option.bind (get "max") opt_float_of_json;
        })
    | "integer" ->
        Ok (Metadata.Integer {
          min = Option.bind (get "min") opt_int64_of_json;
          max = Option.bind (get "max") opt_int64_of_json;
        })
    | "boolean" -> Ok Metadata.Boolean
    | "timestamp" -> Ok Metadata.Timestamp
    | "enum" ->
        let* one_of_v =
          match get "one_of" with
          | Some v -> Ok v
          | None -> Error "enum requires \"one_of\""
        in
        let* xs = as_list one_of_v in
        let* vals =
          List.fold_left
            (fun acc v ->
              let* acc = acc in
              let* s = as_string v in
              Ok (s :: acc))
            (Ok []) xs
        in
        Ok (Metadata.Enum { one_of = List.rev vals })
    | other -> Error (Printf.sprintf "unknown field type %S" other)
  in
  Ok Metadata.{ typ; required }

let schema_of_json (v : Yojson.Safe.t) : (Schema.t, string) result =
  let* kvs = as_assoc v in
  let* version =
    match List.assoc_opt "version" kvs with
    | Some (`Int i) -> Ok i
    | Some (`Intlit s) ->
        (match int_of_string_opt s with
         | Some i -> Ok i
         | None -> Error "bad version")
    | _ -> Error "missing or non-integer field \"version\""
  in
  let* edges_v =
    match List.assoc_opt "edges" kvs with
    | Some v -> Ok v
    | None -> Error "missing field \"edges\""
  in
  let* edges_kvs = as_assoc edges_v in
  let* edges =
    List.fold_left
      (fun acc (parent_s, inner_v) ->
        let* acc = acc in
        let* parent = Level.of_string parent_s in
        let* inner_kvs = as_assoc inner_v in
        let* children =
          List.fold_left
            (fun acc (child_s, labels_v) ->
              let* acc = acc in
              let* child = Level.of_string child_s in
              let* labels_kvs = as_assoc labels_v in
              let* specs =
                List.fold_left
                  (fun acc (label, body) ->
                    let* acc = acc in
                    let* body_kvs = as_assoc body in
                    let get k = List.assoc_opt k body_kvs in
                    let spec =
                      Schema.{
                        label;
                        min = Option.bind (get "min") opt_int_of_json;
                        max = Option.bind (get "max") opt_int_of_json;
                      }
                    in
                    Ok (spec :: acc))
                  (Ok []) labels_kvs
              in
              Ok ((child, List.rev specs) :: acc))
            (Ok []) inner_kvs
        in
        Ok ((parent, List.rev children) :: acc))
      (Ok []) edges_kvs
    |> Result.map List.rev
  in
  let* metadata =
    match List.assoc_opt "metadata" kvs with
    | None -> Ok []
    | Some m_v ->
        let* m_kvs = as_assoc m_v in
        List.fold_left
          (fun acc (lvl_s, inner_v) ->
            let* acc = acc in
            let* lvl = Level.of_string lvl_s in
            let* inner_kvs = as_assoc inner_v in
            let* fields =
              List.fold_left
                (fun acc (fname, spec_v) ->
                  let* acc = acc in
                  let* spec = decode_field_spec spec_v in
                  Ok ((fname, spec) :: acc))
                (Ok []) inner_kvs
            in
            Ok ((lvl, List.rev fields) :: acc))
          (Ok []) m_kvs
        |> Result.map List.rev
  in
  let* sensors =
    match List.assoc_opt "sensors" kvs with
    | None -> Ok []
    | Some v ->
        let* xs = as_list v in
        List.fold_left
          (fun acc item ->
            let* acc = acc in
            let* s = as_string item in
            let* lvl = Level.of_string s in
            Ok (lvl :: acc))
          (Ok []) xs
        |> Result.map List.rev
  in
  Ok Schema.{ version; edges; metadata; sensors }

let node_ref_to_json (id, name) : Yojson.Safe.t =
  `Assoc [
    ("id",   `String (Node_id.to_string id));
    ("name", `String name);
  ]

let node_to_json (n : Node.t) : Yojson.Safe.t =
  let parent =
    match n.parent with
    | None -> `Null
    | Some p -> `String (Node_id.to_string p)
  in
  let created = `String (Ptime.to_rfc3339 ~tz_offset_s:0 n.created) in
  let base =
    [
      ("id", `String (Node_id.to_string n.id));
      ("name", `String n.name);
      ("parent", parent);
      ("created", created);
      ("metadata", n.metadata);
    ]
  in
  let with_schema =
    match n.schema with
    | Some s -> base @ [ ("schema", schema_to_json s) ]
    | None -> base
  in
  `Assoc with_schema

let sensor_to_json (s : Sensor.t) : Yojson.Safe.t =
  let unit_json = match s.unit with Some u -> `String u | None -> `Null in
  `Assoc [
    ("id",         `String (Sensor_id.to_string s.id));
    ("created",    `String (Ptime.to_rfc3339 ~tz_offset_s:0 s.created));
    ("daq_id",     `String s.daq_id);
    ("path",       `String s.path);
    ("purpose",    `String s.purpose);
    ("meter_type", `String (Sensor.meter_type_to_string s.meter_type));
    ("unit",       unit_json);
  ]

let user_to_json (u : User.t) : Yojson.Safe.t =
  `Assoc [
    ("id",            `String (User_id.to_string u.User.id));
    ("email",         `String (User_id.email u.User.id));
    ("name",          `String u.User.name);
    ("cognito_group", `String (Cognito_group.to_string u.User.cognito_group));
    ("language",      `String (Language.to_string u.User.language));
    ("currency",      `String (Currency.to_string u.User.currency));
    ("created",       `String (Ptime.to_rfc3339 ~tz_offset_s:0 u.User.created));
  ]

let error_body (err : Errors.t) : string =
  let j =
    `Assoc [
      ("error", `Assoc (
        [
          ("code", `String (Errors.to_code err));
          ("message", `String (Errors.message err));
        ]
        @ (match Errors.details err with
           | Some d -> [ ("details", d) ]
           | None -> [])
      ));
    ]
  in
  Yojson.Safe.to_string j

let v2_response ?(headers = [ ("content-type", "application/json") ]) ~status body =
  let open Lambda_runtime_api_gateway in
  let response = Api_gateway.V2.make_response ~status_code:status ~headers body in
  Yojson.Safe.to_string (Api_gateway.V2.response_to_json response)

let ok_response j      = v2_response ~status:200 (Yojson.Safe.to_string j)
let error_response err = v2_response ~status:(Errors.http_status err) (error_body err)
