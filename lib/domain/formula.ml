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

exception Unknown_ref of string

let rec eval_expr ~self ~resolve = function
  | Num n          -> n
  | Self           -> self
  | Ref alias      -> resolve alias
  | Abs e          -> Float.abs (eval_expr ~self ~resolve e)
  | Add (a, b)     -> eval_expr ~self ~resolve a +. eval_expr ~self ~resolve b
  | Sub (a, b)     -> eval_expr ~self ~resolve a -. eval_expr ~self ~resolve b
  | Mul (a, b)     -> eval_expr ~self ~resolve a *. eval_expr ~self ~resolve b
  | Div (a, b)     -> eval_expr ~self ~resolve a /. eval_expr ~self ~resolve b

let eval ~self ~resolve = function
  | Identity -> self
  | Expr { ast; refs } ->
      let lookup alias =
        match List.assoc_opt alias refs with
        | Some _ -> resolve alias
        | None -> raise (Unknown_ref alias)
      in
      eval_expr ~self ~resolve:lookup ast

let referenced_uuids = function
  | Identity -> []
  | Expr { refs; _ } -> List.map snd refs
