type _ Effect.t +=
  | Get_node       : Node_id.t -> Node.t option Effect.t
  | List_children  : Node_id.t * Edge_kind.t option -> Node.t list Effect.t
  | List_child_refs : Node_id.t * Edge_kind.t option -> (Node_id.t * string) list Effect.t
  | Get_schema     : Node_id.t -> Schema.t option Effect.t

  | Put_node       : Node.t -> unit Effect.t
  | Put_edge       : {
      from_   : string;
      to_     : string;
      kind    : Edge_kind.t;
      name    : string;
      created : Ptime.t;
    } -> unit Effect.t
  | Delete_node    : Node_id.t -> unit Effect.t

  | Gen_uuid       : unit -> Uuidm.t Effect.t
  | Now            : unit -> Ptime.t Effect.t

type _ Effect.t +=
  | Put_sensor            : { sensor : Sensor.t; parent : Node_id.t } -> unit Effect.t
  | Get_active_sensor     : Sensor_id.t -> Sensor.t option Effect.t
  | List_sensor_ids       : Node_id.t -> Sensor_id.t list Effect.t
  | Replace_sensor_device : {
      old_created : Ptime.t;
      new_sensor : Sensor.t;
    } -> unit Effect.t
  | Delete_sensor         : { sensor_id : Sensor_id.t; parent : Node_id.t }
                              -> unit Effect.t
  | Get_sensor_reading    : Sensor_id.t -> float option Effect.t

let get_node id                 = Effect.perform (Get_node id)
let list_children ?kind parent = Effect.perform (List_children (parent, kind))
let list_child_refs ?kind parent =
  Effect.perform (List_child_refs (parent, kind))
let get_schema id               = Effect.perform (Get_schema id)
let put_node n                  = Effect.perform (Put_node n)
let put_edge ~from_ ~to_ ~kind ~name ~created =
  Effect.perform (Put_edge { from_; to_; kind; name; created })
let delete_node id              = Effect.perform (Delete_node id)
let gen_uuid ()                 = Effect.perform (Gen_uuid ())
let now ()                      = Effect.perform (Now ())

let put_sensor ~sensor ~parent =
  Effect.perform (Put_sensor { sensor; parent })

let get_active_sensor id =
  Effect.perform (Get_active_sensor id)

let list_sensor_ids parent =
  Effect.perform (List_sensor_ids parent)

let replace_sensor_device ~old_created ~new_sensor =
  Effect.perform (Replace_sensor_device { old_created; new_sensor })

let delete_sensor ~sensor_id ~parent =
  Effect.perform (Delete_sensor { sensor_id; parent })

let get_sensor_reading id =
  Effect.perform (Get_sensor_reading id)

type _ Effect.t +=
  | Put_user    : User.t -> unit Effect.t
  | Get_user    : User_id.t -> User.t option Effect.t
  | List_users  : unit -> User.t list Effect.t
  | Delete_user : User_id.t -> unit Effect.t

let put_user u     = Effect.perform (Put_user u)
let get_user id    = Effect.perform (Get_user id)
let list_users ()  = Effect.perform (List_users ())
let delete_user id = Effect.perform (Delete_user id)

type _ Effect.t +=
  | List_blocked_nodes : User_id.t -> Node_id.t list Effect.t
  | List_blocked_users : Node_id.t -> User_id.t list Effect.t
  | List_administrated_nodes : User_id.t -> Node_id.t list Effect.t
  | Delete_edge        : { from_ : string; to_ : string; kind : Edge_kind.t }
                          -> unit Effect.t

let list_blocked_nodes id = Effect.perform (List_blocked_nodes id)
let list_blocked_users id = Effect.perform (List_blocked_users id)
let list_administrated_nodes id =
  Effect.perform (List_administrated_nodes id)
let delete_edge ~from_ ~to_ ~kind =
  Effect.perform (Delete_edge { from_; to_; kind })
