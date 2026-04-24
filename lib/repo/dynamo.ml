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

let query_child_edges cfg parent kind_opt =
  let pk_val = s (Node_id.to_string parent) in
  let input =
    match kind_opt with
    | Some k ->
        Dyn.make_query_input
          ~key_condition_expression:"#pk = :pk AND begins_with(#sk, :sk)"
          ~expression_attribute_names:[ ("#pk", "pk"); ("#sk", "sk") ]
          ~expression_attribute_values:[
            (":pk", pk_val);
            (":sk", s (Edge_kind.sk_verb k ^ "#"));
          ]
          ~table_name:cfg.table ()
    | None ->
        Dyn.make_query_input
          ~key_condition_expression:"#pk = :pk"
          ~expression_attribute_names:[ ("#pk", "pk") ]
          ~expression_attribute_values:[ (":pk", pk_val) ]
          ~table_name:cfg.table ()
  in
  match Dyn.Query.request cfg.ctx input with
  | Error _ -> []
  | Ok { items = None; _ } -> []
  | Ok { items = Some edge_rows; _ } -> edge_rows

let query_child_refs cfg parent kind_opt =
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
    (query_child_edges cfg parent kind_opt)

let query_children cfg parent kind_opt =
  List.filter_map
    (fun kvs ->
      match (List.assoc_opt "gsi1pk" kvs : Dyn.attribute_value option) with
      | Some (Dyn.S child_s) ->
          (match Node_id.of_string child_s with
           | Error _ -> None
           | Ok child_id -> get_item cfg child_id)
      | _ -> None)
    (query_child_edges cfg parent kind_opt)

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
      (* Prefer the dedicated `sensor_id` attribute; fall back to `gsi1pk` for
         older edges, then parse the sk suffix (has_sensor#S#<uuid>). *)
      List.filter_map
        (fun kvs ->
          let from_attr k =
            match (List.assoc_opt k kvs : Dyn.attribute_value option) with
            | Some (Dyn.S s) -> Some s
            | _ -> None
          in
          let from_sk () =
            match from_attr "sk" with
            | Some sk when
                String.length sk > String.length has_sensor_sk_prefix
                && String.sub sk 0 (String.length has_sensor_sk_prefix)
                   = has_sensor_sk_prefix ->
                Some
                  (String.sub sk (String.length has_sensor_sk_prefix)
                     (String.length sk - String.length has_sensor_sk_prefix))
            | _ -> None
          in
          let sid_s =
            match from_attr "sensor_id" with
            | Some s -> Some s
            | None ->
                (match from_attr "gsi1pk" with
                 | Some s -> Some s
                 | None -> from_sk ())
          in
          match sid_s with
          | Some s ->
              (match Sensor_id.of_string s with
               | Ok id -> Some id
               | Error _ -> None)
          | None -> None)
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

let put_user cfg (u : User.t) =
  let input =
    Dyn.make_put_item_input
      ~item:(Codec.user_item u) ~table_name:cfg.table ()
  in
  match Dyn.PutItem.request cfg.ctx input with
  | Ok _ -> ()
  | Error _ -> failwith "PutItem user failed"

let get_user cfg (id : User_id.t) =
  let key = pk_key (User_id.to_string id) in
  let input = Dyn.make_get_item_input ~key ~table_name:cfg.table () in
  match Dyn.GetItem.request cfg.ctx input with
  | Error _ -> None
  | Ok { item = None; _ } -> None
  | Ok { item = Some kvs; _ } ->
      (match Codec.user_of_item kvs with
       | Ok u -> Some u
       | Error _ -> None)

let list_users cfg =
  let input =
    Dyn.make_query_input
      ~key_condition_expression:"#pk = :pk"
      ~expression_attribute_names:[ ("#pk", "gsi1pk") ]
      ~expression_attribute_values:[ (":pk", s "user") ]
      ~index_name:"gsi1"
      ~table_name:cfg.table ()
  in
  match Dyn.Query.request cfg.ctx input with
  | Error _ -> []
  | Ok { items = None; _ } -> []
  | Ok { items = Some rows; _ } ->
      List.filter_map
        (fun kvs ->
          match Codec.user_of_item kvs with
          | Ok u -> Some u
          | Error _ -> None)
        rows

let delete_user cfg (id : User_id.t) =
  let uid = User_id.to_string id in
  let _ =
    Dyn.DeleteItem.request cfg.ctx
      (Dyn.make_delete_item_input
         ~key:[ ("pk", s uid); ("sk", s uid) ]
         ~table_name:cfg.table ())
  in
  ()

let query_blocked_nodes cfg (user_id : User_id.t) =
  let pk_val = s (User_id.to_string user_id) in
  let prefix = Edge_kind.sk_verb Edge_kind.Blocked ^ "#" in
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
  | Ok { items = Some rows; _ } ->
      List.filter_map
        (fun kvs ->
          match (List.assoc_opt "gsi1pk" kvs : Dyn.attribute_value option) with
          | Some (Dyn.S child_s) ->
              (match Node_id.of_string child_s with
               | Ok id -> Some id
               | Error _ -> None)
          | _ -> None)
        rows

let query_administrated_nodes cfg (user_id : User_id.t) =
  let pk_val = s (User_id.to_string user_id) in
  let prefix = Edge_kind.sk_verb Edge_kind.Administrates ^ "#" in
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
  | Ok { items = Some rows; _ } ->
      List.filter_map
        (fun kvs ->
          let from_gsi =
            match (List.assoc_opt "gsi1pk" kvs : Dyn.attribute_value option) with
            | Some (Dyn.S v) -> Some v
            | _ -> None
          in
          let from_sk () =
            match (List.assoc_opt "sk" kvs : Dyn.attribute_value option) with
            | Some (Dyn.S sk) when
                String.length sk > String.length prefix
                && String.sub sk 0 (String.length prefix) = prefix ->
                Some (String.sub sk (String.length prefix)
                        (String.length sk - String.length prefix))
            | _ -> None
          in
          let nid_s =
            match from_gsi with
            | Some v -> Some v
            | None -> from_sk ()
          in
          match nid_s with
          | Some s ->
              (match Node_id.of_string s with
               | Ok id -> Some id
               | Error _ -> None)
          | None -> None)
        rows

let query_blocked_users cfg (node_id : Node_id.t) =
  let pk_val = s (Node_id.to_string node_id) in
  let prefix = Edge_kind.gsi_verb Edge_kind.Blocked ^ "#" in
  let input =
    Dyn.make_query_input
      ~key_condition_expression:"#pk = :pk AND begins_with(#sk, :sk)"
      ~expression_attribute_names:[ ("#pk", "gsi1pk"); ("#sk", "gsi1sk") ]
      ~expression_attribute_values:[ (":pk", pk_val); (":sk", s prefix) ]
      ~index_name:"gsi1"
      ~table_name:cfg.table ()
  in
  match Dyn.Query.request cfg.ctx input with
  | Error _ -> []
  | Ok { items = None; _ } -> []
  | Ok { items = Some rows; _ } ->
      List.filter_map
        (fun kvs ->
          match (List.assoc_opt "pk" kvs : Dyn.attribute_value option) with
          | Some (Dyn.S user_s) ->
              (match User_id.of_string user_s with
               | Ok id -> Some id
               | Error _ -> None)
          | _ -> None)
        rows

let delete_edge cfg ~from_ ~to_ ~kind =
  let sk = Printf.sprintf "%s#%s" (Edge_kind.sk_verb kind) to_ in
  let _ =
    Dyn.DeleteItem.request cfg.ctx
      (Dyn.make_delete_item_input
         ~key:[ ("pk", s from_); ("sk", s sk) ]
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
          | Effects.List_children (parent, kind_opt) ->
              Some (fun k -> continue k (query_children cfg parent kind_opt))
          | Effects.List_child_refs (parent, kind_opt) ->
              Some (fun k -> continue k (query_child_refs cfg parent kind_opt))
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
          | Effects.Put_user u ->
              put_user cfg u;
              Some (fun k -> continue k ())
          | Effects.Get_user id ->
              Some (fun k -> continue k (get_user cfg id))
          | Effects.List_users () ->
              Some (fun k -> continue k (list_users cfg))
          | Effects.Delete_user id ->
              delete_user cfg id;
              Some (fun k -> continue k ())
          | Effects.List_blocked_nodes id ->
              Some (fun k -> continue k (query_blocked_nodes cfg id))
          | Effects.List_blocked_users id ->
              Some (fun k -> continue k (query_blocked_users cfg id))
          | Effects.List_administrated_nodes id ->
              Some (fun k -> continue k (query_administrated_nodes cfg id))
          | Effects.Delete_edge { from_; to_; kind } ->
              delete_edge cfg ~from_ ~to_ ~kind;
              Some (fun k -> continue k ())
          | _ -> None);
    }
