Feature: Device payload parsing and record validation
  The Flink job reads raw device JSON from the Kinesis DAQ input stream and parses each
  device-specific format into one or more canonical SensorRecords. Malformed payloads and
  records that fail validation must never reach the enrichment stage or a sink as good data —
  they are dropped or routed to PARSE_ERROR instead.

  Background:
    Given the Flink pipeline is consuming the DAQ input stream
    And each processor transforms its device format into canonical SensorRecords

  # source: StdProcessorSpec — "transform STD JSON data correctly"
  Scenario: One SensorRecord is emitted per sensor in a std_json_v1 batch
    Given a std_json_v1 payload for customer "iotfabrikken" meter "33333" with 5 sensors
    When the StdProcessor transforms it
    Then exactly 5 SensorRecords are produced
    And each record's daq_id is "daq:std_json_v1:<customerid>:<meterid>:<sensorid>"
    And each record carries the original timestamp, value (as string) and unit unchanged
    And the value 0 with an empty unit is preserved, not dropped

  # source: StdProcessorSpec — "handle gatewayid field with fallback logic"
  Scenario: gatewayId falls back to customerid but daq_id always uses customerid
    Given a std_json_v1 record whose gatewayid is the literal "None"
    When the StdProcessor transforms it
    Then the record's gatewayId falls back to the customerid
    And the record's daq_id still uses the customerid, never the gateway value

  # source: StdProcessorSpec — "throw exception when required fields are missing"
  Scenario: Missing required top-level fields is a hard parse failure
    Given a payload missing "customerid" or "data"
    When the StdProcessor transforms it
    Then an IllegalArgumentException is raised
    And the payload is treated as a parse failure, not silently skipped

  # source: ProcessUtilsSpec — "filterValid"
  Scenario Outline: A record is valid only when its value and timestamp both parse
    Given a SensorRecord with value "<value>" and timestamp "<timestamp>"
    When ProcessUtils.filterValid runs over it
    Then the record is <kept_or_dropped>

    Examples:
      | value        | timestamp                   | kept_or_dropped |
      | 123.45       | 2025-01-06T08:00:00.000000Z | kept            |
      | 0            | 2025-01-06T08:00:00.000000Z | kept            |
      | not-a-number | 2025-01-06T08:00:00.000000Z | dropped         |
      | 123.45       | not-a-timestamp             | dropped         |

  # source: ProcessUtilsSpec — "filter out null records"
  Scenario: Null records are removed without aborting the batch
    Given a batch containing a null record between two valid records
    When ProcessUtils.filterValid runs
    Then the null is removed and both valid records survive

  # source: ProcessUtilsSpec — "return empty sequence on exception"
  Scenario: A transform that throws yields no records rather than crashing the operator
    Given a transform function that raises an exception
    When ProcessUtils.processDataToRecords invokes it
    Then an empty sequence is returned

  # source: EnrichmentMiniClusterSpec — "Parse error routing"
  Scenario: Unparseable payloads are routed to PARSE_ERROR and produce no enriched records
    Given an unknown schematype payload and a payload that is not valid JSON
    When the pipeline processes them
    Then no EnrichedRecords are produced
    And both payloads appear in the PARSE_ERROR side output with errorType "parse_error"
