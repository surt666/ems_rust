module Dyn = Smaws_Client_DynamoDB

type cfg = { ctx : Smaws_Lib.Context.t; table : string }

let s (x : string) : Dyn.attribute_value = Dyn.S x

let pk_key id = [ ("pk", s id); ("sk", s id) ]

let get_item cfg id =
  let input =
    Dyn.make_get_item_input ~key:(pk_key (Node_id.to_string id))
      ~table_name:cfg.table ()
  in
  match Dyn.GetItem.request cfg.ctx input with
  | Error e -> failwith (Printf.sprintf "GetItem failed: %s"
                           (match e with
                            | `InternalServerError _ -> "internal"
                            | `ResourceNotFoundException _ -> "table not found"
                            | `AWSServiceError { message; _type = { name; namespace } } ->
                                Printf.sprintf "aws %s/%s: %s" namespace name
                                  (Option.value message ~default:"<none>")
                            | `HttpError _ -> "http"
                            | `JsonParseError _ -> "json parse"
                            | _ -> "other"))
  | Ok { item = None; _ } -> None
  | Ok { item = Some kvs; _ } ->
      (match Codec.node_of_item kvs with
       | Ok n -> Some n
       | Error _ -> None)

let query_child_edges cfg parent label_opt =
  let pk_val = s (Node_id.to_string parent) in
  let prefix =
    match label_opt with
    | Some lbl -> lbl
    | None -> "has_"
  in
  let input =
    Dyn.make_query_input
      ~key_condition_expression:"#pk = :pk AND begins_with(#sk, :sk)"
      ~expression_attribute_names:[ ("#pk", "pk"); ("#sk", "sk") ]
      ~expression_attribute_values:[ (":pk", pk_val); (":sk", s prefix) ]
      ~table_name:cfg.table ()
  in
  match Dyn.Query.request cfg.ctx input with
  | Error _ -> []
  | Ok { items = None; _ } -> []
  | Ok { items = Some edge_rows; _ } -> edge_rows

let query_child_refs cfg parent label_opt =
  List.filter_map
    (fun kvs ->
      match
        (List.assoc_opt "gsi1pk" kvs : Dyn.attribute_value option),
        (List.assoc_opt "name" kvs : Dyn.attribute_value option)
      with
      | Some (Dyn.S child_s), Some (Dyn.S name) ->
          (match Node_id.of_string child_s with
           | Error _ -> None
           | Ok child_id -> Some (child_id, name))
      | _ -> None)
    (query_child_edges cfg parent label_opt)

let query_children cfg parent label_opt =
  List.filter_map
    (fun kvs ->
      match (List.assoc_opt "gsi1pk" kvs : Dyn.attribute_value option) with
      | Some (Dyn.S child_s) ->
          (match Node_id.of_string child_s with
           | Error _ -> None
           | Ok child_id -> get_item cfg child_id)
      | _ -> None)
    (query_child_edges cfg parent label_opt)

let put_node cfg (nd : Node.t) =
  let input =
    Dyn.make_put_item_input ~item:(Codec.node_to_item nd)
      ~table_name:cfg.table ()
  in
  match Dyn.PutItem.request cfg.ctx input with
  | Ok _ -> ()
  | Error _ -> failwith "PutItem node failed"

let put_edge cfg ~from_ ~to_ ~kind ~name ~created =
  let item = Codec.edge_item ~from_ ~to_ ~kind ~name ~created in
  let input = Dyn.make_put_item_input ~item ~table_name:cfg.table () in
  match Dyn.PutItem.request cfg.ctx input with
  | Ok _ -> ()
  | Error _ -> failwith "PutItem edge failed"

let delete_node cfg id =
  let id_s = Node_id.to_string id in
  let _ =
    Dyn.DeleteItem.request cfg.ctx
      (Dyn.make_delete_item_input ~key:(pk_key id_s) ~table_name:cfg.table ())
  in
  let delete_where_pk_eq ~pk_attr ~sk_attr ~index_name_opt =
    let input =
      Dyn.make_query_input
        ~key_condition_expression:"#pk = :pk"
        ~expression_attribute_names:[ ("#pk", pk_attr) ]
        ~expression_attribute_values:[ (":pk", s id_s) ]
        ?index_name:index_name_opt
        ~table_name:cfg.table ()
    in
    match Dyn.Query.request cfg.ctx input with
    | Error _ -> ()
    | Ok { items = None; _ } -> ()
    | Ok { items = Some rows; _ } ->
        List.iter
          (fun kvs ->
            match List.assoc_opt pk_attr kvs, List.assoc_opt sk_attr kvs with
            | Some pk_v, Some sk_v ->
                let _ =
                  Dyn.DeleteItem.request cfg.ctx
                    (Dyn.make_delete_item_input
                       ~key:[ ("pk", pk_v); ("sk", sk_v) ]
                       ~table_name:cfg.table ())
                in
                ()
            | _ -> ())
          rows
  in
  delete_where_pk_eq ~pk_attr:"pk" ~sk_attr:"sk" ~index_name_opt:None;
  delete_where_pk_eq ~pk_attr:"gsi1pk" ~sk_attr:"gsi1sk" ~index_name_opt:(Some "gsi1")

let active_sk_prefix = "active#"
let has_sensor_sk_prefix = Edge_kind.sk_verb Edge_kind.Has_sensor ^ "#"

let put_sensor_and_edge cfg ~(sensor : Sensor.t) ~parent =
  let active_item = Codec.sensor_to_item ~active:true sensor in
  let edge_item =
    Codec.sensor_edge_item ~parent ~sensor_id:sensor.Sensor.id
      ~created:(Ptime_clock.now ())
  in
  let put_active = Dyn.make_put ~item:active_item ~table_name:cfg.table () in
  let put_edge   = Dyn.make_put ~item:edge_item   ~table_name:cfg.table () in
  let items =
    [
      Dyn.make_transact_write_item ~put:put_active ();
      Dyn.make_transact_write_item ~put:put_edge ();
    ]
  in
  let input = Dyn.make_transact_write_items_input ~transact_items:items () in
  match Dyn.TransactWriteItems.request cfg.ctx input with
  | Ok _ -> ()
  | Error _ -> failwith "TransactWriteItems put_sensor failed"

let query_active_sensor cfg (id : Sensor_id.t) =
  let input =
    Dyn.make_query_input
      ~key_condition_expression:"#pk = :pk AND begins_with(#sk, :sk)"
      ~expression_attribute_names:[ ("#pk", "pk"); ("#sk", "sk") ]
      ~expression_attribute_values:[
        (":pk", s (Sensor_id.to_string id));
        (":sk", s active_sk_prefix);
      ]
      ~limit:1
      ~scan_index_forward:false
      ~table_name:cfg.table ()
  in
  match Dyn.Query.request cfg.ctx input with
  | Error _ -> None
  | Ok { items = None; _ } -> None
  | Ok { items = Some []; _ } -> None
  | Ok { items = Some (kvs :: _); _ } ->
      (match Codec.sensor_of_item kvs with
       | Ok s -> Some s
       | Error _ -> None)

let query_sensor_ids cfg parent =
  let pk_val = s (Node_id.to_string parent) in
  let input =
    Dyn.make_query_input
      ~key_condition_expression:"#pk = :pk AND begins_with(#sk, :sk)"
      ~expression_attribute_names:[ ("#pk", "pk"); ("#sk", "sk") ]
      ~expression_attribute_values:[ (":pk", pk_val); (":sk", s has_sensor_sk_prefix) ]
      ~table_name:cfg.table ()
  in
  match Dyn.Query.request cfg.ctx input with
  | Error _ -> []
  | Ok { items = None; _ } -> []
  | Ok { items = Some rows; _ } ->
      List.filter_map
        (fun kvs ->
          match (List.assoc_opt "gsi1pk" kvs : Dyn.attribute_value option) with
          | Some (Dyn.S sid_s) ->
              (match Sensor_id.of_string sid_s with
               | Ok id -> Some id
               | Error _ -> None)
          | _ -> None)
        rows

let transact_replace cfg ~old_created ~new_sensor =
  let old_item = Codec.sensor_to_item ~active:true
    { new_sensor with Sensor.created = old_created }
  in
  let old_sk =
    match List.assoc_opt "sk" old_item with
    | Some (Dyn.S v) -> v
    | _ -> failwith "old_sk"
  in
  let old_pk =
    match List.assoc_opt "pk" old_item with
    | Some (Dyn.S v) -> v
    | _ -> failwith "old_pk"
  in
  let history_item =
    Codec.sensor_to_item ~active:false
      { new_sensor with Sensor.created = old_created }
  in
  let new_active_item = Codec.sensor_to_item ~active:true new_sensor in
  let delete =
    Dyn.make_delete
      ~key:[ ("pk", s old_pk); ("sk", s old_sk) ]
      ~table_name:cfg.table ()
  in
  let put_hist =
    Dyn.make_put ~item:history_item ~table_name:cfg.table ()
  in
  let put_new =
    Dyn.make_put ~item:new_active_item ~table_name:cfg.table ()
  in
  let items =
    [
      Dyn.make_transact_write_item ~delete ();
      Dyn.make_transact_write_item ~put:put_hist ();
      Dyn.make_transact_write_item ~put:put_new ();
    ]
  in
  let input = Dyn.make_transact_write_items_input ~transact_items:items () in
  match Dyn.TransactWriteItems.request cfg.ctx input with
  | Ok _ -> ()
  | Error _ -> failwith "TransactWriteItems replace failed"

let delete_sensor cfg (id : Sensor_id.t) (parent : Node_id.t) =
  let id_s = Sensor_id.to_string id in
  (* Delete active + history rows in the sensor's partition *)
  let input =
    Dyn.make_query_input
      ~key_condition_expression:"#pk = :pk"
      ~expression_attribute_names:[ ("#pk", "pk") ]
      ~expression_attribute_values:[ (":pk", s id_s) ]
      ~table_name:cfg.table ()
  in
  (match Dyn.Query.request cfg.ctx input with
   | Error _ -> ()
   | Ok { items = None; _ } -> ()
   | Ok { items = Some rows; _ } ->
       List.iter
         (fun kvs ->
           match List.assoc_opt "pk" kvs, List.assoc_opt "sk" kvs with
           | Some pkv, Some skv ->
               let _ = Dyn.DeleteItem.request cfg.ctx
                 (Dyn.make_delete_item_input
                    ~key:[ ("pk", pkv); ("sk", skv) ]
                    ~table_name:cfg.table ())
               in ()
           | _ -> ())
         rows);
  (* Delete the parent edge row *)
  let edge_sk = has_sensor_sk_prefix ^ id_s in
  let _ =
    Dyn.DeleteItem.request cfg.ctx
      (Dyn.make_delete_item_input
         ~key:[ ("pk", s (Node_id.to_string parent)); ("sk", s edge_sk) ]
         ~table_name:cfg.table ())
  in
  ()

let run (cfg : cfg) (f : unit -> 'a) : 'a =
  let open Effect.Deep in
  try_with f ()
    {
      effc =
        (fun (type a) (eff : a Effect.t) ->
          match eff with
          | Effects.Gen_uuid () ->
              let v =
                match Uuidm.v4_gen (Random.State.make_self_init ()) () with
                | u -> u
              in
              Some (fun (k : (a, _) continuation) -> continue k v)
          | Effects.Now () ->
              Some (fun k -> continue k (Ptime_clock.now ()))
          | Effects.Get_node id ->
              Some (fun k -> continue k (get_item cfg id))
          | Effects.Get_schema id ->
              let schema_opt =
                match get_item cfg id with
                | Some n -> n.Node.schema
                | None -> None
              in
              Some (fun k -> continue k schema_opt)
          | Effects.List_children (parent, label_opt) ->
              Some (fun k -> continue k (query_children cfg parent label_opt))
          | Effects.List_child_refs (parent, label_opt) ->
              Some (fun k -> continue k (query_child_refs cfg parent label_opt))
          | Effects.Put_node n ->
              put_node cfg n;
              Some (fun k -> continue k ())
          | Effects.Put_edge { from_; to_; kind; name; created } ->
              put_edge cfg ~from_ ~to_ ~kind ~name ~created;
              Some (fun k -> continue k ())
          | Effects.Delete_node id ->
              delete_node cfg id;
              Some (fun k -> continue k ())
          | Effects.Put_sensor { sensor; parent } ->
              put_sensor_and_edge cfg ~sensor ~parent;
              Some (fun k -> continue k ())
          | Effects.Get_active_sensor id ->
              Some (fun k -> continue k (query_active_sensor cfg id))
          | Effects.List_sensor_ids parent ->
              Some (fun k -> continue k (query_sensor_ids cfg parent))
          | Effects.Replace_sensor_device { old_created; new_sensor } ->
              transact_replace cfg ~old_created ~new_sensor;
              Some (fun k -> continue k ())
          | Effects.Delete_sensor { sensor_id; parent } ->
              delete_sensor cfg sensor_id parent;
              Some (fun k -> continue k ())
          | Effects.Get_sensor_reading _ ->
              (* raw readings come from the flink-optimized table, not this one *)
              Some (fun k -> continue k None)
          | _ -> None);
    }
