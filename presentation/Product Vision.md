---
tags:
  - slides
  - ems
  - DAQ
theme: white
height: 1200
width: 1600
margin: 0
maxScale: 4
---
<!-- slide template="[[tpl-kc-title]]" -->
::: title1
EMS Product Vision

:::

::: title2
Steen Larsen
::: 

---
<!-- slide template="[[tpl-kc-fullpage]]" -->
::: title
Customers
:::

::: block
- EMS (Licencesd customers)
- Intelligence (ML, BI)
- Technicians
- Energy Advicers (BI, Reports etc. beyond what EMS does)
- Marketing (Users and what they do - users)
- Billing (Meters and where they are - hierarchy)
- External API users
:::

---
<!-- slide template="[[tpl-kc-fullpage]]" -->
::: title
Extensability and Simplicity
:::

::: block
- Consider everything as a graph
- A hierarchy is just a DAG
- What's in the different nodes are just decided by rules
- Only the main traversing mechanim needs to rigid
- Everything else is configurable, such as 
	- The node types allowed
	- Their internal relationship
	- What metadata is carried on each node
	- What metadata is mandatory (lat/lon f.i.)
- Access from a user just becomes entrypoint(s) into the graph
- Edges between nodes define access and rules
- Sensors are just leaf nodes in the graph
- A graph implementation on top of DynamoDb is fast and cheap
:::

---
<!-- slide template="[[tpl-kc-fullpage]]" -->
::: title
Graph illustration
:::

::: block
adminuser   ->       	hn0(root)
				|
partneradmin ->	     -- hn1(partner)
				|
companyadmin ->    -- hn2(company) contains schema definition, defaults to current group/property -> building -> area
				|
				-- hn3 - hn9

An admin user will normally have all permissions (administrates), but other user types will typically have labels with writes or reads instead of administrates

Writes means you can change metadata but not add children (only admin can do that)

Reads means you can read everything from your access level and down but not chanege anything

Blocked means a specific node in blocked (invisible) to you, f.i. a super secret lab in a company you besides that have access to

Permissions are inherited top down, unless explicitly overruled

If field level access is needed we can add a CEDAR rule definition, but seems overkill

:::

---

<!-- slide template="[[tpl-kc-fullpage]]" -->
::: title
The actual data pipeline
:::

::: block
- All data goes through the same data hose
- The same general error handling applies to all
- Specific API, Physical Meter and File error handling is handled in the indivdual parser, since they are based on specific rules
- Switch to pipeline slide and demo
:::

---

<!-- slide template="[[tpl-kc-fullpage]]" -->
::: title
Conclusion
:::

::: block
- Simplifying and thinking differently about data structures can give real benefits
	- In term of performance
	- In term of scalability
	- In term of availability
	- In term of regionality
	- In term of cost
- Number of true micro services can be drastically reduced (2-3 major and a few helper ones)
- All services can run as a Lambda, since no synchronous http call should ever take more than a few seconds
- Reports should run as async background jobs (report back over WebSocket or SSE if needed)
- All data enrichment and validation is handled in Flink or Spark
- All data querying should never be able to slow the system for another user
:::
