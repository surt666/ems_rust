# Enity EMS Data Acquisition & Processing System — Design Document

**Version:** 0.1 (Draft)
**Date:** 2026-04-10
**Status:** Living document — foundational structure, details to be added incrementally

---

## 1. Strategic Context

Enity EMS is an Energy Management System that ingests sensor data from heterogeneous IoT sources, enriches it with organizational context, computes consumption metrics, and serves it to end users for energy monitoring, budgeting, and climate accounting. The system follows **Domain-Driven Design** principles with clearly separated Bounded Contexts, an event-sourced append-only data model, and a streaming-first architecture.

### 1.1 Ubiquitous Language

| Term                      | Definition                                                                                                                                 |
| ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| **DAQ ID**                | A globally unique identifier for a physical sensor data stream: `protocol:schematype:customer:gatewayid:meterserial:sensorid`              |
| **Logical Sensor**         | A domain concept aggregating one or more physical sensors over time, identified by a ID (`logical_id`)                                    |
| **Sensor Type**            | Either `gauge` (instantaneous reading — temperature, power) or `counter` (cumulative reading — kWh, m^3)                                  |
| **Delta**                 | The computed difference between consecutive counter readings: `delta(t_n) = cumulative(t_n) - cumulative(t_{n-1})`                         |
| **Baseline/Offset**       | The first counter reading for a meter; absorbed silently since `delta` requires two points                                                 |
| **Enrichment**            | The process of resolving a physical `daqId` to its logical meter identity and hierarchy position                                           |
| **Hierarchy Node**        | A vertex in the building element tree (Partner, Company, Property, Group, Building, Area)                                                  |
| **Main Meter**            | A meter with no parent in the meter forest — a root node in the meter tree                                                                 |
| **Sub-Meter**             | A meter with a parent meter; measures a subset of what the parent measures. Edge `(M_parent, M_sub)` in the meter forest                   |
| **Summation Meter**       | A meter that participates in aggregate consumption calculations, subject to ancestor-exclusion rules to prevent double-counting            |
| **Calculation Meter**     | A virtual meter whose value is derived from other meters via a formula (e.g., difference, sum, ratio) rather than from a physical sensor   |
| **Tombstone**             | A sentinel value (`-1111111`) appended to logically delete a reading in the append-only data model                                         |
| **Manual Correction**     | A human-entered insert or tombstone appended to `logical_sensor_data`, labelled with origin and reason                                     |
| **Manual Meter**          | A logical meter with no underlying pipeline data — only manual correction records                                                          |
| **Degree Day Correction** | Temperature normalization: `corrected = (consumption - base_load) / actual_dd * normal_dd + base_load` should be reevaluated (divide by 0) |

---

## 2. Bounded Contexts

The system is decomposed into five Bounded Contexts, each with its own models, responsibilities, and integration points.

```
+--------------------+      +-------------------------+      +---------------------+
|   Data Acquisition |      |  Data Feature           |      |  Energy Management  |
|   (DAQ)            |----->|  Engineering (DFE)      |----->|  (EMS)              |
|                    |      |                         |      |                     |
| Ingestion, Parsing,|      | Enrichment, Delta,      |      | Dashboards, Alarms, |
| Raw Storage        |      | Aggregation, Correction |      | Reports, Viz        |
+--------------------+      +-------------------------+      +---------------------+
         |                            |                               |
         v                            v                               v
+------------------------------------------------------------------------+
|                    Identity & Configuration Context                     |
|                                                                        |
|  Meter Identity (DynamoDB), Meter Type, Resampling, Hierarchy Path        |
+------------------------------------------------------------------------+
                                      ^
                                      |
+------------------------------------------------------------------------+
|              Organization & Access Control Context (OAC)               |
|                                                                        |
|  Hierarchy Nodes, Users, Permissions, Meter Attachment                 |
+------------------------------------------------------------------------+
```

### 2.1 Data Acquisition Context (DAQ)

**Responsibility:** Ingest raw sensor data from all sources, parse device-specific protocols into a canonical `SensorRecord`, and persist all raw data unconditionally.

**Aggregate Root:** `SensorRecord`

### 2.2 Data Feature Engineering Context (DFE)

**Responsibility:** Enrich sensor records with logical identity and hierarchy context, compute counter deltas, detect anomalies, handle late arrivals, and produce the curated `logical_sensor_data` dataset. Also performs aggregation (hourly, daily) and degree-day correction.

**Aggregate Roots:** `EnrichedRecord`, `Aggregation`

### 2.3 Organization & Access Control Context (OAC)

**Responsibility:** Own the building element hierarchy (the organizational tree), user lifecycle, and permission graph. This context is the authority on _who can see what_ and _how the organization is structured_. It manages CRUD operations for hierarchy nodes, user accounts (Cognito + DynamoDB), and the permission edges that connect users to nodes.

**Aggregate Roots:** `HierarchyNode`, `User`

**Entities:** `EdgePermission`

**Key Operations:**

- Hierarchy node CRUD (create, read, update, soft-delete via `blocked` flag)
- User lifecycle (create in Cognito + DynamoDB, delete with cross-system rollback)
- Permission traversal (precedence-based scope resolution)
- Subtree queries (materialized path prefix scans)

### 2.4 Energy Management Context (EMS)

**Responsibility:** Visualization and presentation layer. Consumes enriched data from DFE and hierarchy/permission context from OAC to render dashboards, reports, alarms, and energy analysis. This context does not own any domain entities — it reads from other contexts and presents.

**Key Capabilities:**

- Hierarchy navigation (HTMX-driven tree with lazy loading)
- Consumption dashboards and time-series visualization
- Alarm configuration and display (daily, hourly, cumulative, budget, cooling)
- Degree-day corrected views
- CO2 and energy price overlays (future)
- CSV export / reporting

### 2.5 Identity & Configuration Context

**Responsibility:** Maintain the mapping between physical sensors (`daqId`) and logical meters (`logical_id`), including hierarchy path, meter type, and resampling configuration. Propagated via CDC (DynamoDB Streams). This context is downstream of OAC — when the hierarchy changes or meters are attached/detached, those changes flow into `meter-identity` and are picked up by DFE.

**Aggregate Root:** `MeterMapping`

---

## 3. Domain Model

### 3.1 Building Element Hierarchy (Tree Model)

The organizational structure is modeled as a **rooted forest** `F = (V, E)` where:

- `V` is the set of **hierarchy nodes** (vertices)
- `E subset V x V` is the set of **parent-child edges**, where each edge `(u, v)` means `u` is the parent of `v`
- Every vertex `v in V` has at most one parent: `|{u : (u,v) in E}| <= 1`
- The graph is acyclic (DAG constraint: no vertex is its own ancestor)

