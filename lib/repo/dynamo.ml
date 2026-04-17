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
                            | _ -> "other"))
  | Ok { item = None; _ } -> None
  | Ok { item = Some kvs; _ } ->
      (match Codec.node_of_item kvs with
       | Ok n -> Some n
       | Error _ -> None)

let query_children cfg parent label_opt =
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
  | Ok { items = Some edge_rows; _ } ->
      List.filter_map
        (fun kvs ->
          match (List.assoc_opt "gsi1pk" kvs : Dyn.attribute_value option) with
          | Some (Dyn.S child_s) ->
              (match Node_id.of_string child_s with
               | Error _ -> None
               | Ok child_id -> get_item cfg child_id)
          | _ -> None)
        edge_rows

let put_node cfg (nd : Node.t) =
  let input =
    Dyn.make_put_item_input ~item:(Codec.node_to_item nd)
      ~table_name:cfg.table ()
  in
  match Dyn.PutItem.request cfg.ctx input with
  | Ok _ -> ()
  | Error _ -> failwith "PutItem node failed"

let put_edge cfg ~from_ ~to_ ~label =
  let item = Codec.edge_item ~from_ ~to_ ~label ~created:(Ptime_clock.now ()) in
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
          | Effects.Put_node n ->
              put_node cfg n;
              Some (fun k -> continue k ())
          | Effects.Put_edge { from_; to_; label } ->
              put_edge cfg ~from_ ~to_ ~label;
              Some (fun k -> continue k ())
          | Effects.Delete_node id ->
              delete_node cfg id;
              Some (fun k -> continue k ())
          | _ -> None);
    }
