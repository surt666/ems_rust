#!/usr/bin/env python3
"""
Generate historical counter data and send to Kinesis for ingestion testing.

Produces cumulative (monotonically rising) counter values in std_json_v1 format.
Data is sent in batches to avoid Kinesis throttling.

Usage:
  # Single sensor, 2 months of hourly data
  python generate_historical_counters.py

  # Custom parameters
  python generate_historical_counters.py \
    --sensors 5 \
    --days 365 \
    --stream DAQ_INPUT_STREAM \
    --region eu-central-1 \
    --dry-run
"""

import argparse
import json
import random
import time
from datetime import datetime, timedelta, timezone

import boto3


def generate_sensor_data(
    meter_id: str,
    days: int,
    base_value: float = 1000.0,
    min_increment: float = 0.1,
    max_increment: float = 5.0,
) -> list[dict]:
    """Generate hourly cumulative counter readings going back `days` from now."""
    now = datetime.now(timezone.utc).replace(minute=0, second=0, microsecond=0)
    start = now - timedelta(days=days)
    hours = days * 24

    readings = []
    cumulative = base_value

    for h in range(hours):
        ts = start + timedelta(hours=h)
        # Simulate varying consumption: higher during day (6-22), lower at night
        hour_of_day = ts.hour
        if 6 <= hour_of_day <= 22:
            increment = random.uniform(min_increment * 2, max_increment)
        else:
            increment = random.uniform(min_increment, max_increment * 0.3)

        cumulative += round(increment, 3)

        readings.append({
            "gatewayid": "gw_countertest",
            "meterid": meter_id,
            "timestamp": ts.strftime("%Y-%m-%dT%H:%M:%S.000Z"),
            "sensorid": "volume",
            "value": round(cumulative, 3),
            "unit": "kWh",
        })

    return readings


def build_kinesis_messages(readings: list[dict], batch_size: int = 50) -> list[str]:
    """Package readings into std_json_v1 Kinesis messages."""
    messages = []
    for i in range(0, len(readings), batch_size):
        batch = readings[i:i + batch_size]
        msg = {
            "data": batch,
            "customerid": "countertest",
            "schematype": "std_json_v1",
        }
        messages.append(json.dumps(msg))
    return messages


def send_to_kinesis(messages: list[str], stream_name: str, region: str, dry_run: bool = False):
    """Send messages to Kinesis, respecting rate limits."""
    if dry_run:
        print(f"DRY RUN: would send {len(messages)} messages to {stream_name}")
        print(f"First message preview:\n{messages[0][:500]}...")
        print(f"Last message preview:\n{messages[-1][:500]}...")
        return

    client = boto3.client("kinesis", region_name=region)

    # Send in Kinesis PutRecords batches (max 500 per call)
    kinesis_batch_size = 100
    total_sent = 0

    for i in range(0, len(messages), kinesis_batch_size):
        batch = messages[i:i + kinesis_batch_size]
        records = [
            {
                "Data": msg.encode("utf-8"),
                "PartitionKey": f"countertest-{j}",
            }
            for j, msg in enumerate(batch)
        ]

        response = client.put_records(StreamName=stream_name, Records=records)
        failed = response.get("FailedRecordCount", 0)
        total_sent += len(records) - failed

        if failed > 0:
            print(f"  Warning: {failed} records failed in batch {i // kinesis_batch_size}")

        print(f"  Sent batch {i // kinesis_batch_size + 1}/{(len(messages) + kinesis_batch_size - 1) // kinesis_batch_size} "
              f"({total_sent}/{len(messages)} messages)")

        # Throttle to stay within Kinesis limits (1 MB/s per shard)
        time.sleep(0.5)

    print(f"Done: {total_sent} messages sent to {stream_name}")


def main():
    parser = argparse.ArgumentParser(description="Generate historical counter data for Kinesis ingestion")
    parser.add_argument("--sensors", type=int, default=1, help="Number of sensors to generate (default: 1)")
    parser.add_argument("--days", type=int, default=60, help="Days of history to generate (default: 60)")
    parser.add_argument("--stream", default="DAQ_INPUT_STREAM", help="Kinesis stream name")
    parser.add_argument("--region", default="eu-central-1", help="AWS region")
    parser.add_argument("--batch-size", type=int, default=50, help="Readings per Kinesis message (default: 50)")
    parser.add_argument("--base-value", type=float, default=1000.0, help="Starting counter value (default: 1000)")
    parser.add_argument("--dry-run", action="store_true", help="Print stats without sending to Kinesis")
    args = parser.parse_args()

    sensor_ids = [f"test_counter_{i:03d}" for i in range(1, args.sensors + 1)]

    all_readings = []
    for sensor_id in sensor_ids:
        readings = generate_sensor_data(
            meter_id=sensor_id,
            days=args.days,
            base_value=args.base_value,
        )
        all_readings.extend(readings)
        print(f"Sensor {sensor_id}: {len(readings)} readings, "
              f"range [{readings[0]['value']} → {readings[-1]['value']}] kWh")

    print(f"\nTotal: {len(all_readings)} readings across {len(sensor_ids)} sensor(s)")

    # Build and send Kinesis messages
    messages = build_kinesis_messages(all_readings, batch_size=args.batch_size)
    print(f"Packaged into {len(messages)} Kinesis messages ({args.batch_size} readings each)\n")

    send_to_kinesis(messages, args.stream, args.region, dry_run=args.dry_run)

    # Print daq_ids for DDB registration
    print(f"\nDAQ IDs to register in meter-identity:")
    for sensor_id in sensor_ids:
        daq_id = f"daq:std_json_v1:countertest:{sensor_id}:volume"
        print(f"  {daq_id}")


if __name__ == "__main__":
    main()
