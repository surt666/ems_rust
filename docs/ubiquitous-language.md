# Ubiquitous Language — English ⇄ Dansk

A starting point for defining the language of this system. Every row is taken from
something that already exists: a type, a `translations/*.json` key, a `.feature`
file, a design spec, or Energihåndbogen 2019. Nothing here is invented vocabulary
except where explicitly marked as a **proposal**.

## How to read this

| Column | Means |
|---|---|
| **English** | The term as used (or proposed) in code, specs and English UI |
| **Dansk** | The Danish term as used (or proposed) in the UI and by the trade |
| **Means** | The concept — the thing both words point at |
| **Where** | Where it lives today |
| **St.** | Status, see legend |

**Status legend**

| | |
|---|---|
| ✅ | **Settled.** Code, UI and handbook agree. Use it, don't re-litigate. |
| ⚠️ | **Conflict.** Two or more words compete for one concept, or one word covers two concepts. Listed again in [Conflicts to resolve](#conflicts-to-resolve). |
| 🔤 | **Don't translate.** The English word is the Danish word in this trade. |
| 🆕 | **Proposal.** No Danish exists yet in code or UI; this is a suggestion, not a decision. |
| 📕 | **Handbook term.** Established Danish in Energihåndbogen 2019 that the domain borrows or should borrow. |


---

## 1. Hierarchy and structure

| English | Dansk | Means | Where | St. |
|---|---|---|---|---|
| Node | Node | A point in the hierarchy. Everything is a node — partner, company, building, area. | `domain::node::Node` | 🔤 |
| Hierarchy | Hierarki | The tree of nodes. One tree, rooted at `HN0#root`. | `logic::hierarchy` | ✅ |
| Hierarchy schema | Hierarki-skema | The per-company type graph: which node types may contain which. | `node.schema` / `domain::schema` | ✅ |
| Level | Niveau | Depth in the tree, `Hn0`–`Hn9`. Structural, not semantic. | `domain::ids::Level` | ✅ |
| Path | Sti | Pipe-separated ancestor ids, root to self: `HN0#root\|HN1#10001\|HN2#997`. | `node.path` | ✅ |
| Child / children | Barn / Børn | Nodes directly beneath a node. | `node.children` | ✅ |
| Parent | Forælder | The node directly above. | `node.parent` | 🆕 |
| Node type | Nodetype | What a node *is*: partner, company, building… Stored in the `label` field. | `node.label` | ⚠️ |
| Root | Rod | The single global root, `HN0#root`. Not a customer concept. | `NodeId::Root` | ✅ |
| Partner | Partner | `Hn1`. The reseller/operator above a company. | `logic::hierarchy` | ✅ |
| Company | Virksomhed | `Hn2`. The tenant boundary — schemas, sensors and formulas are company-scoped. | `logic::hierarchy` | ✅ |
| Property | Ejendom | A site/estate. Schema-governed node type below company. | schema `allowed_children` | ✅ |
| Building | Bygning | A building. | `common.building` | ✅ |
| Area | Område | A sub-part of a building. **Not** floor area. | `nav.areas` | ⚠️ |
| Floor | Etage | A storey. | schema type | ✅ |
| Floor area | Areal | m². The measured surface. Distinct from *Område*. | metadata field | ⚠️ |
| Edge | Kant | A directed relation between nodes: `has_label`, `has_sensor`, `reads`, `writes`, `administrates`, `blocked`. | `domain::values::EdgeKind` | 🆕 |
| Metadata | Metadata | Schema-validated JSON on a node. | `node.metadata` | ✅ |
| Master data | Stamdata | The setup surface where nodes and their metadata are maintained. | `nav.master_data` | ✅ |
| Context | Kontekst | The currently selected node — everything on screen is scoped to it. | `.feature` files | ✅ |

## 2. Sensors and readings

| English | Dansk | Means | Where | St. |
|---|---|---|---|---|
| Sensor | Sensor | **One measuring channel.** The system's input. One per device channel — a 3-phase meter is 3 sensors. | `domain::sensor::Sensor` | ⚠️ |
| Meter | Måler | A **physical device**. Exists in the hierarchy only as a node type. Not a sensor. | node type / `nav.meters` | ⚠️ |
| Main meter | Hovedmåler | The utility's meter for a supply. | `standby.main_meters` | ✅📕 |
| Sub-meter | Bimåler | A meter on a branch beneath a main meter. Energihåndbogen: required on DHW supply above 10.000 kWh/year. | `standby.sub_meters` | ✅📕 |
| Reading | Aflæsning | One value at one timestamp, as received. | `measurements.reading` | ⚠️ |
| Reading kind | Aflæsningstype | *How* a sensor's values accumulate — counter or gauge. Not what it measures. | `domain::values::ReadingKind` | 🆕 |
| Counter | Tæller / tællerstand | An odometer. The value only grows; consumption is the difference between two readings. | `ReadingKind::Counter` | 🆕 |
| Gauge | Øjebliksværdi | An instantaneous value: power, temperature, flow. | `ReadingKind::Gauge` | 🆕 |
| Unit | Enhed | kWh, m³, °C. | `sensor.unit` | ✅ |
| Sampling interval | Måleinterval | The interval a sensor's readings are resampled to. | `sensor.resample` / `resample_minutes` | ✅ |
| Data acquisition | Datatilegnelse | The raw-readings view for one sensor. | `measurements.title` | ✅ |
| Raw meter reading | Rå måleraflæsning | An untouched reading as ingested, before resampling. | `measurements.subtitle` | ✅ |
| Ingested | Indlæst | When the platform received the reading. Later versions of the same timestamp win. | `measurements.ingested` | ✅ |
| DAQ id | DAQ id | The device-channel identity from the acquisition layer. | `sensor.daq_id` | 🔤 |
| Logical id | Logisk id | The enriched data identity for a sensor's series. | `measurements.logical_id` | ⚠️ |
| Gateway | Gateway | The device that relays readings to the platform. | `nav.gateways` | 🔤 |
| Tag | Tag | Free-form sensor label used for filtering. | `sensor.menu.tags` | 🔤 |

## 3. Energy type and purpose — the two axes

The whole model rests on these being **independent**: electricity serves lighting,
cooling and ventilation alike; space heating can arrive as district heating, gas or
a heat pump.

### 3.1 Energy type — *what is measured*

| English | Dansk | Means | Where | St. |
|---|---|---|---|---|
| Energy type | Energiart | The resource a sensor measures. Legacy EMS calls this *Målertype*. | `domain::values::EnergyType` | ⚠️ |
| Electricity | El | | `EnergyType::Electricity` | ✅ |
| District heating | Fjernvarme | Heat delivered from a district network. | `EnergyType::DistrictHeating` | ✅📕 |
| District cooling | Fjernkøling | Cooling delivered from a district network. | `EnergyType::DistrictCooling` | ✅ |
| Gas | Gas | Natural gas. Handbook: *naturgas*. | `EnergyType::Gas` | ✅📕 |
| Water | Vand | | `EnergyType::Water` | ✅ |
| Heat | Varme | Locally produced heat (boiler, heat pump). | `EnergyType::Heat` | ✅📕 |
| Dimension | Dimension | The unit family a type sums in. Derived, never stored. | `domain::values::Dimension` | 🆕 |
| Energy (dimension) | Energi | Sums in kWh: electricity, heat, district heating/cooling. | `Dimension::Energy` | ✅ |
| Volume (dimension) | Volumen | Sums in m³: gas, water. | `Dimension::Volume` | 🆕 |

### 3.2 Purpose — *what the energy is spent on*

The taxonomy follows Energihåndbogen 2019's chapters.

| English | Dansk | Means | Where | St. |
|---|---|---|---|---|
| Purpose | Formål | What the energy is spent on. | `domain::values::Purpose` | ✅ |
| Space heating | Rumvarme | Heating the rooms. | `Purpose::SpaceHeating` | ✅📕 |
| Domestic hot water (DHW) | Varmt brugsvand (VBV) | Tap-water heating and circulation. The handbook's most-used term (221 hits). | `Purpose::Dhw` | ✅📕 |
| Ventilation | Ventilation | | `Purpose::Ventilation` | ✅📕 |
| Cooling | Køling | Comfort/process cooling. **Not** *afkøling*. | `Purpose::Cooling` | ⚠️📕 |
| Lighting | Belysning | | `Purpose::Lighting` | ✅📕 |
| Plug loads | Apparater | Appliances on socket outlets. | `Purpose::PlugLoads` | ⚠️ |
| EV charging | Elbilopladning | Charge points. | `Purpose::EvCharging` | 🆕 |
| Process | Proces | Production/process energy. | `Purpose::Process` | 🆕 |
| Common | Fællesforbrug | Shared/common-area consumption. | `Purpose::Common` | ⚠️ |
| Generation | Egenproduktion | Own production, e.g. PV. Reported, but never reduces *Ikke fordelt* — exported energy is not a slice of consumption. | `Purpose::Generation` | ✅📕 |
| Total | I alt | The node's own value. Declarable; defaults to Σ children + own sensors. | `Purpose::Total` | ✅ |
| Unallocated | Ikke fordelt | *I alt* − Σ(allocating purposes). Derived by the roll-up, never declared. | `Purpose::Unallocated` | ✅ |

## 4. Formulas and roll-up

| English | Dansk | Means | Where | St. |
|---|---|---|---|---|
| Node formula | Nodeformel | A node's declared output for one `(energitype, formål)`. | `domain::node_formula::NodeFormula` | 🆕 |
| Formula | Formel | | `formula.edit_title` | ✅ |
| Term | Led | One weighted reference: a sensor or a child node × a number. | `formula.add_term` | ✅ |
| Coefficient | Koefficient | The multiplier on a term. `1` includes, `0` excludes, `−1` subtracts, `0,28` takes 28 %. | `Term.coefficient` | 🆕 |
| Reference | Reference | What a term points at — a sensor anywhere in the company, or a **direct child** node. | `node_formula::Reference` | 🆕 |
| Default | Standard | No formula ⇒ the node's value is the sum of everything beneath it. | `formula.help_default` | ✅ |
| Roll-up | Opsummering | The hourly job that sums consumption per node/energitype/formål/period. | `measurements_aggregate` | 🆕 |
| Coefficient matrix | Koefficientmatrix | The flattened formulas. Ancestry and defaults are baked in, so the job is one join and one grouped sum. | `logic::formulas::flatten` | 🆕 |
| Granularity | Opløsning | Hour or day. | `common.resolution` | ✅ |

**The sub-meter pattern.** Energihåndbogen's *bimåler* rule is the reason coefficients
exist: a branch metered separately is subtracted from the main so it is not counted
twice — `hovedmåler × 1` + `bimåler × −1`.

## 5. Energy technology — Energihåndbogen vocabulary

Terms the trade already owns. Where a concept appears in our formulas, the handbook's
Danish is the authority, not a translation of our English.

| English | Dansk | Means | St. |
|---|---|---|---|
| Efficiency | Virkningsgrad | Output ÷ input. 145 hits — the handbook's central number. | 📕 |
| Annual efficiency | Årsvirkningsgrad / årsnyttevirkning | Efficiency over a full year, e.g. 95–98 % for district heating vs. far lower for oil boilers. | 📕 |
| Calorific value | Brændværdi | Energy per unit fuel. *Nedre* (lower) and *øvre* (upper) are different numbers — the handbook always says which. | 📕 |
| Coefficient of performance | COP | Heat delivered ÷ electricity used, at a point. | 🔤📕 |
| Seasonal COP | SCOP / sæsoneffektfaktor | Year-round average COP incl. seasonal variation. A SCOP of 3,95 means 3,95× the electrical energy is delivered as heat. | 📕 |
| Degree days | Graddage | Σ(17 °C − daily mean outdoor temperature). Normalises consumption between a month and a normal month. 102 hits. | 📕 |
| Flow temperature | Fremløbstemperatur | Supply-side temperature. | 📕 |
| Return temperature | Returtemperatur | Return-side temperature. | 📕 |
| Cooling (ΔT performance) | Afkøling | How far the water is cooled between flow and return. A district-heating **performance** measure. **Not** *køling*. | ⚠️📕 |
| Heat pump | Varmepumpe | | 📕 |
| Boiler | Kedel | Gas-, oil- or biomass-fired. | 📕 |
| Heat exchanger | Veksler / varmeveksler | | 📕 |
| Hot-water cylinder | Varmtvandsbeholder | | 📕 |
| Building services | Bygningsinstallationer | The technical installations in a building. | 📕 |
| Building automation | Bygningsautomatik | Control systems. | 📕 |
| Commissioning test | Funktionsafprøvning | Statutory functional test of an installation. 149 hits. | 📕 |
| Balancing | Indregulering | Adjusting a system to its design flows. | 📕 |
| Heat loss | Varmetab | | 📕 |
| Insulation | Isolering | | 📕 |
| Energy saving | Energibesparelse | | 📕 |
| Consumption | Forbrug | | ✅📕 |
| Power / capacity | Effekt | kW. Distinct from *energi* (kWh). | 📕 |
| Building regulations | Bygningsreglementet | The Danish building code (BR). | 📕 |
| Indoor climate | Indeklima | | 📕 |

## 6. Derived read models

Computed at query time from the roll-up. None of these are stored.

| English | Dansk | Means | Where | St. |
|---|---|---|---|---|
| Cost | Omkostning | Consumption × tariff. | `get_cost` | ✅ |
| Tariff | Tarif | Price per unit. | `get_cost` | 🆕 |
| Extra cost | Meromkostning | The cost of a deviation. | `alarms.extra_cost` | ✅ |
| Emissions | Emissioner | CO₂e from consumption × emission factor. | `get_emissions` | 🆕 |
| Emission factor | Emissionsfaktor | kg CO₂e per unit. | `nav.emission_factors` | ✅ |
| Climate accounting | Klimaregnskab | The GHG report — Scope 1/2/3. | `nav.climate` | ✅ |
| CSRD report | CSRD-rapport | | `nav.csrd` | 🔤 |
| Location-based / market-based | Lokationsbaseret / markedsbaseret | The two GHG accounting bases. | `klimaregnskab.feature` | 🆕 |
| Benchmark | Benchmark | A node compared against its peers. | `get_benchmark` | 🔤 |
| Building benchmark | Bygningsbenchmark | | `nav.building_benchmark` | ✅ |
| Alarm | Alarm | A detected deviation. | `get_alarms` | ✅ |
| Acknowledge | Kvittere | Marking an alarm as seen. | `alarms.acknowledge` | ✅ |
| Unacknowledged | Ukvitteret | | `alarms.unacknowledged` | ✅ |
| Deviation | Afvigelse | | `alarms.deviation` | ✅ |
| Alarm recipient | Alarmmodtager | | `alarms.recipients` | ✅ |
| Liveness | Liveness | Whether a device is still reporting. | `get_liveness` | 🆕 |
| Purpose split | Formålsopdeling | Consumption broken down by *formål*. The handbook's own framing. | `get_purpose_split` | 🆕📕 |
| Standby | Standby | Consumption outside operating hours. | `nav.standby_analysis` | 🔤 |
| Operation | Drift | Consumption during operating hours. | `standby.operation` | ✅ |
| Standby share | Standby andel | Standby ÷ total. | `standby.standby_share` | ✅ |
| Building usage | Bygningsanvendelse | The building's use class — drives peer grouping. | `standby.building_usage` | ✅ |

## 7. UI surface

The words users actually see. Already agreed in `translations/`.

| English | Dansk | Where |
|---|---|---|
| Overview | Oversigt | `nav.overview` |
| Statistics | Statistik | `nav.statistics` |
| Analysis | Analyse | `nav.analysis` |
| Monitoring | Overvågning | `nav.monitoring` |
| Meters | Målere | `nav.meters` ⚠️ |
| Reporting | Rapportering | `nav.reporting` |
| Setup | Opsætning | `nav.setup` |
| Active control | Aktiv styring | `nav.active_control` |
| Energy model | Energimodel | `nav.ri_energy_model` |
| Custom reports | Brugertilpassede rapporter | `nav.custom_reports` |
| Export consumption data | Eksporter forbrugsdata | `nav.export_data` |
| User list | Brugerliste | `nav.user_list` |
| Period | Periode | `common.period` |
| Resolution | Opløsning | `common.resolution` |
| Filters | Filtre | `common.filters` |
| Row count | Antal rækker | `measurements.rows` |
| Load more | Hent flere | `measurements.load_more` |

**Access.** `Profile` (Developer, Standard, Technician, Reader, SysAdm) and the Cognito
groups (Reader/Writer/Admin) are English-only identity concepts — no Danish is owed.
*Bruger* = user, *Adgangskode* = password, *Log ind* = sign in.

---

## Conflicts to resolve

Ten places where the language is not yet one language. Each needs a decision, not a
translation.

### C1 — Sensor vs. Måler
The domain settled on **Sensor** ("one per device channel"; a meter is a physical
device and appears only as a node type). The Danish UI says **Målere** everywhere —
nav, statistics, alarms, standby — and `formula.help_terms` even glosses a term as
*"en måler eller en underliggende node"*. Either the UI moves to *Sensor* (already used
in `node.sensors` = "Sensorer"), or *Måler* is accepted as the Danish word for sensor
and the split is English-only. Today both are true in different files.

### C2 — Energitype vs. Energiform vs. Målertype
Three Danish words for `EnergyType`:
- `formula.gloss_energy_type` → **Energitype** (what the rollup spec fixes)
- `common.energy_form` → **Energiform** (EN side says "Energy type")
- `alarms.meter_type` → **Målertype** (EN side says "Meter type"; the legacy EMS name, and what `EnergyType`'s own doc-comment cites)

`Energitype` is the one the design spec commits to. The other two should follow or be
retired.

### C3 — "Målertype" names the wrong axis
Worse than a synonym: *Målertype* literally reads as "kind of meter", which is exactly
what `ReadingKind` (counter/gauge) is — but it denotes `EnergyType`. Anyone reading the
alarms screen and then the code will get these backwards. Renaming this label is
probably the single highest-value fix in the list.

### C4 — Afkøling vs. Køling
`nav.cooling` maps **Afkøling → "Cooling"**. But *afkøling* is the district-heating ΔT
performance measure (how far the return water is cooled), while *køling* is
`Purpose::Cooling`, comfort cooling. Same English word, two unrelated concepts; the
alarm type *Afkølingsalarm* is about the former. English needs two words —
"return-temperature performance" / "cooling" — or Danish needs the nav item renamed.

### C5 — Reading: value vs. accumulation
*Aflæsning* is one value at one timestamp (`measurements.reading`). `ReadingKind` is
how a sensor's values accumulate. Both are "reading" in English. Candidates for the
second: *Aflæsningstype*, *Måletype*, *Akkumuleringstype*.

### C6 — Node type is stored in a field called `label`
`node.label` holds the node **type** ("building", "company"). Meanwhile `common.label`
= *Etiket* is an unrelated UI concept, and `EdgeKind::HasLabel` uses "label" for the
same type-tagging. Three uses of one word. The domain concept wants to be *Nodetype*.

### C7 — Område vs. Areal
Both are "area" in English. *Område* is a hierarchy node type (`nav.areas`); *areal* is
m² of floor space and drives every intensity figure (kWh/m²). Keeping them apart in
Danish is easy; the English side is the risk.

### C8 — "Logisk måler-id" for a per-sensor identity
`logical_id` identifies one sensor's processed series, but the UI calls it *Logisk
måler-id*. Inherits C1: if a sensor is not a meter, this label is wrong. *Logisk
sensor-id* or just *Logisk id*.

### C9 — Purposes without settled Danish
`Purpose::PlugLoads` and `Purpose::Common` have no Danish anywhere in the codebase.
Proposals: *Apparater* (or *Stikkontaktforbrug*) and *Fællesforbrug* (or
*Fællesarealer*). `EvCharging` and `Process` are also unattested but read obviously as
*Elbilopladning* and *Proces*.

### C10 — "Measure" rendered as "Visning"
`common.measure` is the cost-vs-consumption toggle, but reads EN "View" / DA "Visning"
— which is the word for a *view*, not a *measure*. Suggest *Målestok* or, more plainly,
*Vis som*.

---

## Words we should not use

| Don't | Why | Use instead |
|---|---|---|
| Resource | Renamed to `EnergyType`. Named a thing, not what it describes. | Energy type / Energitype |
| SensorType | Would repeat `Resource`'s mistake — there is only one kind of sensor. | ReadingKind |
| Logical meter | No logical-meter concept exists. A sensor has a logical id; there is no logical meter. | Sensor / logical id |
| Main meter / sub-meter *as a sensor property* | The main/sub taxonomy belongs to formulas, not to sensors. Rejected twice in design. | A formula term with coefficient ±1 |
| Binning | Renamed 2026-06-07. | Resampling / måleinterval |
| Metric (as a third axis) | There are two axes only. Cost and CO₂ are derived at query time. | Energy type + purpose |

---

## Next steps

1. Decide **C1** and **C2/C3** first — they touch the most screens and are the ones most
   likely to mislead a reader of the code.
2. Fill the 🆕 rows with real Danish, ideally with someone from the trade.
3. Once agreed, this file becomes the source: `translations/*.json` should be checked
   against it, and new terms added here before they appear in code.
