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

let eval_identity () =
  let v =
    Formula.eval ~self:7.5 ~resolve:(fun _ -> failwith "should not call")
      Formula.Identity
  in
  Alcotest.(check (float 1e-9)) "identity returns self" 7.5 v

let eval_arithmetic () =
  let ast =
    Formula.Sub (Formula.Self, Formula.Add (Formula.Ref "S1", Formula.Ref "S2"))
  in
  let f = Formula.Expr { ast; refs = [ ("S1", uuid_a); ("S2", uuid_b) ] } in
  let resolve alias =
    match alias with
    | "S1" -> 3.0
    | "S2" -> 1.0
    | _ -> failwith "unknown alias"
  in
  let v = Formula.eval ~self:10.0 ~resolve f in
  Alcotest.(check (float 1e-9)) "10 - (3 + 1) = 6" 6.0 v

let eval_abs_flips_negative () =
  let ast = Formula.Abs (Formula.Sub (Formula.Self, Formula.Ref "S1")) in
  let f = Formula.Expr { ast; refs = [ ("S1", uuid_a) ] } in
  let resolve _ = 12.0 in
  let v = Formula.eval ~self:5.0 ~resolve f in
  Alcotest.(check (float 1e-9)) "|5 - 12| = 7" 7.0 v

let eval_multiplier () =
  let ast = Formula.Mul (Formula.Self, Formula.Num 2.5) in
  let f = Formula.Expr { ast; refs = [] } in
  let v = Formula.eval ~self:4.0 ~resolve:(fun _ -> 0.0) f in
  Alcotest.(check (float 1e-9)) "4 * 2.5 = 10" 10.0 v

let eval_div_by_zero_is_infinity () =
  let ast = Formula.Div (Formula.Self, Formula.Num 0.0) in
  let f = Formula.Expr { ast; refs = [] } in
  let v = Formula.eval ~self:1.0 ~resolve:(fun _ -> 0.0) f in
  Alcotest.(check bool) "infinite" true (Float.is_infinite v)

let eval_unknown_ref_raises () =
  let ast = Formula.Ref "missing" in
  let f = Formula.Expr { ast; refs = [] } in
  (try
     let _ = Formula.eval ~self:0.0 ~resolve:(fun _ -> 0.0) f in
     Alcotest.fail "expected exception"
   with Formula.Unknown_ref "missing" -> ())

let collect_refs_identity_empty () =
  let xs = Formula.referenced_uuids Formula.Identity in
  Alcotest.(check int) "none" 0 (List.length xs)

let eval_zero_returns_zero () =
  let v =
    Formula.eval ~self:123.0 ~resolve:(fun _ -> failwith "should not call")
      Formula.Zero
  in
  Alcotest.(check (float 0.0)) "0" 0.0 v

let referenced_uuids_zero_empty () =
  let xs = Formula.referenced_uuids Formula.Zero in
  Alcotest.(check int) "none" 0 (List.length xs)

let collect_refs_of_expr () =
  let ast = Formula.Sub (Formula.Ref "S1", Formula.Ref "S2") in
  let f = Formula.Expr { ast; refs = [ ("S1", uuid_a); ("S2", uuid_b) ] } in
  let xs = Formula.referenced_uuids f |> List.sort Uuidm.compare in
  let expected = List.sort Uuidm.compare [ uuid_a; uuid_b ] in
  Alcotest.(check int) "two" 2 (List.length xs);
  Alcotest.(check bool) "set equal" true
    (List.for_all2 Uuidm.equal xs expected)

let tests =
  [
    Alcotest.test_case "Identity constructs"    `Quick identity_constructs;
    Alcotest.test_case "Expr carries refs"      `Quick expr_has_refs;
    Alcotest.test_case "deeply nested Abs/Sub"  `Quick deeply_nested_expr;
    Alcotest.test_case "eval Identity = self"        `Quick eval_identity;
    Alcotest.test_case "eval arithmetic"             `Quick eval_arithmetic;
    Alcotest.test_case "eval Abs flips negative"     `Quick eval_abs_flips_negative;
    Alcotest.test_case "eval multiplier"             `Quick eval_multiplier;
    Alcotest.test_case "eval div by zero = infinity" `Quick eval_div_by_zero_is_infinity;
    Alcotest.test_case "eval unknown ref raises"     `Quick eval_unknown_ref_raises;
    Alcotest.test_case "referenced_uuids identity" `Quick collect_refs_identity_empty;
    Alcotest.test_case "referenced_uuids expr"     `Quick collect_refs_of_expr;
    Alcotest.test_case "eval Zero = 0"             `Quick eval_zero_returns_zero;
    Alcotest.test_case "referenced_uuids zero"     `Quick referenced_uuids_zero_empty;
  ]