All hierarchy nodes share the same entity structure — they differ only by their **metadata variant** and their position in the tree. This is a key design decision: the tree is homogeneous in structure, heterogeneous in metadata.

#### 3.1.1 Node Types and Tree Constraints

Metadata fields below are examples. They should be taken from EMS, but maybe validate which are actually used, as there are a lot of them.

```
Level   Node Type    Prefix   Metadata Fields                          Children Allowed
-----   ---------    ------   ---------------                          ----------------
0       Root         H#       (system node)                            Partner
1       Partner      P#       email, address, phone                    Company
2       Company      C#       email, address, cvr, phone, contact,     Property, Group, Building
                              homepage, status, sla
3       Property     PR#      location, address, usage, bbr,           Building
                              weather_station
3       Group        G#       name                                     Building
4       Building     B#       email, location, usage, total_area,      Area
                              heated_area, bbr, build_year, p_nr,
                              ext_id
5       Area         A#       name                                     (leaf)
```

**Formal constraints:**

- `forall v in V: type(v) in {Root, Partner, Company, Property, Group, Building, Area}`
- Parent type must precede child type in the partial order above
- Company nodes cannot span multiple Partner subtrees
- Meter hierarchy cannot span multiple Company subtrees
- Buildings are the lowest level that can contain further structure (Areas)
- Areas are always leaf nodes

#### 3.1.2 HierarchyNode Entity

```
HierarchyNode := {
  id:       NodeId (u32)          -- unique numeric identifier
  type:     NodeType              -- determines metadata variant and tree position
  name:     String                -- display name
  parent:   Option<Path>          -- materialized path e.g. "H#root#P#1#C#2"
  metadata: Option<MetaData>      -- type-discriminated metadata (see 3.1.1)
  blocked:  Bool                  -- access control flag
  created:  DateTime<UTC>
}
```

The **materialized path** encodes the full ancestry from root, enabling efficient subtree queries in DynamoDB using prefix-based sort key scans.

#### 3.1.3 Hierarchy Path Encoding

For the streaming pipeline, hierarchy position is encoded as a compact string:

```
P{partnerId}#C{companyId}[#PR{propertyId}][#G{groupId}][#B{buildingId}][#A{areaId}]
```

Partner and Company are always present. Property, Group, Building, and Area are optional depending on where the meter attaches. Valid structural paths:

```
P1#C2#B8                    -- Partner -> Company -> Building
P1#C2#B8#A3                 -- Partner -> Company -> Building -> Area
P1#C2#G5#B8                 -- Partner -> Company -> Group -> Building
P1#C2#G5#B8#A3              -- Partner -> Company -> Group -> Building -> Area
P1#C2#PR4#B8                -- Partner -> Company -> Property -> Building
P1#C2#PR4#B8#A3             -- Partner -> Company -> Property -> Building -> Area
```

### 3.2 Meter Model

#### 3.2.1 Meter Entity

?? Why meter and not sensor ??

A **meter** is a central domain concept. Every meter:

- Belongs to exactly one building element (Building or Area)
- Has a globally unique **meter ID** (unique across all customers)
- Has exactly one **meter type** (see 3.2.2)
- Has exactly one **energy type** (see 3.2.3) 
- Is either a **main meter** (no parent) or a **sub-meter** (has one parent meter)
- Has a boolean **`part_of_summation`** flag (stored on the meter entity in DynamoDB) that controls whether the meter participates in aggregate consumption calculations (see 3.2.6)

```
Meter := {
  meter_id:           String              -- globally unique across all customers
  meter_type:         MeterType           -- Guage or Counter (Counter Resampled ?)
  energy_type:        EnergyType          -- Heat, Water, Electricity, Transport, etc.
  parent_meter_id:    Option<String>      -- REMOVE ? if set, this meter is a sub-meter (Not as field but as an edge)
  part_of_summation:  Bool                -- flag controlling summation participation
  building_element:   NodeId              -- REMOVE the Building or Area this meter belongs to (Not as field but as an edge)
  unit:               MeterUnit           -- REMOVE the unit the meter reports in (GJ, kWh, m3, etc.)? Why is this needed. Normalize everything to SI units ?
}
```

#### 3.2.2 Meter Types

A meter type describes what the meter physically measures:

| Meter Type     | Typical Unit | Notes                        |
| -------------- | ------------ | ---------------------------- |
| Electricity    | kWh          |                              |
| Natural gas    | m^3 (Ngas)   | Regional gas variants exist  |
| Water          | m^3          |                              |
| District heat  | GJ / kWh     | Often has Counter 2 (water)  |
| District cool  | kWh          | See 3.3.5 computed cooling   |
| Petrol car     | L            | Transport energy             |

#### 3.2.3 Energy Types

An energy type is an abstract classification used for grouping, reporting, and dashboard filtering:

Energy types: **Heat**, **Water**, **Electricity**, **Transport**, **Counter** (generic cumulative), ...

A meter type maps to an energy type (e.g., District heat -> Heat, Natural gas -> Heat, Electricity -> Electricity).

#### 3.2.4 Units and Conversion

Each meter reports values in a **meter unit** (e.g., GJ, MWh, m^3-Ngas). The system normalizes to a **base unit** per category using a conversion factor:

| Category | Meter Units                    | Base Unit | Example Conversion              |
| -------- | ------------------------------ | --------- | ------------------------------- |
| Energy   | GJ, kWh, MWh, Gcal             | Wh        | 1 MWh = 1,000,000 Wh            |
| Volume   | Liter, m^3, Gallon             | m^3       | 1 Liter = 0.001 m^3             |
| Gas      | m^3-Ngas, m^3-Bgas, m^3-Fgas   | m^3       | Regional variants               |

Measurement systems: Metric, US Imperial, UK Imperial (configurable per customer).

?? If everything is just normalized to SI units, it's trival for the frontend to apply whatever conversion a user wants ??

#### 3.2.5 Meter Hierarchy (Forest)

Meters form a separate **forest** `M = (V_m, E_m)` overlaid on the building hierarchy:

- Each meter `m in V_m` attaches to exactly one Building or Area node
- A **sub-meter** `Ms` has a **parent meter** `Mp` and measures *a part* of what `Mp` measures (and nothing else)
- A meter can have zero or one parent meter, and any number of child meters
- Cycles are not allowed — the meter hierarchy is a forest (disjoint union of trees)
- The meter hierarchy may span multiple buildings (a meter on one building can have a parent meter on another building)
- The meter hierarchy may **not** span multiple companies

```
M_main (main meter, no parent)
  |-- M_sub1 (sub-meter of M_main)
  |-- M_sub2 (sub-meter of M_main)
       |-- M_sub3 (sub-meter of M_sub2)
```

