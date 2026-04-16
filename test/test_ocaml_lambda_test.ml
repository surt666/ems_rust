let sample_v2_event ?name () =
  let qs : (string * Yojson.Safe.t) list =
    match name with
    | Some n -> [ ("queryStringParameters", `Assoc [ ("name", `String n) ]) ]
    | None -> []
  in
  let event : Yojson.Safe.t =
    `Assoc
      ([
         ("version", `String "2.0");
         ("routeKey", `String "GET /hello");
         ("rawPath", `String "/hello");
         ("rawQueryString", `String "");
         ("headers", `Assoc [ ("accept", `String "*/*") ]);
         ( "requestContext",
           `Assoc
             [
               ("accountId", `String "123456789012");
               ("apiId", `String "api-id");
               ( "http",
                 `Assoc
                   [
                     ("method", `String "GET");
                     ("path", `String "/hello");
                     ("protocol", `String "HTTP/1.1");
                     ("sourceIp", `String "127.0.0.1");
                     ("userAgent", `String "test");
                   ] );
               ("requestId", `String "req-1");
               ("stage", `String "$default");
               ("timeEpoch", `Int 1700000000000);
             ] );
         ("isBase64Encoded", `Bool false);
       ]
      @ qs)
  in
  Yojson.Safe.to_string event

let ctx : Lambda_runtime.Context.t =
  {
    memory_limit_in_mb = 128;
    function_name = "test";
    function_version = "$LATEST";
    log_group = "test";
    log_stream = "test";
    aws_request_id = "req-1";
    invoked_function_arn = "arn:aws:lambda:us-east-1:123:function:test";
    xray_trace_id = None;
    client_context = None;
    cognito_identity = None;
    deadline_ms = 0L;
  }

let invoke event =
  match Ocaml_lambda_test.Handler.handler ctx event with
  | Ok body -> body
  | Error e -> Alcotest.failf "handler returned Error: %s" e

let response_field body field =
  Yojson.Safe.from_string body |> Yojson.Safe.Util.member field

let message_of body =
  response_field body "body"
  |> Yojson.Safe.Util.to_string
  |> Yojson.Safe.from_string
  |> Yojson.Safe.Util.member "message"
  |> Yojson.Safe.Util.to_string

let test_default_greeting () =
  let body = invoke (sample_v2_event ()) in
  Alcotest.(check int)
    "status code" 200
    (response_field body "statusCode" |> Yojson.Safe.Util.to_int);
  Alcotest.(check string) "default message" "Hello, World!" (message_of body)

let test_with_name () =
  let body = invoke (sample_v2_event ~name:"Steen" ()) in
  Alcotest.(check string) "custom message" "Hello, Steen!" (message_of body)

let test_invalid_json () =
  match Ocaml_lambda_test.Handler.handler ctx "not json" with
  | Ok _ -> Alcotest.fail "expected Error for invalid JSON"
  | Error msg ->
      Alcotest.(check bool)
        "error mentions invalid JSON" true
        (String.length msg > 0)

let name_generator =
  let open Base_quickcheck.Generator in
  (* printable, non-empty strings — keeps assertions readable on failure *)
  string_non_empty_of char_print

let test_name_roundtrip_prop () =
  let open Base in
  Base_quickcheck.Test.run_exn
    (module struct
      type t = string [@@deriving sexp_of]

      let quickcheck_generator = name_generator
      let quickcheck_shrinker = Base_quickcheck.Shrinker.string
    end)
    ~f:(fun name ->
      let body = invoke (sample_v2_event ~name ()) in
      let got = message_of body in
      let expected = Printf.sprintf "Hello, %s!" name in
      if not (String.equal got expected) then
        Alcotest.failf "name=%S expected=%S got=%S" name expected got)

let () =
  Alcotest.run "handler"
    [
      ( "v2 events",
        [
          Alcotest.test_case "default name" `Quick test_default_greeting;
          Alcotest.test_case "custom name" `Quick test_with_name;
          Alcotest.test_case "invalid JSON" `Quick test_invalid_json;
        ] );
      ( "properties",
        [
          Alcotest.test_case "name roundtrips into greeting" `Quick
            test_name_roundtrip_prop;
        ] );
    ]
