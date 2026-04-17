type t = {
  id       : Node_id.t;
  name     : string;
  parent   : Node_id.t option;
  created  : Ptime.t;
  metadata : Yojson.Safe.t;
  schema   : Schema.t option;
}

let make ~uuid ~level ~name ~parent ~created ~metadata ~schema =
  { id = Node_id.make level uuid; name; parent = Some parent; created; metadata; schema }

let make_root ~created =
  {
    id = Node_id.root;
    name = "root";
    parent = None;
    created;
    metadata = `Assoc [];
    schema = None;
  }

let level t = Node_id.level t.id
