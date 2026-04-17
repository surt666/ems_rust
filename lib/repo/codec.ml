module Dyn = Smaws_Client_DynamoDB

let s (x : string) : Dyn.attribute_value = Dyn.S x
let n (x : string) : Dyn.attribute_value = Dyn.N x
let b (x : bool) : Dyn.attribute_value = Dyn.BOOL x

let rec json_to_attr (j : Yojson.Safe.t) : Dyn.attribute_value =
  match j with
  | `Null -> Dyn.NULL true
  | `Bool v -> Dyn.BOOL v
  | `Int i -> Dyn.N (string_of_int i)
  | `Intlit l -> Dyn.N l
  | `Float f -> Dyn.N (Printf.sprintf "%.17g" f)
  | `String v -> Dyn.S v
  | `List xs -> Dyn.L (List.map json_to_attr xs)
  | `Assoc kvs -> Dyn.M (List.map (fun (k, v) -> (k, json_to_attr v)) kvs)

let rec attr_to_json (a : Dyn.attribute_value) : Yojson.Safe.t =
  match a with
  | Dyn.NULL _ -> `Null
  | Dyn.BOOL v -> `Bool v
  | Dyn.S v -> `String v
  | Dyn.N v ->
      (match int_of_string_opt v with
       | Some i -> `Int i
       | None -> `Float (float_of_string v))
  | Dyn.L xs -> `List (List.map attr_to_json xs)
  | Dyn.M kvs -> `Assoc (List.map (fun (k, v) -> (k, attr_to_json v)) kvs)
  | Dyn.B _ | Dyn.BS _ | Dyn.NS _ | Dyn.SS _ -> `Null

let schema_to_attr (sch : Schema.t) : Dyn.attribute_value =
  let edge_spec_m (spec : Schema.edge_spec) =
    let base = [ ("label", s spec.label) ] in
    let with_min =
      match spec.min with Some m -> ("min", n (string_of_int m)) :: base | None -> base
    in
    let with_max =
      match spec.max with Some m -> ("max", n (string_of_int m)) :: with_min | None -> with_min
    in
    Dyn.M with_max
  in
  let edges_m =
    List.map
      (fun (lvl, cs) ->
        let inner =
          List.map (fun (child, spec) -> (Level.to_string child, edge_spec_m spec)) cs
        in
        (Level.to_string lvl, Dyn.M inner))
      sch.edges
  in
  let field_type_attr = function
    | Metadata.String { min_len; max_len } ->
        let base = [ ("type", s "string") ] in
        let base = match min_len with Some v -> ("min_len", n (string_of_int v)) :: base | _ -> base in
        let base = match max_len with Some v -> ("max_len", n (string_of_int v)) :: base | _ -> base in
        Dyn.M base
    | Metadata.Number { min; max } ->
        let base = [ ("type", s "number") ] in
        let base = match min with Some v -> ("min", n (Printf.sprintf "%.17g" v)) :: base | _ -> base in
        let base = match max with Some v -> ("max", n (Printf.sprintf "%.17g" v)) :: base | _ -> base in
        Dyn.M base
    | Metadata.Integer { min; max } ->
        let base = [ ("type", s "integer") ] in
        let base = match min with Some v -> ("min", n (Int64.to_string v)) :: base | _ -> base in
        let base = match max with Some v -> ("max", n (Int64.to_string v)) :: base | _ -> base in
        Dyn.M base
    | Metadata.Boolean -> Dyn.M [ ("type", s "boolean") ]
    | Metadata.Timestamp -> Dyn.M [ ("type", s "timestamp") ]
    | Metadata.Enum { one_of } ->
        Dyn.M [ ("type", s "enum"); ("one_of", Dyn.L (List.map s one_of)) ]
  in
  let spec_m (fs : Metadata.field_spec) =
    match field_type_attr fs.typ with
    | Dyn.M kvs -> Dyn.M (("required", b fs.required) :: kvs)
    | other -> other
  in
  let metadata_m =
    List.map
      (fun (lvl, fields) ->
        let inner = List.map (fun (name, fs) -> (name, spec_m fs)) fields in
        (Level.to_string lvl, Dyn.M inner))
      sch.metadata
  in
  Dyn.M [
    ("version", n (string_of_int sch.version));
    ("edges", Dyn.M edges_m);
    ("metadata", Dyn.M metadata_m);
  ]

let node_to_item (nd : Node.t) : (string * Dyn.attribute_value) list =
  let id = Node_id.to_string nd.Node.id in
  let base =
    [
      ("pk", s id);
      ("sk", s id);
      ("type", s "node");
      ("name", s nd.Node.name);
      ("created", s (Ptime.to_rfc3339 ~tz_offset_s:0 nd.Node.created));
      ("metadata", json_to_attr nd.Node.metadata);
    ]
  in
  let with_parent =
    match nd.Node.parent with
    | Some p -> ("parent", s (Node_id.to_string p)) :: base
    | None -> base
  in
  match nd.Node.schema with
  | Some sch -> ("schema", schema_to_attr sch) :: with_parent
  | None -> with_parent

let edge_item ~from_ ~to_ ~label ~created =
  let from_s = Node_id.to_string from_ in
  let to_s = Node_id.to_string to_ in
  let sk = Printf.sprintf "has_%s#%s" label to_s in
  [
    ("pk", s from_s);
    ("sk", s sk);
    ("type", s "edge");
    ("label", s label);
    ("created", s (Ptime.to_rfc3339 ~tz_offset_s:0 created));
    ("gsi1pk", s to_s);
    ("gsi1sk", s from_s);
  ]

let ( let* ) = Result.bind

