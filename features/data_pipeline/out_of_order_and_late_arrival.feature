Feature: Out-of-order handling and late arrivals
  Readings can arrive out of event-time order. The ResampleFunction buffers readings in keyed
  state and computes deltas/bins against the true event-time predecessor, so results are
  independent of arrival order. Once a reading's predecessor has been purged from the buffer
  (the configured retention window has passed) the reading cannot be resampled in-stream and is
  routed to LATE_ARRIVAL for the Glue recomputation path to handle.

  Background:
    Given the ResampleFunction keeps a per-logical-meter event-time buffer
    And it resamples each reading against its true event-time predecessor

  # source: ResampleHarnessSpec — "buffer the first ... reading and emit nothing"
  Scenario: The first reading for a meter is buffered and emits nothing
    Given no prior reading for a meter
    When the first reading arrives
    Then nothing is emitted and the reading is held as the baseline

  # source: EnrichmentPipelineSpec — "produce same deltas regardless of arrival order"
  Scenario: Deltas are identical regardless of arrival order
    Given a fixed set of counter readings
    When they are fed in order, reversed, and shuffled
    Then the resulting per-period deltas and timestamps are identical across all orderings

  # source: EnrichmentPipelineSpec — "produce correct deltas when records arrive out of order"
  Scenario: An out-of-order reading is reconciled against its true neighbours
    Given counter readings that arrive out of event-time order within the buffer window
    When they are processed
    Then each delta is computed against the correct event-time predecessor
    And the deltas sum to (last cumulative - first cumulative)

  # source: EnrichmentMiniClusterSpec — "Counter out-of-order"
  Scenario: A late-but-buffered reading produces corrected deltas after the watermark
    Given counter readings 100 @10:00, 130 @10:30 then 115 @10:15 arriving late but within retention
    When the watermark advances past all three
    Then no ANOMALY is raised
    And the emitted deltas include both 30.0 and 15.0 and are all non-negative

  # source: ResampleHarnessSpec — "emit late arrival side output when predecessor is purged"
  Scenario: A reading whose predecessor was purged is routed to LATE_ARRIVAL
    Given a short buffer retention
    And two readings have advanced the watermark and the early predecessor has been purged
    When an even earlier reading arrives
    Then a LATE_ARRIVAL side output is produced with errorType "late_arrival"
    And its error message indicates there was no predecessor in the buffer

  # source: ResampleHarnessSpec — "track state independently per meter (logicalId)"
  Scenario: Buffers and deltas are isolated per logical meter
    Given interleaved readings for two different logical meters
    When they are processed
    Then each meter's delta is computed only from its own readings
