module Dyn = Smaws_Client_DynamoDB

type cfg = { ctx : Smaws_Lib.Context.t; table : string }

let s (x : string) : Dyn.attribute_value = Dyn.S x
let n (x : string) : Dyn.attribute_value = Dyn.N x

let pk_key id = [ ("pk", s id); ("sk", s id) ]

let counter_key pk =
  [ ("pk", s pk); ("sk", s Codec.counter_sk) ]

(* ----------------------------------------------------------------------
   Counter ops
   --------------------------------------------------------------------- *)

(* Read counter; returns (n, live) or None if the row is missing. *)
let read_counter cfg ~pk =
  let input =
    Dyn.make_get_item_input ~key:(counter_key pk)
      ~table_name:cfg.table ()
  in
  match Dyn.GetItem.request cfg.ctx input with
  | Error _ -> None
  | Ok { item = None; _ } -> None
  | Ok { item = Some kvs; _ } ->
      let read k =
        match List.assoc_opt k kvs with
        | Some (Dyn.N v) -> int_of_string_opt v
        | _ -> None
      in
      (match read "n", read "live" with
       | Some n, Some l -> Some (n, l)
       | Some n, None -> Some (n, 0)
       | _ -> None)

(* Idempotent counter seed; never overwrites an existing row. *)
let seed_counter cfg ~pk ~initial_n =
  let input =
    Dyn.make_put_item_input
      ~item:(Codec.counter_seed_item ~pk ~initial_n)
      ~table_name:cfg.table
      ~condition_expression:"attribute_not_exists(pk)"
      ()
  in
  ignore (Dyn.PutItem.request cfg.ctx input)

(* Adjust live by delta (no condition; used on delete-time cleanup). *)
let bump_live cfg ~pk ~delta =
  let input =
    Dyn.make_update_item_input
      ~table_name:cfg.table
      ~key:(counter_key pk)
      ~update_expression:"ADD #live :d"
      ~expression_attribute_names:[ ("#live", "live") ]
      ~expression_attribute_values:[ (":d", n (string_of_int delta)) ]
      ()
  in
  ignore (Dyn.UpdateItem.request cfg.ctx input)

(* ----------------------------------------------------------------------
   Atomic add: TransactWriteItems with conditional counter update + node put
   + edge put. Retries on counter contention.
   --------------------------------------------------------------------- *)

let max_alloc_retries = 5

(* Returns Ok () or Error `Counter_race / `Conflict / `Other reason. *)
let try_transact_alloc cfg ~counter_pk ~current_n ~next_n
    ~node_item ~edge_item =
  let upd =
    Dyn.make_update
      ~table_name:cfg.table
      ~key:(counter_key counter_pk)
      ~update_expression:"SET #n = :next ADD #live :one"
      ~condition_expression:"#n = :current"
      ~expression_attribute_names:[ ("#n", "n"); ("#live", "live") ]
      ~expression_attribute_values:[
        (":next", n (string_of_int next_n));
        (":current", n (string_of_int current_n));
        (":one", n "1");
      ]
      ()
  in
  let put_node =
    Dyn.make_put
      ~table_name:cfg.table
      ~item:node_item
      ~condition_expression:"attribute_not_exists(pk)"
      ()
  in
  let put_edge =
    Dyn.make_put
      ~table_name:cfg.table
      ~item:edge_item
      ~condition_expression:"attribute_not_exists(sk)"
      ()
  in
  let items =
    [
      Dyn.make_transact_write_item ~update:upd ();
      Dyn.make_transact_write_item ~put:put_node ();
      Dyn.make_transact_write_item ~put:put_edge ();
    ]
  in
  let input = Dyn.make_transact_write_items_input ~transact_items:items () in
  match Dyn.TransactWriteItems.request cfg.ctx input with
  | Ok _ -> Ok ()
  | Error (`TransactionCanceledException { cancellation_reasons = Some rs; _ }) ->
      (* index 0 = counter, 1 = node, 2 = edge. The counter cond fails
         on race; node cond fails on duplicate id (real conflict). *)
      let code_at i =
        match List.nth_opt rs i with
        | Some { Dyn.code = Some c; _ } -> c
        | _ -> ""
      in
      let counter_code = code_at 0 in
      let node_code = code_at 1 in
      let edge_code = code_at 2 in
      if counter_code = "ConditionalCheckFailed" then Error `Counter_race
      else if node_code = "ConditionalCheckFailed"
              || edge_code = "ConditionalCheckFailed" then
        Error (`Conflict "id collision on put")
      else Error (`Other "transact canceled")
  | Error _ -> Error (`Other "transact failed")

