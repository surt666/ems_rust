#!/usr/bin/env python3
"""Smoke tests: inject into real Kinesis, verify via Athena.

Usage:
    uv run test_smoke.py --input-stream DAQ_STREAM --error-stream ERROR_STREAM
    uv run test_smoke.py --input-stream DAQ_STREAM --error-stream ERROR_STREAM --verbose
"""

import argparse
import json
import logging
import time
import uuid
from datetime import datetime, timezone

import boto3

from smoke_report import SmokeReport, SmokeScenarioResult

logger = logging.getLogger(__name__)

REGION = "eu-central-1"
TEST_PREFIX = "test_smoke_"
SENSOR_IDENTITY_TABLE = "sensor-identity"
ATHENA_DATABASE = "s3tablescatalog"
ATHENA_OUTPUT = "s3://athena-results-891377204778-eu-central-1/"
CHECKPOINT_WAIT_S = 420    # 7 minutes (5-min checkpoint + buffer)


class SmokeTestRunner:
    def __init__(self, input_stream: str, error_stream: str, verbose: bool = False):
        self.kinesis = boto3.client("kinesis", region_name=REGION)
        self.athena = boto3.client("athena", region_name=REGION)
        self.dynamodb = boto3.resource("dynamodb", region_name=REGION)
        self.table = self.dynamodb.Table(SENSOR_IDENTITY_TABLE)
        self.input_stream = input_stream
        self.error_stream = error_stream
        self.verbose = verbose
        self.run_id = uuid.uuid4().hex[:8]

    def _put_kinesis(self, stream: str, data: dict, partition_key: str = "0"):
        self.kinesis.put_record(
            StreamName=stream,
            Data=json.dumps(data).encode("utf-8"),
            PartitionKey=partition_key,
        )

    def _put_mapping(self, daq_id: str, logical_id: str, meter_type: str,
                     partner_id: int = 1, company_id: int = 1):
        """Insert a test mapping into DynamoDB sensor-identity table."""
        self.table.put_item(Item={
            "daq_id": daq_id,
            "logical_id": logical_id,
            "meter_type": meter_type,
            "hierarchy_path": f"P{partner_id}#C{company_id}",
        })

    def _delete_mapping(self, daq_id: str):
        try:
            self.table.delete_item(Key={"daq_id": daq_id})
        except Exception as e:
            logger.warning("Failed to delete mapping %s: %s", daq_id, e)

    def _execute_athena(self, query: str, max_retries: int = 5):
        """Execute an Athena query and wait for completion. Returns execution_id."""
        execution = self.athena.start_query_execution(
            QueryString=query,
            QueryExecutionContext={"Database": ATHENA_DATABASE},
            ResultConfiguration={"OutputLocation": ATHENA_OUTPUT},
        )
        execution_id = execution["QueryExecutionId"]

        for _ in range(max_retries):
            time.sleep(5)
            status = self.athena.get_query_execution(QueryExecutionId=execution_id)
            state = status["QueryExecution"]["Status"]["State"]
            if state == "SUCCEEDED":
                return execution_id
            if state in ("FAILED", "CANCELLED"):
                reason = status["QueryExecution"]["Status"].get("StateChangeReason", "")
                raise RuntimeError(f"Athena query {state}: {reason}")

        raise TimeoutError("Athena query did not complete")

    def _delete_iceberg_rows(self, table: str, column: str, value: str):
        """Delete test rows from an Iceberg table via Athena."""
        query = f"DELETE FROM {table} WHERE {column} = '{value}'"
        try:
            self._execute_athena(query)
            logger.info("Cleaned up %s rows from %s where %s='%s'", table, table, column, value)
        except Exception as e:
            logger.warning("Failed to clean up %s: %s", table, e)

    def _query_athena(self, query: str, max_retries: int = 5) -> list[dict]:
        """Execute Athena query and return rows as dicts."""
        execution_id = self._execute_athena(query, max_retries)
        results = self.athena.get_query_results(QueryExecutionId=execution_id)
        columns = [c["Name"] for c in results["ResultSet"]["ResultSetMetadata"]["ColumnInfo"]]
        rows = []
        for row in results["ResultSet"]["Rows"][1:]:  # skip header
            values = [d.get("VarCharValue", "") for d in row["Data"]]
            rows.append(dict(zip(columns, values)))
        return rows

    # ── Scenario 1: Gauge end-to-end ──

    def test_gauge_e2e(self) -> SmokeScenarioResult:
        name = "Gauge end-to-end"
        daq_id = f"{TEST_PREFIX}gauge_{self.run_id}"
        logical_id = f"{TEST_PREFIX}gauge_logical_{self.run_id}"
        start = time.time()
        try:
            self._put_mapping(daq_id, logical_id, "gauge")
            time.sleep(5)  # let CDC propagate

            self._put_kinesis(self.input_stream, {
                "schematype": "std_json_v1",
                "customerid": "smoke_test",
                "data": [{"meterId": daq_id, "sensorId": "energy",
                          "timestamp": datetime.now(timezone.utc).isoformat(),
                          "value": 42.5, "unit": "kWh"}],
            })

            if self.verbose:
                logger.info("Waiting %ds for checkpoint...", CHECKPOINT_WAIT_S)
            time.sleep(CHECKPOINT_WAIT_S)

            rows = self._query_athena(
                f"SELECT * FROM all.logical_data "
                f"WHERE logical_id = '{logical_id}' "
                f"ORDER BY ingested_time DESC LIMIT 1"
            )

            if not rows:
                return SmokeScenarioResult(name, "FAIL", time.time() - start,
                                           expected="1 row", actual="0 rows")

            value = float(rows[0]["value"])
            if abs(value - 42.5) > 0.01:
                return SmokeScenarioResult(name, "FAIL", time.time() - start,
                                           expected="value=42.5", actual=f"value={value}")

            return SmokeScenarioResult(name, "PASS", time.time() - start)
        except Exception as e:
            return SmokeScenarioResult(name, "FAIL", time.time() - start, error=str(e))
        finally:
            self._delete_mapping(daq_id)
            self._delete_iceberg_rows("all.logical_data", "logical_id", logical_id)

    # ── Scenario 2: Counter delta end-to-end ──

    def test_counter_delta_e2e(self) -> SmokeScenarioResult:
        name = "Counter delta end-to-end"
        daq_id = f"{TEST_PREFIX}counter_{self.run_id}"
        logical_id = f"{TEST_PREFIX}counter_logical_{self.run_id}"
        start = time.time()
        try:
            self._put_mapping(daq_id, logical_id, "counter")
            time.sleep(5)

            now = datetime.now(timezone.utc)
            ts1 = now.isoformat()
            ts2 = (now.replace(second=now.second + 30) if now.second < 30
                    else now.replace(minute=now.minute + 1, second=0)).isoformat()

            for ts, val_ in [(ts1, 1000.0), (ts2, 1050.0)]:
                self._put_kinesis(self.input_stream, {
                    "schematype": "std_json_v1",
                    "customerid": "smoke_test",
                    "data": [{"meterId": daq_id, "sensorId": "energy",
                              "timestamp": ts, "value": val_, "unit": "kWh"}],
                })

            if self.verbose:
                logger.info("Waiting %ds for checkpoint...", CHECKPOINT_WAIT_S)
            time.sleep(CHECKPOINT_WAIT_S)

            rows = self._query_athena(
                f"SELECT value FROM all.logical_data "
                f"WHERE logical_id = '{logical_id}' "
                f"ORDER BY ingested_time DESC LIMIT 1"
            )

            if not rows:
                return SmokeScenarioResult(name, "FAIL", time.time() - start,
                                           expected="delta=50.0", actual="0 rows")

            value = float(rows[0]["value"])
            if abs(value - 50.0) > 0.01:
                return SmokeScenarioResult(name, "FAIL", time.time() - start,
                                           expected="delta=50.0", actual=f"delta={value}")

            return SmokeScenarioResult(name, "PASS", time.time() - start)
        except Exception as e:
            return SmokeScenarioResult(name, "FAIL", time.time() - start, error=str(e))
        finally:
            self._delete_mapping(daq_id)
            self._delete_iceberg_rows("all.logical_data", "logical_id", logical_id)

    # ── Scenario 3: Raw record write ──

    def test_raw_record(self) -> SmokeScenarioResult:
        name = "Raw record write"
        daq_id = f"{TEST_PREFIX}raw_{self.run_id}"
        start = time.time()
        try:
            self._put_kinesis(self.input_stream, {
                "schematype": "std_json_v1",
                "customerid": "smoke_test",
                "data": [{"meterId": daq_id, "sensorId": "energy",
                          "timestamp": datetime.now(timezone.utc).isoformat(),
                          "value": 99.9, "unit": "kWh"}],
            })

            if self.verbose:
                logger.info("Waiting %ds for checkpoint...", CHECKPOINT_WAIT_S)
            time.sleep(CHECKPOINT_WAIT_S)

            # Build the normalized daq_id that the pipeline creates
            normalized_daq = f"daq:std_json_v1:smoke_test:{daq_id}:energy".lower().replace("-", "_")

            rows = self._query_athena(
                f"SELECT * FROM all.raw_data "
                f"WHERE daq_id = '{normalized_daq}' "
                f"ORDER BY ingested_time DESC LIMIT 1"
            )

            if not rows:
                return SmokeScenarioResult(name, "FAIL", time.time() - start,
                                           expected="1 raw row", actual="0 rows")

            return SmokeScenarioResult(name, "PASS", time.time() - start)
        except Exception as e:
            return SmokeScenarioResult(name, "FAIL", time.time() - start, error=str(e))
        finally:
            normalized_daq = f"daq:std_json_v1:smoke_test:{daq_id}:energy".lower().replace("-", "_")
            self._delete_iceberg_rows("all.raw_data", "daq_id", normalized_daq)

    # ── Scenario 4: Dead letter routing ──

    def test_dead_letter(self) -> SmokeScenarioResult:
        name = "Dead letter routing"
        daq_id = f"{TEST_PREFIX}unmapped_{self.run_id}"
        start = time.time()
        try:
            # No mapping inserted — record should go to dead letter
            self._put_kinesis(self.input_stream, {
                "schematype": "std_json_v1",
                "customerid": "smoke_test",
                "data": [{"meterId": daq_id, "sensorId": "energy",
                          "timestamp": datetime.now(timezone.utc).isoformat(),
                          "value": 1.0, "unit": "kWh"}],
            })

            # Wait less time — dead letters route to error stream directly
            time.sleep(60)

            # Read from error stream to verify
            shard_it = self.kinesis.get_shard_iterator(
                StreamName=self.error_stream,
                ShardId="shardId-000000000000",
                ShardIteratorType="LATEST",
            )["ShardIterator"]

            found = False
            for _ in range(10):
                resp = self.kinesis.get_records(ShardIterator=shard_it, Limit=100)
                for record in resp["Records"]:
                    data = json.loads(record["Data"])
                    if daq_id in data.get("daq_id", "") or daq_id in data.get("payload", ""):
                        found = True
                        break
                if found:
                    break
                shard_it = resp["NextShardIterator"]
                time.sleep(5)

            if not found:
                return SmokeScenarioResult(name, "FAIL", time.time() - start,
                                           expected="error record in error stream",
                                           actual="not found after 10 retries")

            return SmokeScenarioResult(name, "PASS", time.time() - start)
        except Exception as e:
            return SmokeScenarioResult(name, "FAIL", time.time() - start, error=str(e))

    def run_all(self) -> SmokeReport:
        report = SmokeReport(timestamp=datetime.now(timezone.utc).isoformat())
        scenarios = [
            self.test_gauge_e2e,
            self.test_counter_delta_e2e,
            self.test_raw_record,
            self.test_dead_letter,
        ]
        for scenario_fn in scenarios:
            logger.info("Running: %s", scenario_fn.__name__)
            result = scenario_fn()
            report.add(result)
            logger.info("  %s (%0.1fs)", result.status, result.duration_s)
        return report


def main():
    parser = argparse.ArgumentParser(description="Smoke tests for DAQ Flink pipeline")
    parser.add_argument("--input-stream", required=True, help="DAQ Kinesis input stream name")
    parser.add_argument("--error-stream", required=True, help="Error Kinesis stream name")
    parser.add_argument("--verbose", action="store_true")
    args = parser.parse_args()

    logging.basicConfig(
        level=logging.DEBUG if args.verbose else logging.INFO,
        format="%(asctime)s %(levelname)s %(message)s",
    )

    runner = SmokeTestRunner(args.input_stream, args.error_stream, args.verbose)
    report = runner.run_all()
    report.print_console()
    path = report.write_json()
    logger.info("JSON report: %s", path)

    raise SystemExit(0 if report.failed == 0 else 1)


if __name__ == "__main__":
    main()
