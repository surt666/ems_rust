type _ Effect.t +=
  | Get_node       : Node_id.t -> Node.t option Effect.t
  | List_children  : Node_id.t * string option -> Node.t list Effect.t
  | Get_schema     : Node_id.t -> Schema.t option Effect.t

  | Put_node       : Node.t -> unit Effect.t
  | Put_edge       : { from_ : Node_id.t; to_ : Node_id.t; label : string } -> unit Effect.t
  | Delete_node    : Node_id.t -> unit Effect.t

  | Gen_uuid       : unit -> Uuidm.t Effect.t
  | Now            : unit -> Ptime.t Effect.t

let get_node id                 = Effect.perform (Get_node id)
let list_children ?label parent = Effect.perform (List_children (parent, label))
let get_schema id               = Effect.perform (Get_schema id)
let put_node n                  = Effect.perform (Put_node n)
let put_edge ~from_ ~to_ ~label = Effect.perform (Put_edge { from_; to_; label })
let delete_node id              = Effect.perform (Delete_node id)
let gen_uuid ()                 = Effect.perform (Gen_uuid ())
let now ()                      = Effect.perform (Now ())
