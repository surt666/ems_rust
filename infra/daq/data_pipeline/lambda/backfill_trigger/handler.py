"""
Lambda trigger: on new meter-identity INSERT, backfill historical raw_data
into logical_meter_data by starting the existing Glue recomputation job.

Triggered by DynamoDB Streams on the meter-identity table.
Filters for INSERT events, extracts the daq_id from the new mapping,
and starts a targeted Glue job with time_range_end = now (the DDB write time)
so that only pre-existing raw data is backfilled — new streaming data is
handled by the Flink pipeline.

Environment variables:
  GLUE_JOB_NAME          Name of the Glue recomputation job
  REGION                 AWS region
  METER_IDENTITY_TABLE   DynamoDB table name
  TABLE_BUCKET_NAME      S3 Tables bucket name
  ACCOUNT_ID             AWS account ID
"""

import logging
import os
from datetime import datetime, timezone

import boto3

logger = logging.getLogger()
logger.setLevel(logging.INFO)

glue_client = boto3.client("glue")

GLUE_JOB_NAME = os.environ["GLUE_JOB_NAME"]
REGION = os.environ["REGION"]
METER_IDENTITY_TABLE = os.environ["METER_IDENTITY_TABLE"]
TABLE_BUCKET_NAME = os.environ["TABLE_BUCKET_NAME"]
ACCOUNT_ID = os.environ["ACCOUNT_ID"]


def handler(event, context):
    """Process DynamoDB Stream records for new meter-identity inserts."""
    records = event.get("Records", [])
    if not records:
        return {"statusCode": 200, "body": "No records"}

    # Collect daq_ids from INSERT events
    daq_ids = []
    cutoff_time = None

    for record in records:
        if record.get("eventName") != "INSERT":
            continue

        new_image = record.get("dynamodb", {}).get("NewImage", {})
        sk = new_image.get("sk", {}).get("S", "")
        if not sk:
            continue

        daq_ids.append(sk)

        # Use the DDB event timestamp as the cutoff
        epoch_seconds = record.get("dynamodb", {}).get("ApproximateCreationDateTime")
        if epoch_seconds:
            event_time = datetime.fromtimestamp(epoch_seconds, tz=timezone.utc)
            if cutoff_time is None or event_time > cutoff_time:
                cutoff_time = event_time

    if not daq_ids:
        logger.info("No INSERT events in batch of %d records", len(records))
        return {"statusCode": 200, "body": "No inserts"}

    # Use the latest event time as cutoff, fall back to now
    if cutoff_time is None:
        cutoff_time = datetime.now(timezone.utc)

    cutoff_iso = cutoff_time.strftime("%Y-%m-%dT%H:%M:%S.000000Z")
    daq_ids_csv = ",".join(daq_ids)

    logger.info(
        "New meter mapping(s): daq_ids=%s, cutoff=%s",
        daq_ids_csv, cutoff_iso,
    )

    # Check if a job for these daq_ids is already running
    if is_already_running(daq_ids):
        logger.info("Glue job already running for %s, skipping", daq_ids_csv)
        return {"statusCode": 200, "body": "Already running"}

    try:
        start_glue_job(daq_ids_csv, cutoff_iso)
    except Exception as e:
        logger.error("Failed to start Glue job for %s: %s", daq_ids_csv, e)
        raise

    return {"statusCode": 200, "body": f"Started backfill for {daq_ids_csv}"}


def is_already_running(daq_ids: list[str]) -> bool:
    """Check if a Glue job is already running for any of the given daq_ids."""
    try:
        response = glue_client.get_job_runs(
            JobName=GLUE_JOB_NAME,
            MaxResults=20,
        )
        for run in response.get("JobRuns", []):
            if run.get("JobRunState") not in ("STARTING", "RUNNING", "STOPPING"):
                continue
            job_daq_ids = run.get("Arguments", {}).get("--daq_ids", "")
            for daq_id in daq_ids:
                if daq_id in job_daq_ids.split(","):
                    return True
        return False
    except Exception as e:
        logger.warning("Failed to check running jobs: %s", e)
        return False


def start_glue_job(daq_ids_csv: str, cutoff_iso: str):
    """Start the Glue recomputation job for the given daq_ids."""
    arguments = {
        "--daq_ids": daq_ids_csv,
        "--time_range_end": cutoff_iso,
        "--region": REGION,
        "--meter_identity_table": METER_IDENTITY_TABLE,
        "--table_bucket_name": TABLE_BUCKET_NAME,
        "--account_id": ACCOUNT_ID,
    }
    response = glue_client.start_job_run(
        JobName=GLUE_JOB_NAME,
        Arguments=arguments,
    )
    logger.info(
        "Started backfill Glue job %s for daq_ids=%s, cutoff=%s",
        response["JobRunId"], daq_ids_csv, cutoff_iso,
    )
