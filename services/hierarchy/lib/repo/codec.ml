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
  let edge_spec_body (spec : Schema.edge_spec) =
    let base = [] in
    let base = match spec.min with Some m -> ("min", n (string_of_int m)) :: base | None -> base in
    let base = match spec.max with Some m -> ("max", n (string_of_int m)) :: base | None -> base in
    Dyn.M base
  in
  let edges_m =
    List.map
      (fun (lvl, cs) ->
        let inner =
          List.map
            (fun (child, specs) ->
              let label_m =
                List.map (fun (sp : Schema.edge_spec) -> (sp.label, edge_spec_body sp)) specs
              in
              (Level.to_string child, Dyn.M label_m))
            cs
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
  let sensors_l =
    Dyn.L (List.map (fun lvl -> s (Level.to_string lvl)) sch.sensors)
  in
  Dyn.M [
    ("version", n (string_of_int sch.version));
    ("edges", Dyn.M edges_m);
    ("metadata", Dyn.M metadata_m);
    ("sensors", sensors_l);
  ]

(* gsi1pk for an item at level [lvl] is just "HN<n>" — a per-level
   partition key. With this, "list every HN<n> in the system" is one
   GSI Query, and "delete this subtree" is one Query per level under
   the subtree root. Sensor items use "S". *)
let node_gsi1pk (lvl : Level.t) = Printf.sprintf "HN%d" (Level.depth lvl)
let sensor_gsi1pk = "S"

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
      ("gsi1pk", s (node_gsi1pk (Node.level nd)));
      ("gsi1sk", s nd.Node.path);
    ]
  in
  match nd.Node.schema with
  | Some sch -> ("schema", schema_to_attr sch) :: base
  | None -> base

(* Generic edge codec used for user→node edges (Blocked, Administrates).
   gsi1pk/gsi1sk encode the reverse direction so user→node lookups can go
   either way via the index. HN-side edges go through edge_with_anchor. *)
let edge_item ~from_ ~to_ ~kind ~name ~created =
  let base =
    [
      ("pk", s from_);
      ("sk", s (Printf.sprintf "%s#%s" (Edge_kind.sk_verb kind) to_));
      ("type", s "edge");
      ("kind", s (Edge_kind.to_string kind));
      ("name", s name);
      ("created", s (Ptime.to_rfc3339 ~tz_offset_s:0 created));
    ]
  in
  match Edge_kind.gsi_verb kind with
  | Some verb ->
      ("gsi1pk", s to_)
      :: ("gsi1sk", s (Printf.sprintf "%s#%s" verb from_))
      :: base
  | None -> base

(* HN-side edges (Has_label and Has_sensor) live under the parent's pk
   and are indexed by the child's level partition + the child's path.
   `gsi1pk` is taken from the child's id: `HN<n>` for Has_label, `S` for
   Has_sensor. `gsi1sk` is the child / sensor path so subtree sweeps
   work via begins_with. *)
