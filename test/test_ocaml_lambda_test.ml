let () =
  Alcotest.run "ocaml_lambda_test"
    [
      ("domain.level", Test_domain_level.tests);
      ("domain.node_id", Test_domain_node_id.tests);
      ("domain.metadata", Test_domain_metadata.tests);
      ("domain.schema", Test_domain_schema.tests);
    ]
