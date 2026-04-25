type t = {
  id       : Node_id.t;
  name     : string;
  parent   : Node_id.t option;
  (* Pipe-separated list of ancestor node-ids from root down to and including
     self. Root has path = "HN0#root". HN1 has "HN0#root|HN1#<uuid>". HN2 has
     "HN0#root|HN1#<uuid>|HN2#<uuid>". Every node must have path populated. *)
  path     : string;
  created  : Ptime.t;
  metadata : Yojson.Safe.t;
  schema   : Schema.t option;
}

let path_sep = "|"

(* child_path p c = path of [c] given parent's full path and child id. *)
let child_path ~parent_path ~child_id_str = parent_path ^ path_sep ^ child_id_str

let make ~uuid ~level ~name ~parent ~parent_path ~created ~metadata ~schema =
  let id = Node_id.make level uuid in
  let path = child_path ~parent_path ~child_id_str:(Node_id.to_string id) in
  { id; name; parent = Some parent; path; created; metadata; schema }

let make_root ~created =
  {
    id = Node_id.root;
    name = "root";
    parent = None;
    path = Node_id.to_string Node_id.root;
    created;
    metadata = `Assoc [];
    schema = None;
  }

let level t = Node_id.level t.id

(* Extract the node-id segment whose level matches [lvl] from a path string.
   Returns None if no segment exists at that level. *)
let segment_at_level ~path ~lvl =
  let parts = String.split_on_char '|' path |> List.filter (fun s -> s <> "") in
  let prefix = Printf.sprintf "HN%d#" (Level.depth lvl) in
  let plen = String.length prefix in
  List.find_map
    (fun seg ->
      if String.length seg >= plen && String.sub seg 0 plen = prefix
      then Some seg else None)
    parts
