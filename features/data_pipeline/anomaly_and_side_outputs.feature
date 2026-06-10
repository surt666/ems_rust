Feature: Counter anomalies and error side outputs
  A counter must be monotonically non-decreasing; a decrease means a reset or bad reading and
  is treated as an anomaly rather than a negative consumption. All non-happy-path records leave
  the operator chain through one of four typed side outputs that share a single error-stream
  record shape, discriminated by a "type" field.

  Background:
    Given the pipeline exposes four side-output tags: PARSE_ERROR, DEAD_LETTER, ANOMALY, LATE_ARRIVAL

  # source: ResampleFunctionSpec / ResampleHarnessSpec — negative delta
  Scenario: A negative counter delta is an ANOMALY, not negative consumption
    Given a counter reading 150.0 followed by 140.0
    When the pair is resampled
    Then no bin rows are emitted for that pair
    And an ANOMALY side output is produced with errorType "anomaly"
    And the error message mentions a negative counter delta
    And the meter's state still advances so it is not stuck on the bad reading

  # source: EnrichmentPipelineSpec — "detect anomaly correctly even with out-of-order arrival"
  Scenario: Anomaly detection uses event-time order, not arrival order
    Given counter readings where the event-time-sorted sequence contains a decrease
    When they are processed out of arrival order
    Then the decrease is still flagged as the anomaly and the earlier valid delta is not

  # source: SideOutputTagsSpec — "have distinct tag IDs"
  Scenario Outline: Each side-output tag has a stable distinct id
    Then the "<tag>" side output has id "<id>"

    Examples:
      | tag          | id           |
      | PARSE_ERROR  | parse-error  |
      | DEAD_LETTER  | dead-letter  |
      | ANOMALY      | anomaly      |
      | LATE_ARRIVAL | late-arrival |

  # source: SideOutputTagsSpec — ErrorRecord JSON shape
  Scenario: Error records serialise with a "type" discriminator, not "errorType"
    Given an ErrorRecord with errorType "parse_error", a daqId, a payload and an error message
    When it is serialised to JSON for the error stream
    Then the JSON field is named "type" and never "errorType"
    And the daq_id and error fields are present
