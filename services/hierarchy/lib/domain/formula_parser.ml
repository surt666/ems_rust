(* Recursive-descent parser for sensor formula expressions.
     expr    := term (('+' | '-') term)*
     term    := factor (('*' | '/') factor)*
     factor  := '-' factor | primary
     primary := number | 'self' | ident | 'abs' '(' expr ')' | '(' expr ')'
   `self` and `abs` are reserved; any other identifier becomes a Ref alias. *)

let ( let* ) = Result.bind

type state = { src : string; mutable pos : int }

let peek st = if st.pos < String.length st.src then Some st.src.[st.pos] else None
let advance st = st.pos <- st.pos + 1

let rec skip_ws st =
  match peek st with
  | Some (' ' | '\t' | '\n' | '\r') -> advance st; skip_ws st
  | _ -> ()

let is_digit c = c >= '0' && c <= '9'
let is_ident_start c = (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || c = '_'
let is_ident_char c = is_ident_start c || is_digit c

let parse_number st =
  let start = st.pos in
  let rec go () =
    match peek st with
    | Some c when is_digit c || c = '.' || c = 'e' || c = 'E' -> advance st; go ()
    | Some ('+' | '-') when st.pos > start
                            && (let p = st.src.[st.pos - 1] in p = 'e' || p = 'E') ->
        advance st; go ()
    | _ -> ()
  in
  go ();
  let tok = String.sub st.src start (st.pos - start) in
  match float_of_string_opt tok with
  | Some f -> Ok (Formula.Num f)
  | None -> Error (Printf.sprintf "invalid number %S" tok)

let parse_ident st =
  let start = st.pos in
  let rec go () =
    match peek st with
    | Some c when is_ident_char c -> advance st; go ()
    | _ -> ()
  in
  go ();
  String.sub st.src start (st.pos - start)

let rec parse_expr st =
  let* left = parse_term st in
  parse_expr_tail st left
and parse_expr_tail st left =
  skip_ws st;
  match peek st with
  | Some '+' -> advance st;
      let* r = parse_term st in parse_expr_tail st (Formula.Add (left, r))
  | Some '-' -> advance st;
      let* r = parse_term st in parse_expr_tail st (Formula.Sub (left, r))
  | _ -> Ok left
and parse_term st =
  let* left = parse_factor st in
  parse_term_tail st left
and parse_term_tail st left =
  skip_ws st;
  match peek st with
  | Some '*' -> advance st;
      let* r = parse_factor st in parse_term_tail st (Formula.Mul (left, r))
  | Some '/' -> advance st;
      let* r = parse_factor st in parse_term_tail st (Formula.Div (left, r))
  | _ -> Ok left
and parse_factor st =
  skip_ws st;
  match peek st with
  | Some '-' -> advance st;
      let* e = parse_factor st in Ok (Formula.Sub (Formula.Num 0., e))
  | _ -> parse_primary st
and parse_primary st =
  skip_ws st;
  match peek st with
  | None -> Error "unexpected end of expression"
  | Some '(' ->
      advance st;
      let* e = parse_expr st in
      skip_ws st;
      (match peek st with
       | Some ')' -> advance st; Ok e
       | _ -> Error "expected ')'")
  | Some c when is_digit c || c = '.' -> parse_number st
  | Some c when is_ident_start c ->
      (match parse_ident st with
       | "self" -> Ok Formula.Self
       | "abs" ->
           skip_ws st;
           (match peek st with
            | Some '(' ->
                advance st;
                let* e = parse_expr st in
                skip_ws st;
                (match peek st with
                 | Some ')' -> advance st; Ok (Formula.Abs e)
                 | _ -> Error "expected ')' after abs(")
            | _ -> Error "expected '(' after abs")
       | name -> Ok (Formula.Ref name))
  | Some c -> Error (Printf.sprintf "unexpected character %C" c)

let parse (s : string) : (Formula.expr, string) result =
  let st = { src = s; pos = 0 } in
  let* e = parse_expr st in
  skip_ws st;
  if st.pos < String.length st.src then
    Error (Printf.sprintf "unexpected trailing input near position %d" st.pos)
  else Ok e
