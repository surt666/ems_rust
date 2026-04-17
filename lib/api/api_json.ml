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
