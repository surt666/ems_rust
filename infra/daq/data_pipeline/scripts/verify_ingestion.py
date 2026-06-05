#!/usr/bin/env python3
"""
Verify that historical counter data was ingested correctly.

Checks:
  1. raw_data: all records present with correct cumulative values
  2. logical_meter_data: delta records present with correct computed deltas
  3. Consistency: sum of deltas ≈ last_cumulative - first_cumulative

Usage:
  python verify_ingestion.py --daq-id "daq:std_json_v1:countertest:test_counter_001:volume"
  python verify_ingestion.py --meter-id "93ea16e6-fecc-42fa-ad56-ff3cc2b31f0c"
  python verify_ingestion.py --check-all
"""

import argparse
import sys

import boto3


def run_athena_query(client, query: str, database: str = "all",
                     output_location: str = "s3://aws-athena-query-results-891377204778-eu-central-1/") -> list[dict]:
    """Execute an Athena query and return results."""
    response = client.start_query_execution(
        QueryString=query,
        QueryExecutionContext={"Database": database, "Catalog": "s3tablescatalog/measurements"},
        ResultConfiguration={"OutputLocation": output_location},
    )
    execution_id = response["QueryExecutionId"]

    # Wait for completion
    import time
    while True:
        status = client.get_query_execution(QueryExecutionId=execution_id)
        state = status["QueryExecution"]["Status"]["State"]
        if state in ("SUCCEEDED", "FAILED", "CANCELLED"):
            break
        time.sleep(1)

    if state != "SUCCEEDED":
        reason = status["QueryExecution"]["Status"].get("StateChangeReason", "unknown")
        print(f"  Query failed: {state} — {reason}")
        return []

    # Fetch results
    results = client.get_query_results(QueryExecutionId=execution_id)
    columns = [col["Label"] for col in results["ResultSet"]["ResultSetMetadata"]["ColumnInfo"]]
    rows = []
    for row in results["ResultSet"]["Rows"][1:]:  # skip header
        values = [cell.get("VarCharValue", "") for cell in row["Data"]]
        rows.append(dict(zip(columns, values)))
    return rows


def check_raw_data(athena, daq_id: str):
    """Check raw_data for the given daq_id."""
    print(f"\n--- raw_data: {daq_id} ---")

    count_query = f"""
        SELECT COUNT(*) as cnt,
               MIN(timestamp) as min_ts,
               MAX(timestamp) as max_ts,
               MIN(value) as min_val,
               MAX(value) as max_val
        FROM "all"."raw_data"
        WHERE daq_id = '{daq_id}'
    """
    rows = run_athena_query(athena, count_query)
    if not rows or rows[0]["cnt"] == "0":
        print("  NO RECORDS FOUND")
        return False

    r = rows[0]
    print(f"  Records:   {r['cnt']}")
    print(f"  Time range: {r['min_ts']} → {r['max_ts']}")
    print(f"  Value range: {r['min_val']} → {r['max_val']}")

    # Check monotonicity (cumulative values should always increase)
    mono_query = f"""
        SELECT COUNT(*) as violations
        FROM (
            SELECT value,
                   LAG(value) OVER (ORDER BY timestamp) as prev_value
            FROM "all"."raw_data"
            WHERE daq_id = '{daq_id}'
        )
        WHERE prev_value IS NOT NULL AND value < prev_value
    """
    rows = run_athena_query(athena, mono_query)
    violations = int(rows[0]["violations"]) if rows else -1
    if violations == 0:
        print("  Monotonicity: OK (values always increasing)")
    else:
        print(f"  Monotonicity: FAILED ({violations} decreases found)")

    return True


