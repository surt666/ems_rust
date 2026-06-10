Feature: Meter enrichment and hierarchy resolution
  Each valid SensorRecord is enriched with logical meter identity and hierarchy context by
  looking up its daq_id in the broadcast meter-identity state. Enrichment attaches the logical
  id, the hierarchy node ids (hn1..hn9) and purpose, parses the value to a number, and leaves
  the raw timestamp and unit untouched. Records with no mapping are dead-lettered.

  Background:
    Given the broadcast state holds meter mappings keyed by daq_id
    And a mapping carries logicalId, meterType, hn1..hn9, purpose and optional resampleMinutes

  # source: MeterEnrichmentFunctionSpec — "produce EnrichedRecord"
  Scenario: A mapped record is enriched with identity and hierarchy
    Given a SensorRecord "daq:std_json_v1:cust:meter1:temp" with value "21.5" unit "C"
    And a mapping with logicalId 101, hn1 1, hn2 2, hn4 8, hn5 3, purpose "supply temp"
    When MeterEnrichmentFunction.enrich runs
    Then the EnrichedRecord has logicalId 101 and value 21.5
    And it carries hn1 1, hn2 2, hn4 8, hn5 3 and hn3 null
    And its purpose is "supply temp"

  # source: MeterEnrichmentFunctionSpec — "parse string value to double"
  Scenario: The string value is parsed to a double during enrichment
    Given a SensorRecord whose value is the string "123.456"
    When it is enriched
    Then the EnrichedRecord value is the number 123.456

  # source: MeterEnrichmentFunctionSpec — "not normalize units", "leave bin_* fields null"
  Scenario: Enrichment does not normalize units, resample, or floor the timestamp
    Given a SensorRecord with unit "kWh", value "5.0", timestamp "2026-03-27T10:07:23Z"
    And a mapping with resampleMinutes 15
    When it is enriched
    Then the unit stays "kWh" and the value stays 5.0
    And the timestamp stays "2026-03-27T10:07:23Z" regardless of resampleMinutes
    And resampleTimestamp, resampleValue and resampleMethod are all null at this stage

  # source: MeterEnrichmentFunctionSpec — "preserve all hierarchy levels"
  Scenario: All populated hierarchy levels survive enrichment; absent levels stay null
    Given a mapping populating hn1, hn2, hn3, hn4, hn6 and hn9
    When a record is enriched
    Then exactly those levels are present on the EnrichedRecord and the rest are null

  # source: meter-enrichment-design spec — DEAD_LETTER side output
  Scenario: A record with no mapping is dead-lettered, not enriched
    Given a valid SensorRecord whose daq_id is absent from the broadcast state
    When the enrichment function processes it
    Then it is routed to the DEAD_LETTER side output
    And no EnrichedRecord is produced for it

  # source: HierarchyPathParserSpec — "skip non-contiguous depths if absent"
  # INVARIANT: hn1..hn9 are dense DEPTH indices. Under the v2 type-graph schema
  # (see features/hierarchy/schema_type_graph.feature) a child's level is always
  # the parent's depth + 1, derived by the model layer — so a v2-created
  # hierarchy_path is contiguous from hn2 with only trailing nulls. The same
  # TYPE (e.g. building) may appear at different depths; never assume a fixed
  # type-per-level mapping (hn4 is not always "building").
  # LEGACY EXCEPTION (found 2026-06-10 during the v2 migration): the v1 schema
  # validator allowed level-SKIPPING edges, and production hierarchy_new holds
  # 456 of them (buildings at HN4 directly under HN2 companies — hole at hn3).
  # Those nodes keep their ids, so their meters' hierarchy_paths have interior
  # nulls. Consumers walking levels must tolerate such holes for legacy nodes
  # even though v2 never creates new ones.
  Scenario: The parser tolerates a non-contiguous path; legacy nodes can produce one
    Given the hierarchy_path "HN0#root|HN1#1|HN2#2|HN4#8" (a v1 level-skip edge shape)
    When HierarchyPathParser.parse runs
    Then it still parses, yielding hn3 null and hn4 8
    And v2 never creates new holes, but legacy nodes with this shape exist in production

  # source: HierarchyPathParserSpec — valid paths
  Scenario Outline: Hierarchy paths parse into per-level node ids
    Given the hierarchy_path "<path>"
    When HierarchyPathParser.parse runs
    Then hn1 is <hn1> and hn2 is <hn2> and hn3 is <hn3> and hn4 is <hn4>

    Examples:
      | path                                    | hn1 | hn2 | hn3  | hn4  |
      | HN0#root\|HN1#1\|HN2#2                  | 1   | 2   | null | null |
      | HN0#root\|HN1#1\|HN2#2\|HN3#5\|HN4#8    | 1   | 2   | 5    | 8    |
      | HN0#root\|HN1#1\|HN2#2\|HN4#8           | 1   | 2   | null | 8    |
      | HN1#10\|HN2#20\|HN3#30                  | 10  | 20  | 30   | null |

  # source: HierarchyPathParserSpec — "fill all hn3..hn9 levels"
  Scenario: A full-depth path fills hn1 through hn9
    Given the path "HN0#root|HN1#1|HN2#2|HN3#3|HN4#4|HN5#5|HN6#6|HN7#7|HN8#8|HN9#9"
    When it is parsed
    Then hn1..hn9 are 1..9 respectively

  # source: HierarchyPathParserSpec — throw cases
  Scenario Outline: Partner and company are mandatory and segments must be recognised
    Given the hierarchy_path "<path>"
    When HierarchyPathParser.parse runs
    Then it raises IllegalArgumentException

    Examples:
      | path                       |
      | HN0#root\|HN2#2            |
      | HN0#root\|HN1#1            |
      |                           |
      | HN0#root\|HN1#1\|HN2#2\|XX#9 |
