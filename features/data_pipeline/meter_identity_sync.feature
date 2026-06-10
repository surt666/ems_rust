Feature: meter-identity bootstrap and change-data-capture
  The enrichment operator's broadcast state is seeded once from a full DynamoDB scan at
  cold start, then kept live by a DynamoDB Streams source. Insert/modify replace a mapping;
  remove deletes it. The stream is read from TRIM_HORIZON so changes during bootstrap are
  not lost.

  Background:
    Given the meter-identity table keys items by a hash-bucket pk and the daq_id as sk
    And each item carries logical_id (binary UUID), meter_type, hierarchy_path, and optionally resample_minutes and purpose

  # source: DdbStreamDeserializerSpec — "parse INSERT event"
  Scenario: An INSERT stream event becomes a put into broadcast state
    Given a DynamoDB Streams INSERT event with a NewImage for "daq:std_json_v1:cust:meter1:temp"
    When DdbStreamDeserializer.deserialize runs
    Then the change eventType is "INSERT"
    And the daqId is "daq:std_json_v1:cust:meter1:temp"
    And the mapping is defined with the parsed logicalId, meterType and hierarchy node ids

  # source: DdbStreamDeserializerSpec — "parse MODIFY event"
  Scenario: A MODIFY event carries the updated mapping
    Given a DynamoDB Streams MODIFY event for "daq:emu:cust:m2:energy"
    When it is deserialized
    Then the eventType is "MODIFY" and the mapping reflects the new image's values

  # source: DdbStreamDeserializerSpec — "parse REMOVE event using OldImage"
  Scenario: A REMOVE event reads the OldImage and produces no mapping
    Given a DynamoDB Streams REMOVE event whose OldImage identifies "daq:std:cust:m3:temp"
    When it is deserialized
    Then the eventType is "REMOVE" and the mapping is None
    And the enrichment operator removes that daqId from broadcast state

  # source: DdbStreamDeserializerSpec — "parse resample_minutes and purpose when present"
  Scenario: Optional resample_minutes and purpose are read when present
    Given an INSERT image with resample_minutes 15 and purpose "main meter"
    When it is deserialized
    Then the mapping's resampleMinutes is 15 and purpose is "main meter"

  # source: DdbStreamDeserializerSpec — "set optional fields empty/null when not present"
  Scenario: Absent optional fields default to null resampleMinutes and empty purpose
    Given an INSERT image with no resample_minutes and no purpose
    When it is deserialized
    Then the mapping's resampleMinutes is null and purpose is ""

  # source: EnrichmentMiniClusterSpec — "Mapping update"
  Scenario: A CDC update changes how later records for the same daq_id enrich
    Given a record enriched under a mapping with logicalId 1 and hn3 10
    When the mapping for that daq_id is updated to logicalId 2 and hn3 99 via CDC
    And a later record for the same daq_id arrives
    Then the later record enriches with logicalId 2 and hn3 99

  # source: EnrichmentMiniClusterSpec — "Multiple meters"
  Scenario: Distinct daq_ids on the same gateway route to their own logical meters
    Given mappings for an energy daq_id (counter, logicalId 100) and a temperature daq_id (gauge, logicalId 200)
    When readings for both arrive interleaved
    Then each reading is enriched under its own mapping and routed to its own logical meter
