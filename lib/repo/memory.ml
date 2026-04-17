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
          | Effects.List_children _ | Effects.Get_schema _
          | Effects.Put_node _ | Effects.Put_edge _ | Effects.Delete_node _ ->
              Some (fun _k -> failwith "Memory: handler not yet complete")
          | _ -> None);
    }