For a meter `Mx`, any meter reachable by going to the parent meter one or more times is called an **ancestor** of `Mx`.

#### 3.2.6 Counters (Sensors within a Meter)

Each physical meter has one or more **counters** (physical sensors):

| Counter    | Purpose                                                  | Type    |
| ---------- | -------------------------------------------------------- | ------- |
| Counter 1  | Primary consumption (e.g., kWh, m^3)                     | counter |
| Counter 2  | Secondary consumption (e.g., water for district heating) | counter |
| Counter 3  | Tertiary consumption (rarely used, not displayed in EMS) | counter |
| Counter 10 | Forward temperature                                      | gauge   |
| Counter 11 | Return temperature                                       | gauge   |

Counter values can represent: **consumption** (delta), **running total** (cumulative counter reading), or **gauge values** (instantaneous).

#### 3.2.7 Sensor Periods

A meter's counters map to different physical **DAQ IDs** over time as data sources change (e.g., a meter migrating from Electrocom to LoRaWAN). Each mapping has a time period during which it is active:

```
| Meter     | Period A | Period B | ... |
| --------- | -------- | -------- | --- |
| Counter 1 | DAQ A1   | DAQ B1   | ... |
| Counter 2 | DAQ A2   | DAQ B2   | ... |
```

The `meter-identity` DynamoDB table maintains these mappings. When a period changes, the old DAQ ID stops producing enriched data and the new one takes over. Historical data remains under the same `logical_id`.

#### 3.2.8 Summation Meters

When computing aggregate consumption for a building element subtree (e.g., "total electricity for Building X this month"), simply summing all meters would double-count because sub-meter consumption is already included in the parent meter's reading. **Summation meters** solve this.

**`Part of summation`** is a boolean flag stored on each meter entity in DynamoDB. It is not derived — it is explicitly set by the user when configuring the meter.

Determining which meters to sum depends on three things:

1. The **meter hierarchy** (parent-child relationships)
2. The **`part_of_summation`** flag on each meter
3. The **summation context** — the set of meters being considered (typically all meters of a given energy type within a building element subtree)

A meter is a **summation meter** within a summation context `S` iff:

- it belongs to `S`,
- its `part_of_summation` flag is set, **and**
- it does **not** have an ancestor that is also `part_of_summation` and also belongs to `S`

Formally:

```
SummationMeters(S) = { m in S : partOfSummation(m) AND
                       NOT EXISTS m' in ancestors(m) such that
                       m' in S AND partOfSummation(m') }
```

Total consumption for context `S`:

```
Consumption(S) = SUM_{m in SummationMeters(S)} delta(m)
```

**Worked example:**

```
B1:                    B2:                    B3:
  M1- (not summed)       M4+ ──> M1             M8+ ──> M4
    M2- ──> M1             M5+ ──> M4
    M3+ ──> M1             M6+ ──> M4
                           M7+

(+ = part_of_summation, - = not part_of_summation, ──> = child-parent)
```

| Summation Context | Summation Meters | Why                                                            |
| ----------------- | ---------------- | -------------------------------------------------------------- |
| `B1`              | `M3`             | Only `M3` has the flag; `M1`, `M2` do not                     |
| `B2`              | `M4`, `M7`       | `M5`, `M6` excluded: ancestor `M4` is in context and flagged  |
| `B3`              | `M8`             | `M4` (ancestor) is not in `B3`, so `M8` is not excluded       |
| `B1`+`B2`         | `M3`, `M4`, `M7` | `M4` has no flagged ancestor in context (M1 is not flagged)   |
| `B2`+`B3`         | `M4`, `M7`       | `M8` excluded: ancestor `M4` is now in context and flagged    |
| `B1`+`B2`+`B3`    | `M3`, `M4`, `M7` | Same as `B1`+`B2` — `M8` excluded by `M4`                    |

Note how the summation meters change depending on context: `M8` is a summation meter in `B3` alone but not when `B2` is included, because its ancestor `M4` enters the context.

#### 3.2.9 Calculation Meters

A **calculation meter** (Danish: *beregningsmåler*) is a virtual meter whose value is not sourced from a physical sensor but computed from other meters via a formula. Examples:

- **Difference:** `M_calc = M_main - M_sub` (compute unmeasured remainder)
- **Sum:** `M_calc = M_a + M_b` (aggregate across sub-meters)
- **Ratio:** `M_calc = M_energy / M_volume` (e.g., district cooling efficiency)

Calculation meters exist as logical meters in the hierarchy with no underlying `daqId`. Their values are derived at query time or materialized by a batch process. The formula, operand meter references, and operator are stored as meter configuration.

A **manual meter** is a logical meter with no underlying automated sensor data — only manually ingested records (via CSV upload or HTTPS). It participates in the hierarchy and summation like any other meter.

> **Note:** The calculation meter specification is incomplete in the source documentation. The formula language, execution model (query-time vs. materialized), and interaction with summation rules need to be defined.

### 3.3 Measurement & Aggregation Model

#### 3.3.1 Raw Measurement

The raw measurement is stored in the `raw_data` Iceberg table before any enrichment. It carries only the physical sensor identity — no hierarchy context.

```
RawMeasurement := {
  daq_id:        String           -- physical sensor identifier ("daq:protocol:schematype:apiprovider|company|gatewayid|metermanufacturer:meterid:sensorid")
  timestamp:     DateTime<UTC>    -- event time from the device
  value:         Double           -- raw reading (cumulative for counters, instantaneous for gauges)
  unit:          String           -- physical unit as reported by device (pre-normalization)
  ingested_time: DateTime<UTC>    -- wall-clock time when the record entered the pipeline
}
```

#### 3.3.2 Logical Sensor Data

Logical sensor data is the enriched, curated time series stored in the `logical_sensor_data` Iceberg table. It is the merge of pipeline-produced records and manual corrections into a single append-only event-sourced stream per logical meter.

**Append-only semantics:** The system only performs Create and Read operations — never Update or Delete. Corrections are new records appended with a later `created` timestamp for the same `(logical_id, timestamp)` key. Consumers resolve to the current value via `MAX(created)`.

**Tombstone deletion:** To logically delete a reading, a correction record is appended with a sentinel value of `-1111111`. Consumers treat this value as "no data at this timestamp."

**Manual corrections** (inserts and tombstone deletes) are ingested via CSV file upload or HTTPS request — they flow through the standard pipeline like any other data source, not via a side-channel. A manual meter is simply a logical meter with no underlying automated sensor data — only manually ingested records.

