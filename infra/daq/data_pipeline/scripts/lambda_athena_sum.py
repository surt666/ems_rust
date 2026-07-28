import csv
import io
import time
import boto3

WORKGROUP = "daq-workgroup"
REGION = "eu-central-1"
OUTPUT_BUCKET = "daq-athena-query-results-891377204778-eu-central-1"

athena = boto3.client("athena", region_name=REGION)
s3 = boto3.client("s3", region_name=REGION)


def handler(event, context):
    t0 = time.time()

    query = """
        SELECT value
        FROM "s3tablescatalog/measurements"."all"."logical_data"
        WHERE partner_id = 1
          AND company_id = 1
          AND property_id = 1
          AND building_id = 2
          AND timestamp >= timestamp '2025-01-01'
    """

    query_id = athena.start_query_execution(
        QueryString=query,
        WorkGroup=WORKGROUP,
    )["QueryExecutionId"]

    # Poll
    while True:
        status = athena.get_query_execution(QueryExecutionId=query_id)
        state = status["QueryExecution"]["Status"]["State"]
        if state == "SUCCEEDED":
            break
        if state in ("FAILED", "CANCELLED"):
            reason = status["QueryExecution"]["Status"].get("StateChangeReason", "")
            return {"error": f"{state}: {reason}"}
        time.sleep(0.5)

    stats = status["QueryExecution"]["Statistics"]
    t_athena = time.time()

    # Read CSV result file directly from S3
    body = s3.get_object(
        Bucket=OUTPUT_BUCKET,
        Key=f"{query_id}.csv",
    )["Body"].read().decode("utf-8")

    reader = csv.reader(io.StringIO(body))
    next(reader)  # skip header

    total = 0.0
    row_count = 0
    for row in reader:
        total += float(row[0])
        row_count += 1

    t_end = time.time()

    return {
        "total_value": round(total, 3),
        "row_count": row_count,
        "athena_engine_ms": stats.get("EngineExecutionTimeInMillis"),
        "athena_total_ms": stats.get("TotalExecutionTimeInMillis"),
        "data_scanned_bytes": stats.get("DataScannedInBytes"),
        "wall_athena_s": round(t_athena - t0, 2),
        "wall_read_sum_s": round(t_end - t_athena, 2),
        "wall_total_s": round(t_end - t0, 2),
    }
