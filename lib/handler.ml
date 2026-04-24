open Lambda_runtime_api_gateway

let v2_path req = req.Api_gateway.V2.raw_path
let v2_method req = req.Api_gateway.V2.request_context.http.method_

let strip_prefix p s =
  let lp = String.length p and ls = String.length s in
  if ls >= lp && String.sub s 0 lp = p
  then Some (String.sub s lp (ls - lp))
  else None

(* Decode application/x-www-form-urlencoded body into params list. *)
let url_decode s =
  let n = String.length s in
  let buf = Buffer.create n in
  let i = ref 0 in
  while !i < n do
    match s.[!i] with
    | '+' -> Buffer.add_char buf ' '; incr i
    | '%' when !i + 2 < n ->
        let h = String.sub s (!i + 1) 2 in
        (match int_of_string_opt ("0x" ^ h) with
         | Some code -> Buffer.add_char buf (Char.chr code); i := !i + 3
         | None -> Buffer.add_char buf '%'; incr i)
    | c -> Buffer.add_char buf c; incr i
  done;
  Buffer.contents buf

let parse_form body =
  String.split_on_char '&' body
  |> List.filter_map (fun pair ->
    match String.index_opt pair '=' with
    | None -> if pair = "" then None else Some (url_decode pair, "")
    | Some i ->
        let k = String.sub pair 0 i in
        let v = String.sub pair (i + 1) (String.length pair - i - 1) in
        Some (url_decode k, url_decode v))

(* Build JSON command body from flat form fields like "action=attach_sensor&data.daq_id=foo".
   The OCaml command dispatcher expects a flat JSON object (action + fields side by side),
   so strip any "data." prefix rather than nesting. *)
let form_to_command_json fields =
  let flat =
    List.filter_map (fun (k, v) ->
      if k = "" then None
      else
        let key =
          match strip_prefix "data." k with
          | Some name -> name
          | None -> k
        in
        Some (key, `String v)) fields
  in
  Yojson.Safe.to_string (`Assoc flat)

let content_type (req : Api_gateway.V2.request) =
  match List.assoc_opt "content-type" req.headers with
  | Some v -> v
  | None ->
      (match List.assoc_opt "Content-Type" req.headers with
       | Some v -> v | None -> "")

let lower s = String.lowercase_ascii s

let b64_decode s =
  match Base64.decode s with
  | Ok v -> v
  | Error _ -> s

let handle_command (req : Api_gateway.V2.request) =
  let raw0 = Option.value ~default:"" req.body in
  let raw = if req.is_base64_encoded then b64_decode raw0 else raw0 in
  let ct = lower (content_type req) in
  let is_form =
    let needle = "application/x-www-form-urlencoded" in
    String.length ct >= String.length needle
    && String.sub ct 0 (String.length needle) = needle
  in
  let body =
    if is_form then form_to_command_json (parse_form raw) else raw
  in
  Api_command.dispatch ~body

let handler _ctx body =
  match Yojson.Safe.from_string body with
  | exception Yojson.Json_error msg ->
      Error (Printf.sprintf "invalid event JSON: %s" msg)
  | json ->
      let req = Api_gateway.V2.request_of_json json in
      let path = v2_path req in
      let meth = v2_method req in
      let params = req.query_string_parameters in
      let response =
        match meth, path with
        (* JSON query routes — original + /hierarchy/ prefixed alias. *)
        | "GET", p ->
            (match strip_prefix "/query/" p with
             | Some action -> Api_query.dispatch ~action ~params
             | None ->
                 (match strip_prefix "/hierarchy/query/" p with
                  | Some action -> Api_html.dispatch ~action ~params
                  | None -> Api_json.error_response (Errors.Bad_request "no matching route")))
        | "POST", "/command"
        | "POST", "/hierarchy/command" -> handle_command req
        | _ -> Api_json.error_response (Errors.Bad_request "no matching route")
      in
      Ok response