**Resampling:** When a meter has a `resample_minutes` configuration (e.g., 15 minutes) on `meter-identity`, the pipeline emits per-bin rows. **Gauges** emit one row per bin boundary in `(prev_ts, current_ts]` with the value linearly interpolated between the bracketing readings. **Counters** emit one row per bin whose **window** `[B − binSize, B]` overlaps the period `[prev_ts, current_ts]`, with `resample_value` equal to a time-proportional share of the delta — a single reading can contribute to multiple bins, and a single bin can receive contributions from multiple readings (consumer SUMs after dedup). `timestamp` and `value` carry the original reading time and the delta (counter) / instantaneous value (gauge); `resample_timestamp` / `resample_value` / `resample_method` carry the resampled form. Meters without a `resample_minutes` configuration get `bin_*` = NULL (raw shape preserved). Full rules: `docs/superpowers/specs/2026-05-01-resampling-rules-design.md`.

```
LogicalSensorRecord := {
  logical_id:     String              -- UUID of the logical meter
  timestamp:      DateTime<UTC>       -- original reading time (un-floored)
  value:          Double              -- delta for counters, instantaneous for gauges, -1111111 for tombstone
  unit:           String              -- normalized unit
  ingested_time:  DateTime<UTC>       -- when record entered the system
  created:        DateTime<UTC>       -- when this version of the record was created (for event-sourcing)
  partner_id:     Int
  company_id:     Int
  property_id:    Option<Int>
  building_id:    Option<Int>
  area_id:        Option<Int>
  group_id:       Option<Int>
  resample_timestamp:  Option<DateTime<UTC>>  -- bin boundary (multiples of `resample_minutes` from epoch); NULL when meter has no resampling
  resample_value:      Option<Double>         -- linearly interpolated (gauge) or time-proportional share (counter)
  resample_method:     Option<String>         -- "linear_interpolation" | "time_proportional" | "nearest_neighbor"
}
```

**Resolution rule:** For any `(logical_id, timestamp)` pair, the authoritative value is:

```
current_value(logical_id, t) = value WHERE created = MAX(created) AND value != -1111111
```

If the most recent record has `value = -1111111`, the reading is considered deleted.

#### 3.3.3 Aggregation

Consumption is derived from cumulative counter readings via interpolation at time boundaries:

```
consumption(t_start, t_end) = interpolated_value(t_end) - interpolated_value(t_start)
```

Where `interpolated_value(t)` uses linear interpolation between the nearest measurements before and after `t`:

```
v(t) = v(t_before) + (v(t_after) - v(t_before)) * (t - t_before) / (t_after - t_before)
```

Aggregation levels:

| Level  | Resolution | TTL       |
| ------ | ---------- | --------- |
| Hourly | 1 hour     | 90 days   |
| Daily  | 1 day      | Permanent |

Daily aggregation is the sum of 24 hourly aggregations. THIS IS NOT NEEDED IN AN EVENT SOURCED SYSTEM ~Change detection uses SHA-256 hashing to avoid redundant writes.~

#### 3.3.4 Degree Day Correction

For temperature-dependent consumption (heating/cooling):

```
corrected = (consumption - base_load) / actual_degree_days * normal_degree_days + base_load
```

Applied only when `consumption >= base_load`. Degree days for heating:

```
DD(day) = max(0, T_base - T_avg(day))
```

Where `T_base` is the base temperature and `T_avg` is the average daily temperature.

#### 3.3.5 District Cooling

```
computed_cooling = 0.859845 * C * energy_consumption / water_consumption    [m^3/kWh]
```

---

## 4. Data Sources

The system ingests data from heterogeneous sources via multiple transport protocols:

### 4.1 Source Inventory

| Source     | Protocol    | Device/Format      | Processor               | Key Characteristics                                                                                                         |
| ---------- | ----------- | ------------------ | ----------------------- | --------------------------------------------------------------------------------------------------------------------------- |
| IoT Core   | LoRaWAN     | EMU Profes         | `EmuProcessor`          | Base64 binary, TLV energy records                                                                                           |
| IoT Core   | LoRaWAN     | FlowIQ 2200        | `Flowiq2200Processor`   | M-Bus TPL/APL (EN 13757-3:2018), ultrasonic flow/temp                                                                       |
| IoT Core   | LoRaWAN     | Adeunis Pulse      | `PulseProcessor`        | Base64 binary, dual counters (A/B)                                                                                          |
| IoT Core   | LoRaWAN     | MC-603             | `Mc603Processor`        | Kamstrup heat meter, binary protocol                                                                                        |
| IoT Core   | HTTPS       | Standard JSON      | `StdProcessor`          | Newline-delimited JSONL, multi-record                                                                                       |
| IoT Core   | MQTT        | Blue Metering      | `BluemeteringProcessor` | OBIS-indexed, single record per message                                                                                     |
| IoT Core   | MQTT        | Mivo               | `MivoProcessor`         | Multi-meter, multi-value readings                                                                                           |
| IoT Core   | MQTT        | GWB-143            | `Gwb143Processor`       | Telemetry bridge, unit filtering                                                                                            |
| Electrocom | HTTPS GET   | Electrocom loggers | `ElectrocomProcessor`   | 4,814 loggers / 23,685 meters. Known reliability issues: memory corruption causing data spikes and extended corrupt periods |
| Email Push | SMTP        | Ediel (DataHub)    | `EdielProcessor`        | Energy market data, space-delimited timestamps                                                                              |
| Pull API   | HTTPS       | CenterDanmark      | `StdProcessor`          | —                                                                                                                           |
| Pull API   | HTTPS       | Danfoss            | (planned)               | —                                                                                                                           |
| Pull API   | HTTPS       | Entelios           | `StdProcessor`          | —                                                                                                                           |
| Pull API   | HTTPS       | Lichtwart          | (planned)               | —                                                                                                                           |
| Pull API   | HTTPS       | AalborgForsyning   | (planned)               | —                                                                                                                           |
| Pull API   | HTTPS       | Brunata            | (planned)               | —                                                                                                                           |
| Pull API   | HTTPS       | Techem             | (planned)               | —                                                                                                                           |
| CSV Import | File Upload | Manual/bulk data   | (planned)               | READy, Hofor, etc                                                                                                           |

### 4.2 Source Integration Topology

