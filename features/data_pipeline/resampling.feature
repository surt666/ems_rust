Feature: Resampling readings onto fixed time bins
  Irregularly-spaced readings are resampled onto fixed bin boundaries (multiples of the bin
  size from the UTC epoch). Gauges and counters use different rules because they answer
  different questions: a gauge gives the instantaneous value AT a boundary; a counter
  attributes consumed delta ACROSS the bin windows its period overlaps. The same rules run in
  both Flink (streaming) and Glue (batch) — see flink_glue_parity.feature.

  Background:
    Given a meter with resampleMinutes 15 unless stated otherwise
    And bin boundaries are whole multiples of the bin size from the UTC epoch
    And resampling always works on a consecutive (previous, current) reading pair per logical meter

  # ── Bin enumeration: gauges, interval (prevTs, currentTs] ──

  # source: ResampleFunctionSpec — "enumerateBinsIn"
  Scenario Outline: Gauge bin enumeration is left-exclusive and right-inclusive
    Given a previous reading at "<prev>" and a current reading at "<curr>" with 15-minute bins
    When gauge bins are enumerated for the period
    Then the emitted bin boundaries are "<bins>"

    Examples:
      | prev                 | curr                 | bins                                                              |
      | 2026-05-01T10:00:00Z | 2026-05-01T10:14:00Z |                                                                   |
      | 2026-05-01T10:00:00Z | 2026-05-01T10:15:00Z | 2026-05-01T10:15:00Z                                              |
      | 2026-05-01T10:08:00Z | 2026-05-01T10:23:00Z | 2026-05-01T10:15:00Z                                              |
      | 2026-05-01T10:00:00Z | 2026-05-01T10:45:00Z | 2026-05-01T10:15:00Z, 2026-05-01T10:30:00Z, 2026-05-01T10:45:00Z   |

  # source: ResampleFunctionSpec — "enumerateBinsIn ... binSize is zero / currentTs <= prevTs"
  Scenario Outline: Degenerate periods enumerate no gauge bins
    Given a previous reading at "<prev>" and a current reading at "<curr>" with bin size <binMs> ms
    When gauge bins are enumerated
    Then no bins are emitted

    Examples:
      | prev | curr | binMs  |
      | 0    | 1000 | 0      |
      | 1000 | 1000 | 100    |
      | 1000 | 500  | 100    |

  # ── Gauges: linear interpolation ──

  # source: ResampleFunctionSpec / ResampleHarnessSpec — gauge interpolation
  Scenario: A gauge value is linearly interpolated at each passed boundary
    Given a gauge reading 10.0 at "2026-05-01T10:00:00Z"
    And a gauge reading 20.0 at "2026-05-01T10:30:00Z"
    When the pair is resampled
    Then two bins are emitted with resampleMethod "linear_interpolation"
    And the 10:15 bin has resampleValue 15.0 and the 10:30 bin has resampleValue 20.0
    And each emitted row keeps the original current value 20.0 in the value field

  # source: ResampleFunctionSpec — "interpolate gap bins between widely spaced gauge readings"
  Scenario: A wide gauge gap interpolates every intermediate boundary
    Given a gauge reading 0.0 at "2026-05-01T10:00:00Z"
    And a gauge reading 60.0 at "2026-05-01T11:00:00Z"
    When the pair is resampled
    Then four bins are emitted with resampleValues 15.0, 30.0, 45.0, 60.0

  # ── Counters: time-proportional split ──

  # source: ResampleFunctionSpec / ResampleHarnessSpec — counter single bin
  Scenario: A counter delta lands wholly in one bin when the period spans exactly one bin
    Given a counter reading 100.0 at "2026-05-01T10:00:00Z"
    And a counter reading 115.0 at "2026-05-01T10:15:00Z"
    When the pair is resampled
    Then one bin is emitted at "2026-05-01T10:15:00Z"
    And its value field is the delta 15.0
    And its resampleValue is 15.0 with resampleMethod "time_proportional"

  # source: ResampleFunctionSpec — "split delta time-proportionally across multiple bins"
  Scenario: A counter delta is split time-proportionally across the bins it spans
    Given a counter reading 100.0 at "2026-05-01T10:00:00Z"
    And a counter reading 120.0 at "2026-05-01T10:30:00Z"
    When the pair is resampled
    Then two bins are emitted, each carrying delta 20.0 in the value field
    And the resampleValues are 10.0 and 10.0
    And the resampleValues sum to the full delta 20.0 (energy conservation)

  # source: ResampleFunctionSpec — "handle gap with partial first bin overlap"
  Scenario: A partial first-bin overlap splits the delta by overlap fraction
    Given a counter reading 100.0 at "2026-05-01T10:08:00Z"
    And a counter reading 122.0 at "2026-05-01T10:30:00Z"
    When the pair is resampled
    Then the 10:15 bin gets resampleValue 7.0 and the 10:30 bin gets 15.0
    And the two resampleValues sum to the delta 22.0

  # source: ResampleFunctionSpec — "period straddles a boundary mid-period"
  Scenario: A period straddling a boundary contributes to both adjacent bin windows
    Given a counter reading 0.0 at "2026-05-01T09:54:29Z"
    And a counter reading 100.0 at "2026-05-01T10:09:33Z"
    When the pair is resampled
    Then the 10:00 bin and the 10:15 bin each receive a share by overlap = min(curr,B) - max(prev,B-binSize)
    And the two shares sum to the delta 100.0

  # source: ResampleFunctionSpec — "support hourly bins"
  Scenario: Bin size follows the meter's resampleMinutes
    Given a counter meter with resampleMinutes 60
    And readings 100.0 at "2026-05-01T10:00:00Z" and 124.0 at "2026-05-01T11:00:00Z"
    When the pair is resampled
    Then exactly one hourly bin is emitted with resampleValue 24.0

  # ── Pass-through / unconfigured meters ──

  # source: ResampleFunctionSpec / ResampleHarnessSpec — "resampleMinutes is null"
  Scenario: A meter without resampleMinutes still emits the raw row, with null bin columns
    Given a meter whose resampleMinutes is null
    And two consecutive readings
    When they are resampled
    Then one row is emitted per reading-pair with resampleTimestamp, resampleValue and resampleMethod all null

  # source: ResampleFunctionSpec — "pass through unknown meter types unchanged"
  Scenario: An unknown meterType passes the value through with null bin columns
    Given a meter whose meterType is neither "gauge" nor "counter"
    When a pair is resampled
    Then the current value passes through and all three bin columns are null

  # source: ResampleFunctionSpec — "produce no rows when period straddles no bin boundary"
  Scenario: A pair entirely inside one bin emits no bin rows for a gauge
    Given a gauge reading at "2026-05-01T10:01:00Z" and one at "2026-05-01T10:14:00Z"
    When the pair is resampled
    Then no bin rows are emitted
