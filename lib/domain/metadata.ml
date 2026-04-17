type field_type =
  | String of { min_len : int option; max_len : int option }
  | Number of { min : float option; max : float option }
  | Integer of { min : int64 option; max : int64 option }
  | Boolean
  | Timestamp
  | Enum of { one_of : string list }

type field_spec = { typ : field_type; required : bool }

type error = { path : string; message : string }

let validate_spec spec =
  match spec.typ with
  | Enum { one_of = [] } -> Error "enum must declare non-empty one_of"
  | _ -> Ok ()

let is_rfc3339 s =
  match Ptime.of_rfc3339 s with Ok _ -> true | Error _ -> false

let validate_one ~path _spec (v : Yojson.Safe.t) =
  match _spec.typ, v with
  | _, `Null when _spec.required ->
      Error { path; message = "required field missing" }
  | _, `Null ->
      Ok ()
  | String { min_len; max_len }, `String s ->
      let len = String.length s in
      (match min_len with
       | Some n when len < n ->
           Error { path; message = Printf.sprintf "string too short (min %d)" n }
       | _ ->
           match max_len with
           | Some n when len > n ->
               Error { path; message = Printf.sprintf "string too long (max %d)" n }
           | _ -> Ok ())
  | Number { min; max }, `Float f ->
      (match min with
       | Some lo when f < lo ->
           Error { path; message = Printf.sprintf "value below minimum %g" lo }
       | _ ->
           match max with
           | Some hi when f > hi ->
               Error { path; message = Printf.sprintf "value above maximum %g" hi }
           | _ -> Ok ())
  | Number { min; max }, `Int i ->
      let f = Float.of_int i in
      (match min with
       | Some lo when f < lo ->
           Error { path; message = Printf.sprintf "value below minimum %g" lo }
       | _ ->
           match max with
           | Some hi when f > hi ->
               Error { path; message = Printf.sprintf "value above maximum %g" hi }
           | _ -> Ok ())
  | Integer { min; max }, `Int i ->
      let iv = Int64.of_int i in
      (match min with
       | Some lo when Int64.compare iv lo < 0 ->
           Error { path; message = "integer below minimum" }
       | _ ->
           match max with
           | Some hi when Int64.compare iv hi > 0 ->
               Error { path; message = "integer above maximum" }
           | _ -> Ok ())
  | Boolean, `Bool _ -> Ok ()
  | Timestamp, `String s ->
      if is_rfc3339 s then Ok ()
      else Error { path; message = "invalid RFC3339 timestamp" }
  | Enum { one_of }, `String s ->
      if List.mem s one_of then Ok ()
      else Error { path; message = Printf.sprintf "value not in enum: %s" s }
  | _ -> Error { path; message = "type mismatch" }

let validate ~specs (v : Yojson.Safe.t) =
  match v with
  | `Assoc kvs ->
      let errs = ref [] in
      List.iter
        (fun (name, spec) ->
          let present = List.assoc_opt name kvs in
          match present with
          | None when spec.required ->
              errs := { path = name; message = "required field missing" } :: !errs
          | None -> ()
          | Some value ->
              (match validate_one ~path:name spec value with
               | Ok () -> ()
               | Error e -> errs := e :: !errs))
        specs;
      if !errs = [] then Ok () else Error (List.rev !errs)
  | _ -> Error [ { path = ""; message = "metadata must be a JSON object" } ]
