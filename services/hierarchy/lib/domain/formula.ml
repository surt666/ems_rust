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

(* Distinct alias names referenced by an expression, in first-seen order. *)
let expr_aliases (e : expr) : string list =
  let rec go acc = function
    | Num _ | Self -> acc
    | Ref a -> if List.mem a acc then acc else a :: acc
    | Abs sub -> go acc sub
    | Add (a, b) | Sub (a, b) | Mul (a, b) | Div (a, b) -> go (go acc a) b
  in
  List.rev (go [] e)

(* Render an expression back to source text with minimal parentheses.
   Precedence: + - = 1, * / = 2; all binops left-associative. *)
let expr_to_string (e : expr) : string =
  let wrap outer p s = if outer > p then "(" ^ s ^ ")" else s in
  let rec go prec = function
    | Num n -> Printf.sprintf "%.17g" n
    | Self -> "self"
    | Ref a -> a
    | Abs e -> "abs(" ^ go 0 e ^ ")"
    | Add (a, b) -> wrap prec 1 (go 1 a ^ " + " ^ go 2 b)
    | Sub (a, b) -> wrap prec 1 (go 1 a ^ " - " ^ go 2 b)
    | Mul (a, b) -> wrap prec 2 (go 2 a ^ " * " ^ go 3 b)
    | Div (a, b) -> wrap prec 2 (go 2 a ^ " / " ^ go 3 b)
  in
  go 0 e

let to_string (f : t) : string =
  match f with
  | Identity -> "identity"
  | Zero -> "zero"
  | Expr { ast; _ } -> expr_to_string ast