def check_enriched(athena, logical_id: str):
    """Check logical_meter_data for the given logical_id."""
    print(f"\n--- logical_meter_data: {logical_id} ---")

    count_query = f"""
        SELECT COUNT(*) as cnt,
               MIN(timestamp) as min_ts,
               MAX(timestamp) as max_ts,
               MIN(value) as min_val,
               MAX(value) as max_val,
               SUM(value) as total_delta
        FROM (
            SELECT *, ROW_NUMBER() OVER (
                PARTITION BY logical_id, timestamp
                ORDER BY created DESC
            ) as rn
            FROM "all"."logical_meter_data"
            WHERE logical_id = '{logical_id}'
        )
        WHERE rn = 1
    """
    rows = run_athena_query(athena, count_query)
    if not rows or rows[0]["cnt"] == "0":
        print("  NO RECORDS FOUND (Glue job may not have run yet)")
        return False

    r = rows[0]
    print(f"  Records:     {r['cnt']}")
    print(f"  Time range:  {r['min_ts']} → {r['max_ts']}")
    print(f"  Delta range: {r['min_val']} → {r['max_val']}")
    print(f"  Total delta: {r['total_delta']}")

    # Check for negative deltas
    neg_query = f"""
        SELECT COUNT(*) as negatives
        FROM (
            SELECT *, ROW_NUMBER() OVER (
                PARTITION BY logical_id, timestamp
                ORDER BY created DESC
            ) as rn
            FROM "all"."logical_meter_data"
            WHERE logical_id = '{logical_id}'
        )
        WHERE rn = 1 AND value < 0
    """
    rows = run_athena_query(athena, neg_query)
    negatives = int(rows[0]["negatives"]) if rows else -1
    if negatives == 0:
        print("  Negative deltas: NONE (OK)")
    else:
        print(f"  Negative deltas: {negatives} FOUND (anomaly)")

    return True


def check_consistency(athena, daq_id: str, logical_id: str):
    """Verify that sum of deltas ≈ last_cumulative - first_cumulative."""
    print(f"\n--- Consistency check ---")

    raw_query = f"""
        SELECT MIN(value) as first_val, MAX(value) as last_val
        FROM "all"."raw_data"
        WHERE daq_id = '{daq_id}'
    """
    raw_rows = run_athena_query(athena, raw_query)

    enriched_query = f"""
        SELECT SUM(value) as total_delta
        FROM (
            SELECT *, ROW_NUMBER() OVER (
                PARTITION BY logical_id, timestamp
                ORDER BY created DESC
            ) as rn
            FROM "all"."logical_meter_data"
            WHERE logical_id = '{logical_id}'
        )
        WHERE rn = 1
    """
    enriched_rows = run_athena_query(athena, enriched_query)

    if not raw_rows or not enriched_rows:
        print("  Cannot verify — missing data")
        return

    first_val = float(raw_rows[0]["first_val"])
    last_val = float(raw_rows[0]["last_val"])
    expected_total = last_val - first_val
    actual_total = float(enriched_rows[0]["total_delta"]) if enriched_rows[0]["total_delta"] else 0

    diff = abs(expected_total - actual_total)
    pct = (diff / expected_total * 100) if expected_total > 0 else 0

    print(f"  Raw range:       {first_val} → {last_val}")
    print(f"  Expected total:  {expected_total:.3f} kWh")
    print(f"  Actual total:    {actual_total:.3f} kWh")
    print(f"  Difference:      {diff:.3f} kWh ({pct:.2f}%)")

    if pct < 0.01:
        print("  PASS")
    else:
        print("  MISMATCH — check for missing or duplicate records")


def check_glue_job_status():
    """Check if the Glue recomputation job has run."""
    print("\n--- Glue job status ---")
    glue = boto3.client("glue", region_name="eu-central-1")
    try:
        response = glue.get_job_runs(JobName="late-data-recomputation", MaxResults=5)
        runs = response.get("JobRuns", [])
        if not runs:
            print("  No job runs found (Lambda may not have triggered yet)")
        for run in runs:
            print(f"  {run['Id'][:12]}... | {run['JobRunState']} | "
                  f"started={run.get('StartedOn', 'N/A')} | "
                  f"args={run.get('Arguments', {}).get('--daq_ids', 'N/A')}")
    except Exception as e:
        print(f"  Error: {e}")


def main():
    parser = argparse.ArgumentParser(description="Verify historical counter data ingestion")
    parser.add_argument("--daq-id", default="daq:std_json_v1:countertest:test_counter_001:volume",
                        help="DAQ ID to check in raw_data")
    parser.add_argument("--logical-id", default="93ea16e6-fecc-42fa-ad56-ff3cc2b31f0c",
                        help="Logical meter UUID to check in enriched table")
    parser.add_argument("--region", default="eu-central-1")
    args = parser.parse_args()

    athena = boto3.client("athena", region_name=args.region)

    check_glue_job_status()
    raw_ok = check_raw_data(athena, args.daq_id)
    enriched_ok = check_enriched(athena, args.logical_id)

    if raw_ok and enriched_ok:
        check_consistency(athena, args.daq_id, args.logical_id)

    print()


if __name__ == "__main__":
    main()
