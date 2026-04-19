let () =
  Alcotest.run "ocaml_lambda_test"
    [
      ("domain.level", Test_domain_level.tests);
      ("domain.node_id", Test_domain_node_id.tests);
      ("domain.metadata", Test_domain_metadata.tests);
      ("domain.schema", Test_domain_schema.tests);
      ("domain.node", Test_domain_node.tests);
      ("domain.formula", Test_domain_formula.tests);
      ("domain.edge_kind", Test_domain_edge_kind.tests);
      ("domain.user", Test_domain_user.tests);
      ("domain.sensor_id", Test_domain_sensor_id.tests);
      ("domain.sensor_sk", Test_domain_sensor_sk.tests);
      ("domain.sensor", Test_domain_sensor.tests);
      ("repo.memory", Test_repo_memory.tests);
      ("logic.schema_check", Test_logic_schema_check.tests);
      ("logic.hierarchy", Test_logic_hierarchy.tests);
      ("logic.properties", Test_logic_properties.tests);
      ("logic.sensors", Test_logic_sensors.tests);
      ("logic.users", Test_logic_users.tests);
      ("api.json", Test_api_json.tests);
      ("api.query", Test_api_query.tests);
      ("api.command", Test_api_command.tests);
      ("repo.codec", Test_repo_codec.tests);
    ]
