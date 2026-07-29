Feature: measurements_aggregate rollup view
  An hourly Glue job rolls up counter consumption from logical_data into a DynamoDB
  "materialized view" pre-aggregated at every hierarchy level from the company (hn2) down to
  the leaf meter, for both hourly and daily buckets. Items are keyed so that a single range
  query returns exactly one node's own series and never its descendants. The job is idempotent:
  re-running a window overwrites the same items and never double-counts.

  Background:
    Given the source rows carry hn2..hn9, logical_id, energy_type, unit, value, timestamp, reading_kind and ingested_time
    And the aggregated quantity is sum(value), counters only (reading_kind = "counter")
    And pk is "HN2#<hn2>" so each company is its own partition

  # source: glue/tests/test_helpers.py — hour/day bucket UTC
  Scenario Outline: Buckets are derived in UTC
    Given a timestamp "<ts>"
    Then its hour bucket is "<hour>" and its day bucket is "<day>"

    Examples:
      | ts                        | hour          | day        |
      | 2026-06-07T08:45:00+00:00 | 2026-06-07T08 | 2026-06-07 |

  # source: glue/tests/test_helpers.py — ancestor_keys
  Scenario Outline: A reading explodes into one node key per populated level plus its leaf
    Given hn nodes "<nodes>" for logical_id <leaf>
    When ancestor keys are built
    Then the keys are "<keys>"

    Examples:
      | nodes              | leaf  | keys                                                                                  |
      | 2,9,456            | 10009 | HN2#2 ; HN2#2\|HN3#9 ; HN2#2\|HN3#9\|HN4#456 ; HN2#2\|HN3#9\|HN4#456\|L#10009          |
      | 2                  | 10009 | HN2#2 ; HN2#2\|L#10009                                                                 |

  # INVARIANT (mirrors meter_enrichment.feature): v2-created paths are dense (child level =
  # parent + 1), so ancestor_keys stops at the first None — see
  # features/hierarchy/schema_type_graph.feature.
  # KNOWN LIMITATION (accepted 2026-06-10): legacy v1 level-skip nodes exist (456 buildings at
  # HN4 directly under HN2 companies, hole at hn3). A meter under such a node has hn3=null,
  # hn4 set, and ancestor_keys drops its building/deeper levels from the rollup. Accepted
  # because only one building carries a sensor; if meters ever attach under legacy skip-path
  # buildings, change the break to skip interior holes (join to nearest populated ancestor).

  # source: glue/tests/test_helpers.py — build_sk and delimiter invariant
  Scenario: A node's own dated rows sort before any descendant row
    Given a node path "HN2#2|HN3#9|HN4#456" and its child leaf "HN2#2|HN3#9|HN4#456|L#10009"
    When their sort keys are built for the same purpose, granularity and bucket
    Then the node's own sk is strictly less than the child's sk
    And the sk format is "<path>#<purpose>#<gran>#<bucket>"
    # because '#' (0x23) < '|' (0x7C), a BETWEEN range on "<path>#<purpose>#<gran>#" returns
    # only the node's own series, never its children

  # source: glue/tests/test_rollups.py — "test_rollup_sums_at_every_level"
  Scenario: A node's sum is the total across all meters beneath it
    Given two electricity readings on meter 10009 (values 4.0 and 6.0) under HN2#2|HN3#9|HN4#456
    And one electricity reading on meter 10010 (value 5.0) under HN2#2|HN3#9
    When the hourly rollup for 2026-06-07T08 is built
    Then the leaf 10009 sums to 10.0 with count 2, last_value 106.0 and unit "kWh"
    And HN2#2|HN3#9|HN4#456 sums to 10.0
    And HN2#2|HN3#9 sums to 15.0
    And HN2#2 sums to 15.0
    And the day bucket 2026-06-07 for HN2#2 also sums to 15.0

  # source: glue/tests/test_rollups.py — "test_latest_counters_dedup_and_filters"
  Scenario: The newest ingested_time wins per point, and gauges / null-company rows are excluded
    Given two restatements of point (10009, rt) with value 4.0 (older) and 9.0 (newer ingested_time)
    And a gauge row (reading_kind "gauge")
    And a counter row with a null hn2
    When latest_counters runs
    Then only one row survives for that point, with value 9.0
    And the gauge row and the null-hn2 row are dropped

  # source: glue/tests/test_rollups.py — "test_rollup_is_idempotent"
  Scenario: Re-running a window produces identical items (idempotent upsert)
    Given the same input aggregated at two different run times
    When build_rollups runs each time
    Then both runs produce the same set of sks with identical sum and count

  # source: glue/tests/test_helpers.py / test_rollups.py — ttl_for, pk
  Scenario Outline: TTL is the bucket end plus a retention offset per granularity
    Given a "<gran>" bucket "<bucket>"
    Then its ttl equals the bucket-end epoch plus <days> days

    Examples:
      | gran | bucket        | days |
      | h    | 2026-06-07T08 | 90   |
      | d    | 2026-06-07    | 730  |

  # source: glue/tests/test_helpers.py — window_start_is_day_aligned_utc
  Scenario Outline: The recompute window start is day-aligned UTC, N whole days back
    Given the job runs at "2026-06-07T09:30:00+00:00" with lookback <N>
    Then the window start is "<start>"

    Examples:
      | N | start                     |
      | 1 | 2026-06-06T00:00:00+00:00 |
      | 0 | 2026-06-07T00:00:00+00:00 |
