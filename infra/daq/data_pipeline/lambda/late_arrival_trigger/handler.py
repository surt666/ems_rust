"""
Lambda trigger: consume late-arrival error records from Kinesis and
start targeted Glue Spark recomputation jobs.

Reads from the shared error Kinesis stream, filters for errorType="late_arrival",
batches by daq_id, computes time ranges, and triggers the Glue job.

Environment variables:
  GLUE_JOB_NAME          Name of the Glue recomputation job
  REGION                 AWS region
  SENSOR_IDENTITY_TABLE   DynamoDB table name
  TABLE_BUCKET_NAME      S3 Tables bucket name
  ACCOUNT_ID             AWS account ID
"""

import base64
import json
import logging
import os
from collections import defaultdict

import boto3

logger = logging.getLogger()
logger.setLevel(logging.INFO)

glue_client = boto3.client("glue")

GLUE_JOB_NAME = os.environ["GLUE_JOB_NAME"]
REGION = os.environ["REGION"]
SENSOR_IDENTITY_TABLE = os.environ["SENSOR_IDENTITY_TABLE"]
TABLE_BUCKET_NAME = os.environ["TABLE_BUCKET_NAME"]
ACCOUNT_ID = os.environ["ACCOUNT_ID"]


def handler(event, context):
    """Process Kinesis records from the error stream."""
    records = event.get("Records", [])
    if not records:
        return {"statusCode": 200, "body": "No records"}

    # Parse and filter for late_arrival records
    late_arrivals = []
    for record in records:
        try:
            payload = base64.b64decode(record["kinesis"]["data"])
            error_record = json.loads(payload)
            if error_record.get("type") == "late_arrival":
                late_arrivals.append(error_record)
        except Exception as e:
            logger.warning("Failed to parse record: %s", e)

    if not late_arrivals:
        logger.info("No late_arrival records in batch of %d", len(records))
        return {"statusCode": 200, "body": "No late arrivals"}

    logger.info("Processing %d late_arrival records", len(late_arrivals))

    # Group by daq_id and compute time ranges
    by_daq_id = defaultdict(list)
    for record in late_arrivals:
        daq_id = record.get("daq_id", "")
        if daq_id:
            # Extract timestamp from payload (format: "value=X, unit=Y, ts=Z")
            ts = extract_timestamp(record)
            if ts:
                by_daq_id[daq_id].append(ts)

    if not by_daq_id:
        logger.warning("No valid daq_ids found in late arrivals")
        return {"statusCode": 200, "body": "No valid daq_ids"}

    # Check for already-running jobs to avoid duplicates
    running_args = get_running_job_args()

    # Start Glue jobs for each group
    triggered = 0
    for daq_id, timestamps in by_daq_id.items():
        time_start = min(timestamps)
        time_end = max(timestamps)

        if is_already_running(running_args, daq_id, time_start, time_end):
            logger.info("Skipping %s [%s, %s] — job already running", daq_id, time_start, time_end)
            continue

        try:
            start_glue_job(daq_id, time_start, time_end)
            triggered += 1
        except Exception as e:
            logger.error("Failed to start Glue job for %s: %s", daq_id, e)

    logger.info("Triggered %d Glue jobs for %d daq_ids", triggered, len(by_daq_id))
    return {"statusCode": 200, "body": f"Triggered {triggered} jobs"}


def extract_timestamp(record: dict) -> str | None:
    """Extract the original event timestamp from the error record payload."""
    payload = record.get("payload", "")
    for part in payload.split(", "):
        if part.startswith("ts="):
            return part[3:]
    return record.get("timestamp")


def get_running_job_args() -> list[dict]:
    """Get arguments of currently running Glue job runs."""
    try:
        response = glue_client.get_job_runs(
            JobName=GLUE_JOB_NAME,
            MaxResults=20,
        )
        running = []
        for run in response.get("JobRuns", []):
            if run.get("JobRunState") in ("STARTING", "RUNNING", "STOPPING"):
                running.append(run.get("Arguments", {}))
        return running
    except Exception as e:
        logger.warning("Failed to check running jobs: %s", e)
        return []


def is_already_running(running_args: list[dict], daq_id: str,
                       time_start: str, time_end: str) -> bool:
    """Check if a job for the same daq_id overlapping the time range is running."""
    for args in running_args:
        job_daq_ids = args.get("--daq_ids", "")
        if daq_id in job_daq_ids.split(","):
            return True
    return False


def start_glue_job(daq_id: str, time_start: str, time_end: str):
    """Start a Glue job run for the given daq_id and time range."""
    arguments = {
        "--daq_ids": daq_id,
        "--time_range_start": time_start,
        "--time_range_end": time_end,
        "--region": REGION,
        "--sensor_identity_table": SENSOR_IDENTITY_TABLE,
        "--table_bucket_name": TABLE_BUCKET_NAME,
        "--account_id": ACCOUNT_ID,
    }
    response = glue_client.start_job_run(
        JobName=GLUE_JOB_NAME,
        Arguments=arguments,
    )
    logger.info(
        "Started Glue job %s for daq_id=%s [%s, %s]",
        response["JobRunId"], daq_id, time_start, time_end,
    )
