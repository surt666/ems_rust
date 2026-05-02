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
  | Zero
  | Expr of { ast : expr; refs : (string * Sensor_id.t) list }

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
  | Zero -> 0.
  | Expr { ast; refs } ->
      let lookup alias =
        match List.assoc_opt alias refs with
        | Some _ -> resolve alias
        | None -> raise (Unknown_ref alias)
      in
      eval_expr ~self ~resolve:lookup ast

let referenced_ids = function
  | Identity | Zero -> []
  | Expr { refs; _ } -> List.map snd refs