let field kvs k =
  match List.assoc_opt k kvs with
  | Some v -> Ok v
  | None -> Error (Printf.sprintf "missing field %S" k)

let as_string (v : Dyn.attribute_value) =
  match v with
  | Dyn.S s -> Ok s
  | _ -> Error "expected S"

let as_map (v : Dyn.attribute_value) =
  match v with
  | Dyn.M kvs -> Ok kvs
  | _ -> Error "expected M"

let as_list (v : Dyn.attribute_value) =
  match v with
  | Dyn.L xs -> Ok xs
  | _ -> Error "expected L"

let opt_int_of_n : Dyn.attribute_value option -> int option = function
  | Some (Dyn.N s) -> int_of_string_opt s
  | _ -> None

let opt_float_of_n : Dyn.attribute_value option -> float option = function
  | Some (Dyn.N s) -> (try Some (float_of_string s) with _ -> None)
  | _ -> None

let opt_int64_of_n : Dyn.attribute_value option -> int64 option = function
  | Some (Dyn.N s) -> (try Some (Int64.of_string s) with _ -> None)
  | _ -> None

let decode_field_spec (v : Dyn.attribute_value) : (Metadata.field_spec, string) result =
  let* kvs = as_map v in
  let* type_v = field kvs "type" in
  let* typ_s = as_string type_v in
  let required =
    match List.assoc_opt "required" kvs with
    | Some (Dyn.BOOL b) -> b
    | _ -> false
  in
  let* typ =
    match typ_s with
    | "string" ->
        Ok (Metadata.String {
          min_len = opt_int_of_n (List.assoc_opt "min_len" kvs);
          max_len = opt_int_of_n (List.assoc_opt "max_len" kvs);
        })
    | "number" ->
        Ok (Metadata.Number {
          min = opt_float_of_n (List.assoc_opt "min" kvs);
          max = opt_float_of_n (List.assoc_opt "max" kvs);
        })
    | "integer" ->
        Ok (Metadata.Integer {
          min = opt_int64_of_n (List.assoc_opt "min" kvs);
          max = opt_int64_of_n (List.assoc_opt "max" kvs);
        })
    | "boolean" -> Ok Metadata.Boolean
    | "timestamp" -> Ok Metadata.Timestamp
    | "enum" ->
        let* one_of_v = field kvs "one_of" in
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

let decode_edge_spec (v : Dyn.attribute_value) : (Schema.edge_spec, string) result =
  let* kvs = as_map v in
  let* label_v = field kvs "label" in
  let* label = as_string label_v in
  Ok Schema.{
    label;
    min = opt_int_of_n (List.assoc_opt "min" kvs);
    max = opt_int_of_n (List.assoc_opt "max" kvs);
  }

let decode_level_keyed_map ~decode_inner kvs =
  List.fold_left
    (fun acc (lvl_s, inner_v) ->
      let* acc = acc in
      match Level.of_string lvl_s with
      | Error _ -> Ok acc
      | Ok lvl ->
          let* inner_kvs = as_map inner_v in
          let* inner = decode_inner inner_kvs in
          Ok ((lvl, inner) :: acc))
    (Ok []) kvs
  |> Result.map List.rev

let decode_schema (v : Dyn.attribute_value) : (Schema.t, string) result =
  let* kvs = as_map v in
  let* ver_v = field kvs "version" in
  let* version =
    match ver_v with
    | Dyn.N s ->
        (match int_of_string_opt s with
         | Some i -> Ok i
         | None -> Error "bad version N")
    | _ -> Error "expected N for version"
  in
  let* edges_v = field kvs "edges" in
  let* edges_kvs = as_map edges_v in
  let* edges =
    decode_level_keyed_map edges_kvs
      ~decode_inner:(fun inner_kvs ->
        List.fold_left
          (fun acc (child_s, spec_v) ->
            let* acc = acc in
            match Level.of_string child_s with
            | Error _ -> Ok acc
            | Ok child ->
                let* spec = decode_edge_spec spec_v in
                Ok ((child, spec) :: acc))
          (Ok []) inner_kvs
        |> Result.map List.rev)
  in
  let* metadata =
    match List.assoc_opt "metadata" kvs with
    | None -> Ok []
    | Some m_v ->
        let* m_kvs = as_map m_v in
        decode_level_keyed_map m_kvs
          ~decode_inner:(fun inner_kvs ->
            List.fold_left
              (fun acc (fname, spec_v) ->
                let* acc = acc in
                let* spec = decode_field_spec spec_v in
                Ok ((fname, spec) :: acc))
              (Ok []) inner_kvs
            |> Result.map List.rev)
  in
  Ok Schema.{ version; edges; metadata }

let node_of_item kvs =
  let* pk = field kvs "pk" in
  let* pk = as_string pk in
  let* id = Node_id.of_string pk in
  let* name = field kvs "name" in
  let* name = as_string name in
  let* created_s = field kvs "created" in
  let* created_s = as_string created_s in
  let created =
    match Ptime.of_rfc3339 created_s with
    | Ok (t, _, _) -> t
    | Error _ -> Ptime.epoch
  in
  let parent =
    match List.assoc_opt "parent" kvs with
    | Some (Dyn.S v) ->
        (match Node_id.of_string v with Ok p -> Some p | Error _ -> None)
    | _ -> None
  in
  let metadata =
    match List.assoc_opt "metadata" kvs with
    | Some v -> attr_to_json v
    | None -> `Assoc []
  in
  let schema =
    match List.assoc_opt "schema" kvs with
    | None -> None
    | Some v ->
        (match decode_schema v with
         | Ok s -> Some s
         | Error _ -> None)
  in
  Ok {
    Node.id;
    name;
    parent;
    created;
    metadata;
    schema;
  }