let allocate_and_put_node cfg ~level ~build =
  let counter_pk = Codec.counter_pk_node level in
  let rec attempt remaining =
    if remaining <= 0 then
      Error (Errors.Conflict "counter contention exceeded retries")
    else
      let current_n =
        match read_counter cfg ~pk:counter_pk with
        | Some (n, _) -> n
        | None ->
            (* Lazy seed if missing — should normally be done in setup. *)
            seed_counter cfg ~pk:counter_pk ~initial_n:10000;
            10000
      in
      let next_n = current_n + 1 in
      let (node, edge) = build ~id:next_n in
      let node_item = Codec.node_to_item node in
      let edge_item =
        let self_path = Option.value edge.Effects.self_path ~default:"" in
        Codec.edge_with_anchor
          ~from_:edge.from_ ~to_:edge.to_ ~kind:edge.kind
          ~name:edge.name ~created:edge.created ~self_path
      in
      match try_transact_alloc cfg
              ~counter_pk ~current_n ~next_n ~node_item ~edge_item with
      | Ok () -> Ok node
      | Error `Counter_race -> attempt (remaining - 1)
      | Error (`Conflict m) -> Error (Errors.Conflict m)
      | Error (`Other m) -> Error (Errors.Internal m)
  in
  attempt max_alloc_retries

let allocate_and_put_sensor cfg ~build =
  let counter_pk = Codec.counter_pk_sensor in
  let rec attempt remaining =
    if remaining <= 0 then
      Error (Errors.Conflict "counter contention exceeded retries")
    else
      let current_n =
        match read_counter cfg ~pk:counter_pk with
        | Some (n, _) -> n
        | None ->
            seed_counter cfg ~pk:counter_pk ~initial_n:10000;
            10000
      in
      let next_n = current_n + 1 in
      let (sensor, edge) = build ~id:next_n in
      let sensor_item = Codec.sensor_to_item ~active:true sensor in
      let edge_item =
        let self_path = Option.value edge.Effects.self_path ~default:"" in
        Codec.edge_with_anchor
          ~from_:edge.from_ ~to_:edge.to_ ~kind:edge.kind
          ~name:edge.name ~created:edge.created ~self_path
      in
      match try_transact_alloc cfg
              ~counter_pk ~current_n ~next_n
              ~node_item:sensor_item ~edge_item with
      | Ok () -> Ok sensor
      | Error `Counter_race -> attempt (remaining - 1)
      | Error (`Conflict m) -> Error (Errors.Conflict m)
      | Error (`Other m) -> Error (Errors.Internal m)
  in
  attempt max_alloc_retries

(* ----------------------------------------------------------------------
   Reads
   --------------------------------------------------------------------- *)

let get_item cfg id =
  let input =
    Dyn.make_get_item_input ~key:(pk_key (Node_id.to_string id))
      ~table_name:cfg.table ()
  in
  match Dyn.GetItem.request cfg.ctx input with
  | Error _ -> None
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
          ~key_condition_expression:"#pk = :pk AND begins_with(#sk, :sk)"
          ~expression_attribute_names:[ ("#pk", "pk"); ("#sk", "sk") ]
          ~expression_attribute_values:[
            (":pk", pk_val);
            (":sk", s "has_");
          ]
          ~table_name:cfg.table ()
  in
  match Dyn.Query.request cfg.ctx input with
  | Error _ -> []
  | Ok { items = None; _ } -> []
  | Ok { items = Some edge_rows; _ } -> edge_rows

(* Edge sk has shape "has_<label>#<child_id>" or "has_sensor#<sensor_id>".
   Strip the leading "has_<verb>#" prefix to recover the child/sensor id. *)
let child_id_from_edge_sk sk =
  match String.index_opt sk '#' with
  | None -> None
  | Some i ->
      let rest = String.sub sk (i + 1) (String.length sk - i - 1) in
      Some rest

let query_child_refs cfg parent kind_opt =
  List.filter_map
    (fun kvs ->
      match
        (List.assoc_opt "sk" kvs : Dyn.attribute_value option),
        (List.assoc_opt "name" kvs : Dyn.attribute_value option)
      with
      | Some (Dyn.S sk), Some (Dyn.S name) ->
          (match child_id_from_edge_sk sk with
           | None -> None
           | Some child_s ->
               (match Node_id.of_string child_s with
                | Error _ -> None
                | Ok child_id -> Some (child_id, name)))
      | _ -> None)
    (query_child_edges cfg parent
       (match kind_opt with
        | Some (Edge_kind.Has_sensor) -> kind_opt
        | _ -> kind_opt))

let query_children cfg parent kind_opt =
  List.filter_map
    (fun kvs ->
      match (List.assoc_opt "sk" kvs : Dyn.attribute_value option) with
      | Some (Dyn.S sk)
        when not (String.starts_with ~prefix:"has_sensor#" sk) ->
          (match child_id_from_edge_sk sk with
           | None -> None
           | Some child_s ->
               (match Node_id.of_string child_s with
                | Error _ -> None
                | Ok child_id -> get_item cfg child_id))
      | _ -> None)
    (query_child_edges cfg parent kind_opt)

(* ----------------------------------------------------------------------
   Writes (non-allocating)
   --------------------------------------------------------------------- *)

let put_node cfg (nd : Node.t) =
  let input =
    Dyn.make_put_item_input ~item:(Codec.node_to_item nd)
      ~table_name:cfg.table ()
  in
  match Dyn.PutItem.request cfg.ctx input with
  | Ok _ -> ()
  | Error _ -> failwith "PutItem node failed"

let put_edge_spec cfg (es : Effects.edge_spec) =
  let item =
    match es.self_path with
    | Some self_path ->
        Codec.edge_with_anchor
          ~from_:es.from_ ~to_:es.to_ ~kind:es.kind
          ~name:es.name ~created:es.created ~self_path
    | None ->
        Codec.edge_item
          ~from_:es.from_ ~to_:es.to_ ~kind:es.kind
          ~name:es.name ~created:es.created
  in
  let input = Dyn.make_put_item_input ~item ~table_name:cfg.table () in
  match Dyn.PutItem.request cfg.ctx input with
  | Ok _ -> ()
  | Error _ -> failwith "PutItem edge failed"

(* ----------------------------------------------------------------------
   Delete: one GSI Query per level partition + one for sensors.
   --------------------------------------------------------------------- *)

let active_sk_prefix = "active#"
let has_sensor_sk_prefix = "has_sensor#"

(* Page through GSI partition [gsi1pk_v] picking up rows whose gsi1sk
   begins with [path_prefix]. *)
let query_gsi_partition cfg ~gsi1pk_v ~path_prefix =
  let rec loop acc start_key =
    let input =
      Dyn.make_query_input
        ~key_condition_expression:"#pk = :pk AND begins_with(#sk, :sk)"
        ~expression_attribute_names:[ ("#pk", "gsi1pk"); ("#sk", "gsi1sk") ]
        ~expression_attribute_values:[
          (":pk", s gsi1pk_v);
          (":sk", s path_prefix);
        ]
        ~index_name:"gsi1"
        ?exclusive_start_key:start_key
        ~table_name:cfg.table ()
    in
    match Dyn.Query.request cfg.ctx input with
    | Error _ -> acc
    | Ok { items; last_evaluated_key; _ } ->
        let acc =
          match items with
          | None -> acc
          | Some xs -> List.rev_append xs acc
        in
        match last_evaluated_key with
        | None | Some [] -> acc
        | Some _ as k -> loop acc k
  in
  List.rev (loop [] None)

(* Delete a list of (pk, sk) pairs in 25-row BatchWriteItem batches. *)
let batch_delete cfg keys =
  let rec chunks acc lst =
    match lst with
    | [] -> List.rev acc
    | _ ->
        let head, rest =
          let rec take i acc = function
            | [] -> List.rev acc, []
            | xs when i = 0 -> List.rev acc, xs
            | x :: xs -> take (i - 1) (x :: acc) xs
          in
          take 25 [] lst
        in
        chunks (head :: acc) rest
  in
  List.iter
    (fun batch ->
      let requests =
        List.map
          (fun (pk_v, sk_v) ->
            Dyn.make_write_request
              ~delete_request:(Dyn.make_delete_request
                                 ~key:[ ("pk", pk_v); ("sk", sk_v) ] ())
              ())
          batch
      in
      let input =
        Dyn.make_batch_write_item_input
          ~request_items:[ (cfg.table, requests) ] ()
      in
      ignore (Dyn.BatchWriteItem.request cfg.ctx input))
    (chunks [] keys)

let levels_at_or_below lvl =
  let rec loop d acc =
    match Level.of_depth d with
    | None -> List.rev acc
    | Some l -> loop (d + 1) (l :: acc)
  in
  loop (Level.depth lvl) []

(* Delete the node at [id] and everything beneath it. Strategy:
   - the deleted node lives in the gsi1 partition for its own level;
     query that partition with `gsi1sk begins_with self.path` -> picks up
     the node itself + the parent-side `has_<label>` edge that points at it.
   - for each strictly-deeper level (down to HN9), do the same query in
     that level's partition -> picks up descendant nodes + the edges
     terminating at them.
   - in the "S" partition, query `gsi1sk begins_with self.path` -> picks
     up all sensor active/history rows + has_sensor edges in the subtree.
   Per-level counts are decremented on count#HN<n>.live; sensor count on
   count#S.live. *)
let delete_subtree cfg id =
  match get_item cfg id with
  | None -> ()
  | Some n ->
      let path_prefix = n.Node.path in
      let starting_lvl = Node_id.level n.Node.id in
      let levels = levels_at_or_below starting_lvl in
      let total_keys = ref [] in
      let level_counts = Hashtbl.create 8 in
      let sensor_count = ref 0 in
      List.iter
        (fun lvl ->
          let gsi1pk_v = Codec.node_gsi1pk lvl in
          let rows = query_gsi_partition cfg ~gsi1pk_v ~path_prefix in
          let nodes_in_lvl = ref 0 in
          List.iter
            (fun kvs ->
              (match List.assoc_opt "pk" kvs, List.assoc_opt "sk" kvs with
               | Some pkv, Some skv ->
                   total_keys := (pkv, skv) :: !total_keys
               | _ -> ());
              match (List.assoc_opt "type" kvs : Dyn.attribute_value option) with
              | Some (Dyn.S "node") -> incr nodes_in_lvl
              | _ -> ())
            rows;
          if !nodes_in_lvl > 0 then
            Hashtbl.replace level_counts lvl !nodes_in_lvl)
        levels;
      let sensor_rows =
        query_gsi_partition cfg ~gsi1pk_v:Codec.sensor_gsi1pk ~path_prefix
      in
      List.iter
        (fun kvs ->
          (match List.assoc_opt "pk" kvs, List.assoc_opt "sk" kvs with
           | Some pkv, Some skv ->
               total_keys := (pkv, skv) :: !total_keys
           | _ -> ());
          match (List.assoc_opt "type" kvs : Dyn.attribute_value option),
                (List.assoc_opt "sk" kvs : Dyn.attribute_value option) with
          | Some (Dyn.S "sensor"), Some (Dyn.S sk)
            when String.starts_with ~prefix:active_sk_prefix sk ->
              incr sensor_count
          | _ -> ())
        sensor_rows;
      batch_delete cfg (List.rev !total_keys);
      Hashtbl.iter
        (fun lvl count ->
          bump_live cfg
            ~pk:(Codec.counter_pk_node lvl)
            ~delta:(-count))
        level_counts;
      if !sensor_count > 0 then
        bump_live cfg ~pk:Codec.counter_pk_sensor
          ~delta:(- !sensor_count)

(* ----------------------------------------------------------------------
   Sensor active/history reads + replace + delete
   --------------------------------------------------------------------- *)

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
          match (List.assoc_opt "sk" kvs : Dyn.attribute_value option) with
          | Some (Dyn.S sk) ->
              let plen = String.length has_sensor_sk_prefix in
              if String.starts_with ~prefix:has_sensor_sk_prefix sk
              then
                let rest = String.sub sk plen (String.length sk - plen) in
                (match Sensor_id.of_string rest with
                 | Ok id -> Some id
                 | Error _ -> None)
              else None
          | _ -> None)
        rows

(* All active sensors whose gsi1sk (= path) begins with [path_prefix].
   gsi1 projects ALL, so items decode directly. *)
let query_sensors_under_path cfg ~path_prefix =
  let rows = query_gsi_partition cfg ~gsi1pk_v:Codec.sensor_gsi1pk ~path_prefix in
  List.filter_map
    (fun kvs ->
      match (List.assoc_opt "sk" kvs : Dyn.attribute_value option) with
      | Some (Dyn.S sk) when String.starts_with ~prefix:active_sk_prefix sk ->
          (match Codec.sensor_of_item kvs with Ok s -> Some s | Error _ -> None)
      | _ -> None)
    rows

let transact_replace cfg ~old_created ~new_sensor =
  let old_item =
    Codec.sensor_to_item ~active:true
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
  bump_live cfg ~pk:Codec.counter_pk_sensor ~delta:(-1)

(* ----------------------------------------------------------------------
   Users, blocking, administrating
   --------------------------------------------------------------------- *)

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
          match (List.assoc_opt "sk" kvs : Dyn.attribute_value option) with
          | Some (Dyn.S sk) ->
              let plen = String.length prefix in
              if String.starts_with ~prefix sk
              then
                let rest = String.sub sk plen (String.length sk - plen) in
                (match Node_id.of_string rest with
                 | Ok id -> Some id
                 | Error _ -> None)
              else None
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
          match (List.assoc_opt "sk" kvs : Dyn.attribute_value option) with
          | Some (Dyn.S sk) ->
              let plen = String.length prefix in
              if String.starts_with ~prefix sk
              then
                let rest = String.sub sk plen (String.length sk - plen) in
                (match Node_id.of_string rest with
                 | Ok id -> Some id
                 | Error _ -> None)
              else None
          | _ -> None)
        rows