```
                   LoRaWAN Gateways
                   (EMU, FlowIQ, MC-603, Pulse, Electrocom)
                          |
                          v
                   AWS IoT Core (MQTT)
                          |
                          v
+-----------+      +--------------+      +-------------------+
| Pull APIs |----->|   Kinesis    |<-----|  HTTPS Push APIs   |
| (DataHub, |      |   Input      |      |  (StdJSON, Blue,   |
|  Brunata, |      |   Stream     |      |   Mivo, GWB-143)   |
|  Techem,  |      +--------------+      +-------------------+
|  etc.)    |             |
+-----------+             |               +-------------------+
                          |               |  CSV Import       |
                          |               |  (manual upload)  |
+-----------+             v               +-------------------+
| DynamoDB  |      +------------------+          |
| meter-    |----->|  Apache Flink    |<---------+
| identity  |      |  (Scala 3)      |      (future)
| (CDC)     |      +------------------+
+-----------+             |
                    +-----+-------+-------+
                    |             |       |
                    v             v       v
              +---------+  +---------+  +---------+
              |raw_data |  |logical_ |  | Error   |
              |(Iceberg)|  |meter_   |  | Kinesis |
              |         |  |data     |  | Stream  |
              +---------+  |(Iceberg)|  +---------+
                           +---------+       |
                                             v
                                       +----------+
                                       | Lambda   |
                                       | (Late    |
                                       |  Arrival |
                                       |  Trigger)|
                                       +----------+
                                             |
                                             v
                                       +----------+
                                       | Glue Job |
                                       | (Recomp.)|
                                       +----------+
```

---

## 5. Streaming Pipeline (Flink Topology)

### 5.1 Canonical Record Format

All device-specific processors normalize into a single Value Object:

```
SensorRecord := {
  daqId:        String      -- "daq:{type}:{customer|gatewayid}:{meter}:{sensor}" daq could be substituted for protocol type like lora,mqtt,http
  type:         String      -- schema type identifier
  gatewayId:    String      -- customer / datasource ID
  meterId:      String      -- device EUI or serial
  timestamp:    String      -- ISO 8601
  ingestedTime: String      -- wall-clock arrival time
  sensorId:     String      -- sensor within meter
  value:        String      -- numeric reading (as string)
  unit:         String      -- physical unit (pre-normalization)
}
```

DAQ ID construction: `daq:{type}:{customerId}:{meterId}:{sensorId}` — all lowercase, dashes and spaces replaced with underscores.

### 5.2 Data Filtering (Two-Stage)

Data quality filtering is applied at two stages in the pipeline:

**Stage A — Processor-level validation:** Each device-specific processor rejects obviously corrupted data packages during parsing. This includes structural validation (missing fields, malformed payloads, unparseable binary) and basic value sanity (non-numeric values, impossible units). Rejected records emit `PARSE_ERROR`.

**Stage B — Temporal validation in enrichment:** The enrichment and delta stages apply windowed validation against the event-time horizon. The 1-hour watermark window and 6-hour buffer retention provide two temporal scopes for detecting anomalies: negative deltas (counter resets, corruption), and late arrivals beyond the buffer window. Records failing temporal validation are emitted as `ANOMALY` or `LATE_ARRIVAL` side outputs.

All error types are currently routed to a single Kinesis error stream, discriminated by a `type` field. Whether to split into dedicated streams per error category is a future decision.

### 5.3 Pipeline Stages

```
Stage 1: PARSE         Kinesis -> JSON Parse -> Route to Processor -> SensorRecord
                       Side output: PARSE_ERROR (malformed/unknown schema)

Stage 2: RAW SINK      SensorRecord -> Row mapping -> Iceberg "raw_data"
                       (unconditional write, no enrichment needed)

Stage 3: WATERMARK     BoundedOutOfOrderness(1 hour), idleness=24 hours

Stage 4: ENRICH        BroadcastProcessFunction
                       Main input: SensorRecord (watermarked)
                       Broadcast input: IdMappingChange (DDB CDC)
                       Bootstrap: parallel DDB scan (20 segments, 20K partitions)
                       Lookup: broadcast state -> bootstrap cache -> DEAD_LETTER
                       Output: (EnrichedRecord, MeterMapping)  -- mapping carries meterType + resampling
                       (timestamp passed through un-floored; resampling applied in Stage 5)

Stage 5: RESAMPLING       KeyedProcessFunction (keyed by logicalId) — replaces former CounterDeltaFunction
                       One-reading-lag: prev held in buffer until next arrives
                       Gauge: bins B in (prev_ts, current_ts]; linear interpolation between (prev, current)
                              → resample_method=linear_interpolation
                       Counter: bins whose WINDOW [B-binSize, B] overlaps [prev_ts, current_ts]
                              (i.e., bins B in (prev_ts, ceil(current_ts to grid)])
                              → time-proportional split of (current-prev), resample_method=time_proportional
                              → multiple readings can contribute to the same bin (consumer SUMs)
                       Meters with resample_minutes=null: backward-compat (gauge passthrough; counter delta), bin_*=null
                       Side outputs: ANOMALY (negative counter delta), LATE_ARRIVAL (predecessor purged)
                       State: MapState[Long, BufferedReadingV2], ValueState[Long] (lastEmittedTs)
                       Spec: docs/superpowers/specs/2026-05-01-resampling-rules-design.md

Stage 6: ENRICHED SINK EnrichedRecord -> unit normalization -> Row mapping -> Iceberg "logical_sensor_data"

Stage 7: ERROR SINK    Union(PARSE_ERROR, DEAD_LETTER, ANOMALY, LATE_ARRIVAL) -> Kinesis error stream
```

### 5.4 Enriched Record

```
EnrichedRecord := {
  logicalId:    String              -- UUID from meter-identity
  timestamp:    String              -- original reading time (un-floored)
  value:        Double              -- gauge value or computed counter delta
  unit:         String              -- normalized unit
  ingestedTime: String
  partnerId:    Int
  companyId:    Int
  propertyId:   Option<Int>
  buildingId:   Option<Int>
  areaId:       Option<Int>
  groupId:      Option<Int>
  binTimestamp: Option<Long>        -- bin boundary epoch ms (NULL when meter has no resampling)
  binValue:     Option<Double>      -- linearly interpolated (gauge) or proportional share (counter)
  binMethod:    Option<String>      -- "linear_interpolation" | "time_proportional" | "nearest_neighbor"
}
```

### 5.5 Unit Normalization

The pipeline normalizes 70+ unit variants to canonical base units with scale factors:

| Category    | Examples                                 | Base Unit |
| ----------- | ---------------------------------------- | --------- |
| Energy      | kWh (x1000), MWh (x1e6), Gcal (x1.163e6) | Wh        |
| Power       | kW (x1000), MW (x1e6)                    | W         |
| Volume      | Liter (x0.001), Gallon (x0.003785)       | m^3       |
| Flow        | m3PerHour                                | m^3/h     |
| Temperature | Celsius, Kelvin                          | C / K     |
| Gas         | Ngas, Bgas, Fgas (regional variants)     | m^3       |

### 5.6 Event-Time Semantics

