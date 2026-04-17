open Lambda_runtime_api_gateway

let v2_path req = req.Api_gateway.V2.raw_path
let v2_method req = req.Api_gateway.V2.request_context.http.method_

let prefix p s =
  String.length s >= String.length p
  && String.sub s 0 (String.length p) = p

let handler _ctx body =
  match Yojson.Safe.from_string body with
  | exception Yojson.Json_error msg ->
      Error (Printf.sprintf "invalid event JSON: %s" msg)
  | json ->
      let req = Api_gateway.V2.request_of_json json in
      let path = v2_path req in
      let meth = v2_method req in
      let response =
        match meth, path with
        | "GET", p when prefix "/query/" p ->
            let action =
              String.sub p (String.length "/query/") (String.length p - String.length "/query/")
            in
            Api_query.dispatch ~action ~params:req.query_string_parameters
        | "POST", "/command" ->
            let inner =
              match req.body with
              | Some s -> s
              | None -> ""
            in
            Api_command.dispatch ~body:inner
        | _ ->
            Api_json.error_response (Errors.Bad_request "no matching route")
      in
      Ok response
