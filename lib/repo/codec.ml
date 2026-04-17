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
  Ok {
    Node.id;
    name;
    parent;
    created;
    metadata;
    schema = None;
  }