let query_blocked_users cfg (node_id : Node_id.t) =
  let pk_val = s (Node_id.to_string node_id) in
  let prefix = "blocks#" in
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

(* ----------------------------------------------------------------------
   Effect handler
   --------------------------------------------------------------------- *)

let run (cfg : cfg) (f : unit -> 'a) : 'a =
  let open Effect.Deep in
  try_with f ()
    {
      effc =
        (fun (type a) (eff : a Effect.t) ->
          match eff with
          | Effects.Now () ->
              Some (fun (k : (a, _) continuation) -> continue k (Ptime_clock.now ()))
          | Effects.Get_node id ->
              Some (fun (k : (a, _) continuation) -> continue k (get_item cfg id))
          | Effects.List_children (parent, kind_opt) ->
              Some (fun (k : (a, _) continuation) ->
                continue k (query_children cfg parent kind_opt))
          | Effects.List_child_refs (parent, kind_opt) ->
              Some (fun (k : (a, _) continuation) ->
                continue k (query_child_refs cfg parent kind_opt))
          | Effects.Add_node { level; build } ->
              Some (fun (k : (a, _) continuation) ->
                continue k (allocate_and_put_node cfg ~level ~build))
          | Effects.Put_node n ->
              put_node cfg n;
              Some (fun (k : (a, _) continuation) -> continue k ())
          | Effects.Put_edge es ->
              put_edge_spec cfg es;
              Some (fun (k : (a, _) continuation) -> continue k ())
          | Effects.Delete_node id ->
              delete_subtree cfg id;
              Some (fun (k : (a, _) continuation) -> continue k ())
          | Effects.Add_sensor { build } ->
              Some (fun (k : (a, _) continuation) ->
                continue k (allocate_and_put_sensor cfg ~build))
          | Effects.Get_active_sensor id ->
              Some (fun (k : (a, _) continuation) ->
                continue k (query_active_sensor cfg id))
          | Effects.List_sensor_ids parent ->
              Some (fun (k : (a, _) continuation) ->
                continue k (query_sensor_ids cfg parent))
          | Effects.List_sensors_under_path prefix ->
              Some (fun (k : (a, _) continuation) ->
                continue k (query_sensors_under_path cfg ~path_prefix:prefix))
          | Effects.Replace_sensor_device { old_created; new_sensor } ->
              transact_replace cfg ~old_created ~new_sensor;
              Some (fun (k : (a, _) continuation) -> continue k ())
          | Effects.Delete_sensor { sensor_id; parent } ->
              delete_sensor cfg sensor_id parent;
              Some (fun (k : (a, _) continuation) -> continue k ())
          | Effects.Get_sensor_reading _ ->
              Some (fun (k : (a, _) continuation) -> continue k None)
          | Effects.Put_user u ->
              put_user cfg u;
              Some (fun (k : (a, _) continuation) -> continue k ())
          | Effects.Get_user id ->
              Some (fun (k : (a, _) continuation) -> continue k (get_user cfg id))
          | Effects.List_users () ->
              Some (fun (k : (a, _) continuation) -> continue k (list_users cfg))
          | Effects.Delete_user id ->
              delete_user cfg id;
              Some (fun (k : (a, _) continuation) -> continue k ())
          | Effects.List_blocked_nodes id ->
              Some (fun (k : (a, _) continuation) ->
                continue k (query_blocked_nodes cfg id))
          | Effects.List_blocked_users id ->
              Some (fun (k : (a, _) continuation) ->
                continue k (query_blocked_users cfg id))
          | Effects.List_administrated_nodes id ->
              Some (fun (k : (a, _) continuation) ->
                continue k (query_administrated_nodes cfg id))
          | Effects.Delete_edge { from_; to_; kind } ->
              delete_edge cfg ~from_ ~to_ ~kind;
              Some (fun (k : (a, _) continuation) -> continue k ())
          | _ -> None);
    }
