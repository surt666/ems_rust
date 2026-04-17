let () =
  Alcotest.run "ocaml_lambda_test"
    [
      ("domain.level", Test_domain_level.tests);
    ]
