let () =
  Alcotest.run "ocaml_lambda_test"
    [
      ("domain.level", Test_domain_level.tests);
      ("domain.node_id", Test_domain_node_id.tests);
    ]
