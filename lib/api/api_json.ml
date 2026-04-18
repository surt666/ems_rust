let node_to_json (n : Node.t) : Yojson.Safe.t =
  let parent =
    match n.parent with
    | None -> `Null
    | Some p -> `String (Node_id.to_string p)
  in
  let created = `String (Ptime.to_rfc3339 ~tz_offset_s:0 n.created) in
  `Assoc [
    ("id", `String (Node_id.to_string n.id));
    ("name", `String n.name);
    ("parent", parent);
    ("created", created);
    ("metadata", n.metadata);
  ]

let sensor_to_json (s : Sensor.t) : Yojson.Safe.t =
  let unit_json = match s.unit with Some u -> `String u | None -> `Null in
  `Assoc [
    ("id",             `String (Sensor_id.to_string s.id));
    ("active_from",    `String (Ptime.to_rfc3339 ~tz_offset_s:0 s.active_from));
    ("parent",         `String (Node_id.to_string s.parent));
    ("daq_address",    `String s.daq_address);
    ("hierarchy_path", `String s.hierarchy_path);
    ("purpose",        `String s.purpose);
    ("meter_type",     `String (Sensor.meter_type_to_string s.meter_type));
    ("unit",           unit_json);
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