| Parameter            | Value     | Purpose                                          |
| -------------------- | --------- | ------------------------------------------------ |
| Max out-of-orderness | 1 hour    | Watermark bound for late data tolerance          |
| Buffer retention     | 6 hours   | Counter predecessor retention in keyed state     |
| Watermark idleness   | 24 hours  | Mark idle partitions to unblock global watermark |
| Checkpoint interval  | 5 minutes | State durability boundary                        |

---

## 6. Error Taxonomy & Recovery

### 6.1 Error Classification

```
ErrorRecord := {
  type:      ErrorType          -- parse_error | dead_letter | anomaly | late_arrival
  timestamp: String             -- when the error occurred
  daq_id:    String             -- affected sensor (empty for parse errors)
  payload:   String             -- first 1000 chars of relevant data
  error:     String             -- human-readable description
}
```

### 6.2 Error Scenarios and Recovery Paths

| Error Type     | Cause                                                          | Raw Table   | Enriched Table    | Recovery                                            |
| -------------- | -------------------------------------------------------------- | ----------- | ----------------- | --------------------------------------------------- |
| `PARSE_ERROR`  | Malformed JSON, unknown schema, missing fields                 | Not written | Not written       | Fix source device firmware                          |
| `DEAD_LETTER`  | daqId has no meter-identity mapping                            | Written     | Not written       | Add mapping in DynamoDB + Glue recomputation        |
| `ANOMALY`      | Negative counter delta (reset, replacement, corruption)        | Written     | Suppressed        | Manual investigation; value stored as new baseline  |
| `LATE_ARRIVAL` | Record arrives after predecessor purged from buffer (>6h late) | Written     | Initially missing | Automatic: Lambda trigger -> Glue recomputation job |

### 6.3 Late Arrival Recomputation

```
LATE_ARRIVAL error record
    -> Kinesis error stream
    -> LateArrivalTrigger Lambda
        groups by daq_id, computes time range
    -> Glue Spark job: late-data-recomputation
        reads ALL raw records for affected meter from raw_data
        joins with meter-identity from DynamoDB
        computes deltas using LAG() window function over full history
        appends corrected records to logical_sensor_data with created=now()
```

Consumers use event-sourcing semantics: `SELECT ... WHERE created = MAX(created)` per `(logical_id, timestamp)` picks up corrections automatically.

---

## 7. Data Ingestion Scenarios

The pipeline handles 16 distinct scenarios. Key parameters: watermark=1h, buffer retention=6h, checkpoint=5min.

| #   | Scenario                    | Behavior                                                        | Output                                |
| --- | --------------------------- | --------------------------------------------------------------- | ------------------------------------- |
| 1   | Normal in-order             | Buffer -> predecessor lookup -> emit delta                      | Correct delta in `logical_sensor_data` |
| 2   | Out-of-order (<1h)          | Buffer reorders, watermark timer catches missed pairs           | Correct delta, possibly delayed       |
| 3   | First record (new meter)    | `lastEmittedTs == Long.MinValue` -> absorb as baseline          | No output (by design)                 |
| 4   | Identity not found          | Written to raw, routed to DEAD_LETTER                           | Recoverable via mapping + Glue        |
| 5   | Malformed message           | PARSE_ERROR, nothing written                                    | Fix source                            |
| 6   | Negative delta              | Suppressed, ANOMALY side output, value stored as new baseline   | Gap in enriched data                  |
| 7   | Late (1-6h)                 | Predecessor still in buffer, delta computed normally            | Correct, delayed                      |
| 8   | Late (>6h)                  | Predecessor purged, LATE_ARRIVAL -> Lambda -> Glue              | Auto-corrected asynchronously         |
| 9   | Massive backfill            | Mostly in-stream; buffer purging races -> LATE_ARRIVAL for gaps | Glue fills gaps                       |
| 10  | Duplicates                  | Counter: same-timestamp overwrites in buffer. Gauge: duplicated | Consumer-side dedup                   |
| 11  | Identity change             | CDC updates broadcast state, new records use new hierarchy      | No retroactive correction (by design) |
| 12  | Identity deleted            | Subsequent records -> DEAD_LETTER                               | Re-add mapping + Glue                 |
| 13  | Job restart                 | Checkpoint recovery, possible limited duplicates                | No data loss                          |
| 14  | Source idle (>24h)          | Idleness timeout, watermark unblocked                           | Resumes normally                      |
| 15  | Backfill on new mapping     | DDB Stream INSERT -> Lambda -> Glue backfills `raw_data` gap    | Auto-backfilled, zero manual work     |
| 16  | Timestamp resampling           | Per-meter rule: linear interpolation (gauge) per bin in `(prev_ts, current_ts]`; time-proportional split (counter) per bin whose window overlaps `[prev_ts, current_ts]` | `resample_timestamp`, `resample_value`, `resample_method` populated |
| 17  | Sensor gap fill             | Sensor offline → next reading triggers fan-out across missing bins (interpolated) | Multiple bin rows per delayed reading |

### 7.1 Scenario 15 — Backfill on New Meter Mapping

When a meter sends data before its identity mapping exists in DynamoDB, records land in `raw_data` (via the unconditional raw sink) but are routed to `DEAD_LETTER` at the enrichment stage. Once the mapping is added, only new records are enriched by the streaming pipeline — a historical gap remains.

**Automatic recovery flow:**

```
DDB Streams fires INSERT event on new mapping
  -> backfill-trigger Lambda receives the event
     extracts daq_id from the new mapping
     uses DDB write timestamp as time_range_end cutoff
  -> starts Glue job scoped to that daq_id
     reads raw_data up to cutoff
     enriches with the new mapping
     writes corrected records to logical_sensor_data
```

**Key details:**

- Cutoff = DDB event timestamp prevents overlap with Flink streaming (Flink handles everything after the mapping exists)
- Duplicate check: Glue job is skipped if one is already running for the same `daq_id`
- Zero manual intervention required — the entire flow is event-driven

### 7.2 Scenario 16 — Timestamp Resampling

Per-meter alignment to fixed bin boundaries (typically 5, 15, or 60 minutes). Configured via an optional `resample_minutes` column (integer, minutes) on the `meter-identity` DynamoDB table. Implemented in `ResampleFunction` as a one-reading-lag operator: a meter's previous reading is held in keyed state until the next arrives, then per-bin rows are emitted.

**Gauge meters:** Bins `B` in `(prev_ts, current_ts]`. Each emitted bin uses linear interpolation between `(prev_ts, prev_value)` and `(current_ts, current_value)` evaluated at `B`. `resample_method = "linear_interpolation"`.

