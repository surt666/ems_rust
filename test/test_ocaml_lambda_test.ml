let () =
  Alcotest.run "ocaml_lambda_test"
    [
      ("domain.level", Test_domain_level.tests);
      ("domain.node_id", Test_domain_node_id.tests);
      ("domain.metadata", Test_domain_metadata.tests);
      ("domain.schema", Test_domain_schema.tests);
      ("domain.node", Test_domain_node.tests);
      ("repo.memory", Test_repo_memory.tests);
      ("logic.schema_check", Test_logic_schema_check.tests);
      ("logic.hierarchy", Test_logic_hierarchy.tests);
      ("logic.properties", Test_logic_properties.tests);
      ("api.json", Test_api_json.tests);
      ("api.query", Test_api_query.tests);
    ]
