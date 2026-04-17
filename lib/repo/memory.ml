type state = {
  nodes    : (string, Node.t) Hashtbl.t;
  edges    : (Node_id.t * string * Node_id.t) list ref;
  rng      : Random.State.t;
  clock    : unit -> Ptime.t;
}

let empty ?(seed = 42) ?(clock = Ptime_clock.now) () =
  {
    nodes = Hashtbl.create 32;
    edges = ref [];
    rng = Random.State.make [| seed |];
    clock;
  }

let fresh_uuid rng =
  let bytes = Bytes.create 16 in
  for i = 0 to 15 do Bytes.set_uint8 bytes i (Random.State.int rng 256) done;
  Bytes.set_uint8 bytes 6 (0x40 lor (Bytes.get_uint8 bytes 6 land 0x0f));
  Bytes.set_uint8 bytes 8 (0x80 lor (Bytes.get_uint8 bytes 8 land 0x3f));
  Uuidm.unsafe_of_binary_string (Bytes.unsafe_to_string bytes)

let find_node st id =
  Hashtbl.find_opt st.nodes (Node_id.to_string id)

let run (st : state) (f : unit -> 'a) : 'a =
  let open Effect.Deep in
  try_with f ()
    {
      effc =
        (fun (type a) (eff : a Effect.t) ->
          match eff with
          | Effects.Gen_uuid () ->
              Some (fun (k : (a, _) continuation) -> continue k (fresh_uuid st.rng))
          | Effects.Now () ->
              Some (fun k -> continue k (st.clock ()))
          | Effects.Get_node id ->
              Some (fun k -> continue k (find_node st id))
          | Effects.Get_schema id ->
              let schema = Option.bind (find_node st id) (fun n -> n.Node.schema) in
              Some (fun k -> continue k schema)
          | Effects.List_children (parent, label_opt) ->
              let matches =
                List.filter
                  (fun (p, lbl, _c) ->
                    Node_id.equal p parent
                    && (match label_opt with
                        | None -> true
                        | Some needed ->
                            let bare =
                              if String.length needed > 4
                                 && String.sub needed 0 4 = "has_"
                              then String.sub needed 4 (String.length needed - 4)
                              else needed
                            in
                            let bare =
                              try
                                let i = String.index bare '#' in
                                String.sub bare 0 i
                              with Not_found -> bare
                            in
                            lbl = bare))
                  !(st.edges)
              in
              let children =
                List.filter_map
                  (fun (_p, _lbl, c) -> find_node st c)
                  matches
              in
              Some (fun k -> continue k children)
          | Effects.Put_node n ->
              Hashtbl.replace st.nodes (Node_id.to_string n.Node.id) n;
              Some (fun k -> continue k ())
          | Effects.Put_edge { from_; to_; label } ->
              st.edges := (from_, label, to_) :: !(st.edges);
              Some (fun k -> continue k ())
          | Effects.Delete_node id ->
              Hashtbl.remove st.nodes (Node_id.to_string id);
              st.edges :=
                List.filter
                  (fun (p, _l, c) ->
                    not (Node_id.equal p id) && not (Node_id.equal c id))
                  !(st.edges);
              Some (fun k -> continue k ())
          | _ -> None);
    }
