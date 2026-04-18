type expr =
  | Num of float
  | Self
  | Ref of string
  | Abs of expr
  | Add of expr * expr
  | Sub of expr * expr
  | Mul of expr * expr
  | Div of expr * expr

type t =
  | Identity
  | Expr of { ast : expr; refs : (string * Uuidm.t) list }
