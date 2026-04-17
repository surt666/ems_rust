type edge_spec = { label : string; min : int option; max : int option }

type t = {
  version : int;
  edges : (Level.t * (Level.t * edge_spec list) list) list;
  metadata : (Level.t * (string * Metadata.field_spec) list) list;
}

let allowed_children t parent =
  match List.assoc_opt parent t.edges with
  | Some cs -> cs
  | None -> []

let edges_between t parent child =
  match List.assoc_opt child (allowed_children t parent) with
  | Some specs -> specs
  | None -> []

let metadata_for t level =
  match List.assoc_opt level t.metadata with
  | Some fs -> fs
  | None -> []

let validate t =
  let exception Bad of string in
  try
    List.iter
      (fun (parent, children) ->
        List.iter
          (fun (child, specs) ->
            if Level.depth parent >= Level.depth child then
              raise (Bad (Printf.sprintf "edge %s -> %s violates depth ordering"
                            (Level.to_string parent) (Level.to_string child)));
            let seen = Hashtbl.create 4 in
            List.iter
              (fun spec ->
                if Hashtbl.mem seen spec.label then
                  raise (Bad (Printf.sprintf "edge %s -> %s: duplicate label %S"
                                (Level.to_string parent) (Level.to_string child) spec.label));
                Hashtbl.add seen spec.label ();
                match spec.min, spec.max with
                | Some a, Some b when a > b ->
                    raise (Bad (Printf.sprintf "edge %s -> %s (%s) has min > max"
                                  (Level.to_string parent) (Level.to_string child) spec.label))
                | _ -> ())
              specs)
          children)
      t.edges;
    List.iter
      (fun (level, fields) ->
        List.iter
          (fun (name, spec) ->
            match Metadata.validate_spec spec with
            | Ok () -> ()
            | Error msg ->
                raise (Bad (Printf.sprintf "%s.%s: %s" (Level.to_string level) name msg)))
          fields)
      t.metadata;
    Ok ()
  with Bad msg -> Error msg
