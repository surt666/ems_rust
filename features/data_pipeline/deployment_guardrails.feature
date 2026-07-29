Feature: Non-destructive deployment guardrails
  Several pipeline resources cannot be changed in place without data loss. These scenarios are
  guardrails for any agent or human deploying the DAQ pipeline: they encode the decisions that
  keep a deploy from silently replacing a table or losing Flink state. They are operational
  checks (the assertions are on cdk diff / run-configuration), not unit tests.

  Background:
    Given the DAQ pipeline owns Iceberg tables, a Flink (MSF) app and DynamoDB tables
    And every deploy is gated on "cdk diff" being non-destructive

  # source: resampling-rules-design spec — Iceberg schema migration via Athena
  Scenario: Adding columns to logical_data goes via Athena ALTER, not a CDK table replace
    Given new columns must be added to logical_data
    When the schema change is applied
    Then it is done with "ALTER TABLE ... ADD COLUMNS" in Athena
    And the S3TablesStack is NOT redeployed with changed columns (CfnTable replace = data loss)
    And the CDK schema is updated in the same physical column order to stay in sync

  # source: CLAUDE.md "Gotchas" — AWS::S3Tables::Table cannot be replaced in place
  Scenario: Renaming or clearing an Iceberg table column uses the two-step remove-then-restore deploy
    Given a column on an S3 Tables Iceberg table must be renamed or the table cleared
    When the change is deployed
    Then step 1 removes the table resource and deploys (CFN deletes it, clearing data)
    And step 2 restores the resource with new columns and deploys (CFN recreates it fresh)
    And sibling tables are untouched

  # source: resampling-rules-design spec / CLAUDE.md — operator uid & keyed-state stability
  Scenario: A normal Flink deploy restores from snapshot only if uid and state names are unchanged
    Given the Flink app is updated in place
    When the operator uid and keyed-state descriptor names are unchanged
    Then the app snapshots, restarts and restores from snapshot (brief pause only)

  # source: CLAUDE.md "Gotchas" — renaming uid/state forces non-restorable snapshot
  Scenario: Renaming a Flink operator uid or keyed state requires starting fresh
    Given the operator uid or keyed-state descriptor names change
    When the app is deployed
    Then the old snapshot cannot be restored
    And the app is started with AllowNonRestoredState under FlinkRunConfiguration
    And RESTORE_FROM_LATEST_SNAPSHOT is preferred over SKIP (SKIP reprocesses 24h and duplicates raw_data)

  # source: CLAUDE.md — cross-account field contract
  Scenario: resample_minutes is a strict contract with no binning fallback
    Given the bridge writes resample_minutes and the pipeline reads only resample_minutes
    When the attribute is renamed in any future change
    Then both the hierarchy side and the daq side deploy together
    And the affected meter-identity rows are re-written
    And a meter-identity row lacking resample_minutes simply gets raw passthrough (no resampling)