let edge_with_anchor ~from_ ~to_ ~kind ~name ~created ~self_path =
  let gsi1pk_v =
    match kind with
    | Edge_kind.Has_sensor -> sensor_gsi1pk
    | Edge_kind.Has_label _ ->
        (match Node_id.of_string to_ with
         | Ok id -> node_gsi1pk (Node_id.level id)
         | Error _ ->
             failwith (Printf.sprintf "edge_with_anchor: bad to_ %S" to_))
    | _ -> failwith "edge_with_anchor: only HN-side edges"
  in
  [
    ("pk", s from_);
    ("sk", s (Printf.sprintf "%s#%s" (Edge_kind.sk_verb kind) to_));
    ("type", s "edge");
    ("kind", s (Edge_kind.to_string kind));
    ("name", s name);
    ("created", s (Ptime.to_rfc3339 ~tz_offset_s:0 created));
    ("gsi1pk", s gsi1pk_v);
    ("gsi1sk", s self_path);
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

let decode_edge_spec ~label (body : Dyn.attribute_value) : (Schema.edge_spec, string) result =
  let* kvs = as_map body in
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
          (fun acc (child_s, labels_v) ->
            let* acc = acc in
            match Level.of_string child_s with
            | Error _ -> Ok acc
            | Ok child ->
                let* label_kvs = as_map labels_v in
                let* specs =
                  List.fold_left
                    (fun acc (label, body) ->
                      let* acc = acc in
                      let* sp = decode_edge_spec ~label body in
                      Ok (sp :: acc))
                    (Ok []) label_kvs
                  |> Result.map List.rev
                in
                Ok ((child, specs) :: acc))
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
  let* sensors =
    match List.assoc_opt "sensors" kvs with
    | None -> Ok []
    | Some v ->
        let* xs = as_list v in
        List.fold_left
          (fun acc item ->
            let* acc = acc in
            let* s_str = as_string item in
            let* lvl = Level.of_string s_str in
            Ok (lvl :: acc))
          (Ok []) xs
        |> Result.map List.rev
  in
  Ok Schema.{ version; edges; metadata; sensors }

let rec expr_to_attr : Formula.expr -> Dyn.attribute_value = function
  | Formula.Num f  -> Dyn.M [ ("t", s "num"); ("v", n (Printf.sprintf "%.17g" f)) ]
  | Formula.Self   -> Dyn.M [ ("t", s "self") ]
  | Formula.Ref a  -> Dyn.M [ ("t", s "ref"); ("a", s a) ]
  | Formula.Abs e  -> Dyn.M [ ("t", s "abs"); ("e", expr_to_attr e) ]
  | Formula.Add (a, b) ->
      Dyn.M [ ("t", s "add"); ("l", expr_to_attr a); ("r", expr_to_attr b) ]
  | Formula.Sub (a, b) ->
      Dyn.M [ ("t", s "sub"); ("l", expr_to_attr a); ("r", expr_to_attr b) ]
  | Formula.Mul (a, b) ->
      Dyn.M [ ("t", s "mul"); ("l", expr_to_attr a); ("r", expr_to_attr b) ]
  | Formula.Div (a, b) ->
      Dyn.M [ ("t", s "div"); ("l", expr_to_attr a); ("r", expr_to_attr b) ]

let rec expr_of_attr (v : Dyn.attribute_value) : (Formula.expr, string) result =
  let* kvs = as_map v in
  let* tag = field kvs "t" in
  let* tag_s = as_string tag in
  match tag_s with
  | "num" ->
      let* v = field kvs "v" in
      (match v with
       | Dyn.N s -> (try Ok (Formula.Num (float_of_string s))
                     with _ -> Error "bad num")
       | _ -> Error "num needs N")
  | "self" -> Ok Formula.Self
  | "ref" ->
      let* a = field kvs "a" in
      let* a_s = as_string a in
      Ok (Formula.Ref a_s)
  | "abs" ->
      let* e = field kvs "e" in
      let* e' = expr_of_attr e in
      Ok (Formula.Abs e')
  | "add" | "sub" | "mul" | "div" ->
      let* l = field kvs "l" in
      let* l' = expr_of_attr l in
      let* r = field kvs "r" in
      let* r' = expr_of_attr r in
      (match tag_s with
       | "add" -> Ok (Formula.Add (l', r'))
       | "sub" -> Ok (Formula.Sub (l', r'))
       | "mul" -> Ok (Formula.Mul (l', r'))
       | _     -> Ok (Formula.Div (l', r')))
  | other -> Error (Printf.sprintf "unknown expr tag %S" other)

let formula_to_attr (f : Formula.t) : Dyn.attribute_value =
  match f with
  | Formula.Identity -> Dyn.M [ ("kind", s "identity") ]
  | Formula.Zero     -> Dyn.M [ ("kind", s "zero") ]
  | Formula.Expr { ast; refs } ->
      let refs_m =
        List.map (fun (a, sid) -> (a, n (string_of_int (Sensor_id.id sid)))) refs
      in
      Dyn.M [
        ("kind", s "expr");
        ("ast",  expr_to_attr ast);
        ("refs", Dyn.M refs_m);
      ]

let formula_of_attr (v : Dyn.attribute_value) : (Formula.t, string) result =
  let* kvs = as_map v in
  let* kind_v = field kvs "kind" in
  let* kind_s = as_string kind_v in
  match kind_s with
  | "identity" -> Ok Formula.Identity
  | "zero"     -> Ok Formula.Zero
  | "expr" ->
      let* ast_v = field kvs "ast" in
      let* ast = expr_of_attr ast_v in
      let* refs_v = field kvs "refs" in
      let* refs_m = as_map refs_v in
      let* refs =
        List.fold_left
          (fun acc (a, (v : Dyn.attribute_value)) ->
            let* acc = acc in
            match v with
            | Dyn.N str ->
                (match int_of_string_opt str with
                 | Some i -> Ok ((a, Sensor_id.make i) :: acc)
                 | None -> Error (Printf.sprintf "bad ref id %S" str))
            | _ -> Error "expr ref must be N")
          (Ok []) refs_m
      in
      Ok (Formula.Expr { ast; refs = List.rev refs })
  | other -> Error (Printf.sprintf "unknown formula kind %S" other)

let sensor_to_item ~active (sn : Sensor.t) : (string * Dyn.attribute_value) list =
  let pk = Sensor_id.to_string sn.Sensor.id in
  let sk_t =
    if active
    then Sensor_sk.Active sn.Sensor.created
    else Sensor_sk.History sn.Sensor.created
  in
  let base =
    [
      ("pk", s pk);
      ("sk", s (Sensor_sk.to_string sk_t));
      ("type", s "sensor");
      ("daq_id", s sn.Sensor.daq_id);
      ("gsi1pk", s sensor_gsi1pk);
      ("gsi1sk", s sn.Sensor.path);
      ("purpose", s sn.Sensor.purpose);
      ("meter_type", s (Sensor.meter_type_to_string sn.Sensor.meter_type));
      ("formula", formula_to_attr sn.Sensor.formula);
      ("created", s (Ptime.to_rfc3339 ~tz_offset_s:0 sn.Sensor.created));
    ]
  in
  let with_binning =
    match sn.Sensor.binning with
    | Some b -> ("binning", n (string_of_int b)) :: base
    | None -> base
  in
  match sn.Sensor.unit with
  | Some u -> ("unit", s u) :: with_binning
  | None -> with_binning

let sensor_edge_item ~parent ~sensor_id ~created ~self_path =
  edge_with_anchor
    ~from_:(Node_id.to_string parent)
    ~to_:(Sensor_id.to_string sensor_id)
    ~kind:Edge_kind.Has_sensor
    ~name:""
    ~created
    ~self_path

let sensor_of_item kvs : (Sensor.t, string) result =
  let* pk = field kvs "pk" in
  let* pk_s = as_string pk in
  let* id = Sensor_id.of_string pk_s in
  let* daq_v = field kvs "daq_id" in
  let* daq_id = as_string daq_v in
  let* path_v = field kvs "gsi1sk" in
  let* path = as_string path_v in
  let* purpose_v = field kvs "purpose" in
  let* purpose = as_string purpose_v in
  let* mt_v = field kvs "meter_type" in
  let* mt_s = as_string mt_v in
  let* meter_type = Sensor.meter_type_of_string mt_s in
  let unit =
    match List.assoc_opt "unit" kvs with
    | Some (Dyn.S u) -> Some u
    | _ -> None
  in
  let* created_v = field kvs "created" in
  let* created_s = as_string created_v in
  let created =
    match Ptime.of_rfc3339 created_s with
    | Ok (t, _, _) -> t
    | Error _ -> Ptime.epoch
  in
  let* formula =
    match List.assoc_opt "formula" kvs with
    | Some v -> formula_of_attr v
    | None -> Ok Formula.Identity
  in
  let binning = opt_int_of_n (List.assoc_opt "binning" kvs) in
  Ok Sensor.{
    id; created; daq_id; path;
    purpose; meter_type; unit; formula; binning;
  }

(* Walk path string, return the second-to-last node-id segment as the
   parent, if any. Last segment is self. *)
let parent_from_path path =
  let parts = String.split_on_char '|' path |> List.filter (fun s -> s <> "") in
  match List.rev parts with
  | _self :: par :: _ ->
      (match Node_id.of_string par with Ok p -> Some p | Error _ -> None)
  | _ -> None

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
  let* path =
    match List.assoc_opt "gsi1sk" kvs with
    | Some (Dyn.S v) -> Ok v
    | _ -> Error (Printf.sprintf "node %s missing gsi1sk/path" (Node_id.to_string id))
  in
  let parent = parent_from_path path in
  Ok {
    Node.id;
    name;
    parent;
    path;
    created;
    metadata;
    schema;
  }

let user_item (u : User.t) =
  let uid = User_id.to_string u.User.id in
  [
    ("pk", s uid);
    ("sk", s uid);
    ("type", s "user");
    ("name", s u.User.name);
    ("cognito_group", s (Cognito_group.to_string u.User.cognito_group));
    ("language", s (Language.to_string u.User.language));
    ("currency", s (Currency.to_string u.User.currency));
    ("created", s (Ptime.to_rfc3339 ~tz_offset_s:0 u.User.created));
    ("gsi1pk", s "user");
    ("gsi1sk", s uid);
  ]

let user_of_item kvs : (User.t, string) result =
  let* pk_v = field kvs "pk" in
  let* pk_s = as_string pk_v in
  let* id = User_id.of_string pk_s in
  let* name_v = field kvs "name" in
  let* name = as_string name_v in
  let* g_v = field kvs "cognito_group" in
  let* g_s = as_string g_v in
  let* cognito_group = Cognito_group.of_string g_s in
  let language =
    match List.assoc_opt "language" kvs with
    | Some (Dyn.S s) ->
        (match Language.of_string s with
         | Ok l -> l
         | Error _ -> Language.default)
    | _ -> Language.default
  in
  let currency =
    match List.assoc_opt "currency" kvs with
    | Some (Dyn.S s) ->
        (match Currency.of_string s with
         | Ok c -> c
         | Error _ -> Currency.default)
    | _ -> Currency.default
  in
  let* c_v = field kvs "created" in
  let* c_s = as_string c_v in
  let created =
    match Ptime.of_rfc3339 c_s with
    | Ok (t, _, _) -> t
    | Error _ -> Ptime.epoch
  in
  Ok { User.id; name; cognito_group; language; currency; created }

(* Counter rows: one per HN level (and one for sensors).
   pk = "count#HN3" / sk = "count" / n = next-id allocator (monotonic),
   live = current cardinality. *)
let counter_pk_node level = Printf.sprintf "count#HN%d" (Level.depth level)
let counter_pk_sensor = "count#S"
let counter_sk = "count"

let counter_seed_item ~pk ~initial_n =
  [
    ("pk", s pk);
    ("sk", s counter_sk);
    ("type", s "counter");
    ("n", n (string_of_int initial_n));
    ("live", n "0");
  ]
