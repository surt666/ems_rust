Feature: Flink/Glue output parity
  The streaming path (Flink ResampleFunction) and the batch late-recomputation path
  (Glue late_recomputation.py) both write logical_meter_data. Because late-arriving rows
  reprocessed by Glue replace Flink's earlier writes (newest ingested_time wins per source
  reading + bin), the two paths MUST produce identical output rows for the same input — any
  divergence makes consumers see values oscillate whenever a backfill runs.

  Background:
    Given the same (raw reading pair, meter mapping) is processed by both Flink and Glue
    And logical_meter_data is append-only with restatements distinguished by ingested_time

  # source: resampling-rules-design spec — "Output parity invariant"
  Scenario: Both paths emit bit-identical rows for the same input
    Given a counter reading pair with a known delta and bin overlap
    When Flink resamples it and Glue resamples it
    Then they emit the same resample_timestamp, resample_value and resample_method
    And the same per-bin row layout (timestamp, value=delta, resample_*)

  # source: resampling-rules-design spec — shared bin enumeration & overlap formula
  Scenario: Both paths share bin enumeration and the overlap formula
    Given a counter period that straddles a bin boundary
    When each path enumerates overlapping bins and splits the delta
    Then both use overlap = min(currentTs, B) - max(prevTs, B - binSize)
    And both attribute the delta by overlap / (currentTs - prevTs)

  # source: resampling-rules-design spec — unit normalization parity
  Scenario: Both paths apply the same unit normalization to value and resample_value
    Given a raw unit such as "Energy (100 Wh)"
    When each path normalizes the unit
    Then value and resample_value are scaled by the same factor in both paths
    And the unit column holds the same canonical unit name

  # source: resampling-rules-design spec — consumer query / dedup
  Scenario: Consumers dedup by (logical_id, timestamp, resample_timestamp) then sum
    Given multiple restatements of the same source reading in logical_meter_data
    When a consumer reads resampled values
    Then for each (logical_id, timestamp, resample_timestamp) only the newest ingested_time row is kept
    And resample_value is summed per (logical_id, resample_timestamp) after that dedup

  # source: resampling-rules-design spec — single-reading meter behaviour
  Scenario: A meter with only one reading produces no bin rows in either path
    Given a logical meter with exactly one reading in the dataset
    When Flink and Glue process it
    Then neither path emits any bin rows for it (no predecessor to bracket)

  # Guardrail, not a runtime assertion:
  # source: resampling-rules-design spec — "Any change to one side must be mirrored in the other in the same commit."
  Scenario: A change to one resampling path requires the mirror change in the same commit
    Given a change to bin enumeration, the overlap formula, the unit table, or the row layout on one side
    When the change is proposed
    Then the equivalent change must land on the other side in the same commit
