open Lambda_runtime_api_gateway

let handler _ctx body =
  match Yojson.Safe.from_string body with
  | json ->
      let request = Api_gateway.V2.request_of_json json in
      let name =
        match List.assoc_opt "name" request.query_string_parameters with
        | Some n -> n
        | None -> "World"
      in
      let response =
        Api_gateway.V2.make_response ~status_code:200
          ~headers:[ ("content-type", "application/json") ]
          (Yojson.Safe.to_string
             (`Assoc [ ("message", `String (Printf.sprintf "Hello, %s!" name)) ]))
      in
      Ok (Yojson.Safe.to_string (Api_gateway.V2.response_to_json response))
  | exception Yojson.Json_error msg -> Error (Printf.sprintf "Invalid JSON: %s" msg)
