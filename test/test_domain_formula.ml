open Ocaml_lambda_test

let uuid_a = Uuidm.of_string "00000000-0000-4000-8000-000000000001" |> Option.get
let uuid_b = Uuidm.of_string "00000000-0000-4000-8000-000000000002" |> Option.get

let identity_constructs () =
  let f = Formula.Identity in
  Alcotest.(check bool) "is identity"
    true
    (match f with Formula.Identity -> true | _ -> false)

let expr_has_refs () =
  let ast = Formula.Abs (Formula.Sub (Formula.Self, Formula.Ref "S1")) in
  let f = Formula.Expr { ast; refs = [ ("S1", uuid_a) ] } in
  match f with
  | Formula.Expr { refs; _ } ->
      Alcotest.(check int) "one ref" 1 (List.length refs);
      let alias, u = List.hd refs in
      Alcotest.(check string) "alias" "S1" alias;
      Alcotest.(check bool) "uuid equal" true (Uuidm.equal u uuid_a)
  | _ -> Alcotest.fail "expected Expr"

let deeply_nested_expr () =
  let ast =
    Formula.Abs
      (Formula.Sub
         (Formula.Sub (Formula.Self, Formula.Ref "S4"),
          Formula.Ref "S5"))
  in
  let _ = Formula.Expr { ast; refs = [ ("S4", uuid_a); ("S5", uuid_b) ] } in
  Alcotest.(check pass) "compiles" () ()

let tests =
  [
    Alcotest.test_case "Identity constructs"    `Quick identity_constructs;
    Alcotest.test_case "Expr carries refs"      `Quick expr_has_refs;
    Alcotest.test_case "deeply nested Abs/Sub"  `Quick deeply_nested_expr;
  ]