**Counter meters:** Bins whose **window** `[B − binSize, B]` overlaps the reading's period `[prev_ts, current_ts]`. Each bin's `resample_value = delta × overlap / period`, where `overlap = min(current_ts, B) − max(prev_ts, B − binSize)`. Energy is conserved per reading: `sum(resample_value)` across all bins of one reading equals the reading's `value` (delta). Energy is also conserved per bin across readings: when a reading boundary falls inside a bin window, that bin gets contributions from both the reading before and the reading after. **Consumers must SUM `resample_value` per `(logical_id, resample_timestamp)` after dedup on `(logical_id, timestamp, resample_timestamp)`.** `resample_method = "time_proportional"`.

**`resample_minutes IS NULL`:** Backward-compatible passthrough — gauges emit immediately, counters compute deltas as before, and `bin_*` columns are NULL.

**Unit normalization:** Both `value` and `resample_value` are scaled by the unit factor (e.g., raw `1` reported as `"Energy (100 Wh)"` → `100 Wh`). Glue and Flink share the same conversion table — output is bit-identical for the same input.

Full rules and edge cases: `docs/superpowers/specs/2026-05-01-resampling-rules-design.md`.

### 7.3 Scenario 17 — Sensor Gap Fill

When a sensor is offline and resumes, its next reading triggers fan-out across all bins covering the gap. For a 1-hour gap with 15-minute bins, gauge readings emit four rows (one per bin boundary in the period, each linearly interpolated). Counter readings emit one row per bin window the period overlaps (typically 4–5, depending on alignment) with time-proportional splits of the cumulative delta. If the sensor's buffered messages eventually arrive late (>buffer retention), they route to `LATE_ARRIVAL` and Glue recomputes the affected bins, appending corrected rows with newer `ingested_time` — consumer dedup picks the corrections automatically.

---

## 8. Organization & Access Control (OAC) — Detail

This section expands on the OAC Bounded Context (section 2.3). The OAC owns all hierarchy, user, and permission entities. EMS (section 2.4) is a pure consumer of OAC data for visualization purposes.

### 8.1 User Entity

```
User := {
  email:             Email
  id:                UserId ("U#{email}")
  name:              String
  profile:           Profile (Admin | Writer | Reader)
  language:          Language
  currency:          Currency
  hierarchy_access:  NodePermission     -- starting node + permission level
  created:           DateTime<UTC>
}
```

### 8.2 Permission Graph

User-to-node permissions are modeled as **edges** in a bipartite graph `G_p = (U, V, E_p)`:

```
EdgePermission := {
  user_id:    String          -- "U#{email}"
  node_id:    String          -- prefixed node ID ("C#1001")
  node_name:  String          -- denormalized display name
  permission: Permission      -- READ | WRITE | ADMIN | BLOCKED  ? We obviously need more detailed permissions (Cedar ?)
  created:    DateTime<UTC>
}
```

Permission traversal uses precedence: `Root(5) > Partner(4) > Company(3) > Property(2) > Building(1)`. The highest accessible entity determines the user's scope. Blocked nodes are subtracted from the accessible subtree.

---

## 9. Infrastructure

### 9.1 Technology Stack

| Component         | Technology                                          | Purpose                                         |
| ----------------- | --------------------------------------------------- | ----------------------------------------------- |
| Stream Processing | Apache Flink 1.20 (Scala 3) on Amazon Managed Flink | Real-time enrichment & delta computation        |
| Raw Storage       | Apache Iceberg on S3 Tables                         | Append-only raw sensor data                     |
| Curated Storage   | Apache Iceberg on S3 Tables                         | Enriched, delta-computed meter data             |
| Identity Store    | DynamoDB (PAY_PER_REQUEST, Streams enabled)         | Meter-to-logical mapping, hierarchy definitions |
| Error Transport   | Amazon Kinesis                                      | Error record delivery to recovery systems       |
| Batch Recovery    | AWS Glue (Spark)                                    | Late arrival recomputation                      |
| Recovery Trigger  | AWS Lambda                                          | Groups late arrivals, triggers Glue jobs        |
| User Auth (OAC)   | Amazon Cognito                                      | Authentication and user lifecycle               |
| Backend API (OAC) | Lambda                                              | Hierarchy CRUD, user management, permissions    |
| Frontend (EMS)    | Astro + HTMX + HyperScript                          | Dashboard, hierarchy navigation, visualization  |

### 9.2 Data Stores

| Table                | Engine              | Key Design                                           | Purpose                                  |
| -------------------- | ------------------- | ---------------------------------------------------- | ---------------------------------------- |
| `raw_data`           | Iceberg (S3 Tables) | Append-only, no partitioning                         | All parsed sensor readings               |
| `logical_sensor_data` | Iceberg (S3 Tables) | Append-only, event-sourced                           | Enriched readings with deltas            |
| `meter-identity`     | DynamoDB            | PK=hash bucket, SK=daqId, Streams=NEW_AND_OLD_IMAGES | Physical-to-logical mapping              |
| `hierarchy`          | DynamoDB            | PK=prefixed node ID, SK=path                         | Building element tree + user permissions |

### 9.3 Scale Parameters

- Active meters: ~2 million
- DynamoDB meter-identity: ~2M items, ~300MB, 20K partitions
- Bootstrap scan: ~20K RCU burst
- Electrocom loggers: 4,814 handling 23,685 meters
- Manual meters: ~14,255 (1,656 inactive), ~700 for climate accounting

---

## 10. Key Design Decisions

| Decision                                            | Rationale                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| --------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Append-only / event-sourced**                     | No updates or deletes in Iceberg tables. Corrections are new events with later `created` timestamps. Consumers use `MAX(created)` semantics. Iceberg time-travel provides audit trail.                                                                                                                                                                                                                                                                                                                                                                                          |
| **Delta computation isolates counter resets**       | When a counter resets (e.g., 99995 -> 5), the negative delta is emitted as ANOMALY and the new value becomes the baseline. All subsequent deltas are immediately correct. The reset costs exactly one data point — the reset moment — which is acceptable because counter data is interpolated at aggregation boundaries, so a single missing delta is absorbed by the interpolation without visible impact on hourly/daily aggregations.                                                                                                                                       |
| **Negative consumption suppressed**                 | A negative delta could be a counter reset, meter replacement, or data corruption — impossible to distinguish automatically. Route to ANOMALY error stream for investigation. The new value is stored as baseline so subsequent readings produce correct deltas immediately.                                                                                                                                                                                                                                                                                                     |
| **First counter reading silently absorbed**         | Need two points for a delta. Not an error — by design.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| **No retroactive re-enrichment on identity change** | The meter _was_ at the old location at that time. Historical data reflects historical truth. However, retroactive path changes can be supported via **bitemporality**: adding an `active_from` column that normally equals `ingested_time` but can be set to an earlier timestamp. Resolution then uses two dimensions — `(event_timestamp, active_from)` — where a newer `active_from` for the same event timestamp overrides an earlier row. This preserves the full audit trail while allowing hierarchy corrections to be applied retroactively without updates or deletes. |
| **Homogeneous hierarchy nodes**                     | All node types share the same `HierarchyNode` entity; they differ only by `MetaData` variant. Simplifies tree operations and storage.                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| **Materialized paths**                              | Parent field stores full path (`H#root#P#1#C#2`). Enables prefix-based subtree queries in DynamoDB without recursive lookups.                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| **6-hour buffer retention**                         | Balances memory usage against late-arrival tolerance. Records beyond 6h are recovered via batch (Glue).                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |

