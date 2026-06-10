Feature: Company schema as a type graph (v2)
  A company (hn2) schema declares node TYPES and a DAG of allowed containment
  between them, rooted at the reserved type "company". A node's level (hnN) is
  purely its depth — always parent + 1 — so the same type may appear at
  different depths and a node's rules follow its type, not its level.

  Background:
    Given a company schema: company → {group, property, building},
      group → {building}, property → {building}, building → {area}
    And metadata lat/lng required on type "building"
    And sensors allowed on types "building" and "area"

  # source: crates/model hierarchy.rs — building_at_variable_depth
  Scenario: The same type is valid at different depths with the same rules
    When a building is created directly under the company
    Then it sits at hn3 with label "building" and requires lat/lng metadata
    When a building is created under a group
    Then it sits at hn4 with label "building" and requires lat/lng metadata
    And both buildings may only contain "area" children

  # source: crates/model hierarchy.rs — add_under_schema
  Scenario: Containment follows the parent's type, not its depth
    Given a group node and a building node both at hn3
    When an "area" child is requested under each
    Then it is rejected under the group (group allows only building)
    And it is accepted under the building

  # source: crates/model hierarchy.rs — level derivation
  Scenario: A child's level is always the parent's depth plus one
    When any node is created
    Then its level is parent.depth + 1, derived — never chosen by the caller
    And an explicit level parameter that disagrees is rejected
    # paths are therefore always dense/contiguous — the invariant the DAQ
    # rollup's ancestor_keys depends on

  # source: crates/model schema.rs — Schema::validate
  Scenario Outline: Schema validation rejects malformed type graphs
    Given a schema with <defect>
    When it is validated on company creation
    Then it is rejected

    Examples:
      | defect                                              |
      | a cycle between types                               |
      | a self-edge (type contains itself)                  |
      | a type unreachable from "company"                   |
      | an edge targeting the reserved type "company"       |
      | the type "partner" appearing anywhere               |
      | a chain longer than 7 edges from "company" (> hn9)  |
      | duplicate child types under one parent              |
      | min greater than max on an edge                     |

  # source: crates/model sensors.rs — allows_sensors by type
  Scenario: Sensor placement follows node type at any depth
    Given sensors are allowed on type "building"
    Then a sensor attaches to a building at hn3 and to a building at hn4
    And a sensor on a "group" node is rejected

  # source: crates/model codec.rs / json.rs — clean break
  Scenario: Version-1 schemas are rejected loudly, never misread
    Given a stored schema with version 1
    When it is read
    Then the error names the version and points at scripts/migrate_schema_v2.py