---

## 11. Glossary Cross-Reference (EN / DA)

| English           | Danish                   | DDD Role                                         |
| ----------------- | ------------------------ | ------------------------------------------------ |
| Partner           | Administrator / Afdeling | Hierarchy Node (level 1)                         |
| Company           | Firma                    | Hierarchy Node (level 2)                         |
| Property          | Ejendom                  | Hierarchy Node (level 3)                         |
| Group             | Gruppering               | Hierarchy Node (level 3)                         |
| Building          | Bygning                  | Hierarchy Node (level 4, leaf for structure)     |
| Area              | Omrade                   | Hierarchy Node (level 5, leaf)                   |
| Meter             | Maler                    | Domain Entity (attaches to Building/Area)        |
| Sub-meter         | Bimaler                  | Domain Entity (child in meter forest)            |
| Main meter        | Hovedmaler               | Domain Entity (root in meter forest)             |
| Counter           | Taeller                  | Value Object (sensor channel within meter)       |
| Meter type        | Malertype                | Value Object (gauge / counter)                   |
| Energy purpose    | Energiform               | Value Object (heat, water, electricity, etc.)    |
| Summation meter   | Summeringsmaler          | Domain Concept (aggregate participation)         |
| Part of summation | Del af summering         | Flag on meter entity                             |
| Degree day        | Graddage                 | Domain Concept (temperature normalization)       |
| Base load         | Grundlast                | Domain Concept (temperature-independent minimum) |
| DAQ ID            | DAQ ID                   | Value Object (physical sensor identifier)        |
| Logical meter     | Logisk maler             | Aggregate Root (maps physical to logical)        |
| Consumption       | Forbrug                  | Derived Value (delta of counter readings)        |

---

## 12. Open Items / Future Work

- [ ] Pull API integrations: CenterDanmark, Danfoss, Entelios, Lichtwart, AalborgForsyning, Brunata, Techem
- [ ] CSV file import pipeline
- [ ] Dead letter replay mechanism
- [ ] Alarm system integration (daily, hourly, cumulative, budget, cooling alarms)
- [ ] Energy price and CO2 factor time series
- [ ] Weather data integration for degree-day correction
- [ ] Iceberg table partitioning strategy for query performance
- [ ] Scheduled Glue jobs for pre-aggregated query tables
- [ ] QuickSight data source configuration
- [ ] Meter hierarchy CRUD in EMS (attach/detach sub-meters, summation flag management)
- [ ] Resampling and interpolation on-demand vs. pre-computed tradeoff
- [ ] **Event subscription streams:** Requirements doc mentions subscribing to sensor data and logical sensor data change events. Could be implemented as additional Kinesis sinks for raw and/or enriched data (straightforward to add in Flink). Need to define the use cases first — e.g., real-time dashboards, alarm triggers, external integrations — before deciding which streams and what filtering.
- [ ] **"Live Sensor Data" definition:** Terminology doc defines it as "most recent reading, no older than 30 days (?)" with an unsettled threshold. No clear need for pipeline enforcement — likely a query-time/frontend concern if needed at all. Revisit if staleness detection or alerting on silent sensors becomes a requirement.
- [ ] **Data Provider as domain entity:** The terminology distinguishes "Data Provider" (contractual entity — Datahub, Brunata, Enity) from "Data Source" (device/API). Currently not modeled. Needs decision: is it a domain entity in OAC (with agreement details, allowed volumes, contact info) or purely operational metadata outside the system? Affects per-provider rate limiting and billing.
- [ ] **Per-sensor pause:** Ability to pause a sensor producing many errors or quarantined readings. Flag likely in `meter-identity` DynamoDB (already broadcast to Flink via CDC). Affects both raw and enriched — paused sensors are dropped before `raw_data` write, so the check must happen post-parse, pre-sink, using the broadcast state. Needs: pause/unpause API, automatic pause threshold logic (or manual-only?), and a way to backfill the gap after unpausing.
- [ ] **Re-ingestion / replay:** Two cases: (a) Reprocess from existing `raw_data` — handled by current Glue recomputation job. (b) Replay from Kinesis after a processor bug fix — Flink Kinesis consumer supports `INITIAL_POSITION = AT_TIMESTAMP` to start from a specific point, so a separate Flink job instance can be launched with a limited time horizon to reprocess a window. For pull APIs where data was never fetched, the API poller needs a manual re-trigger with a time range. Needs operational tooling.
- [ ] **Per-provider rate limiting / volume monitoring:** The pipeline handles any volume, but we may want warnings when a customer/data provider exceeds agreed-upon payload amounts or frequency (for billing or to request they throttle). Options: (a) IoT Core rules if they can filter at that level — simplest, (b) Flink-side monitoring with per-provider counters and alert thresholds if not. Needs investigation into IoT Core rule capabilities.
- [ ] **Does 0 vale gauge readings ever make sense** This came up in issues with the way Entelios handles summer/wither time
- [ ] **Physically separated user data** We quite often experience customer tender requirements that mandates separation of customer data. In S3 Table Bucket there should not be any issues with this except maybe excessive writes of small files
- [ ] **Multi region support** We more and more often hear about customers wanting data closer to home. With S3 Table Buckets and the separation above of data, that should not be a massive issue, however ve need to test if multiple data pipelines are needed or if we can just regionalize data storage.
- [ ] **Counter numbering scheme (1, 2, 3, 10, 11) — still fit for purpose?** The current model assumes a fixed set of counter slots per meter (Counter 1–3 for consumption, 10–11 for temperatures). In practice many meter types expose more sensors than this (e.g., power, reactive energy, flow rate, pressure, humidity). The rigid numbering forces either ignoring those readings or shoehorning them into the wrong slot. Evaluate whether the counter model should be replaced with a dynamic list of named sensors per meter, each with its own type (counter/gauge), unit, and DAQ ID mapping. This affects meter-identity schema, the enrichment stage, and the logical_sensor_data table layout.

---

_This is a living document. Sections will be expanded as design decisions are finalized and implementation progresses._
