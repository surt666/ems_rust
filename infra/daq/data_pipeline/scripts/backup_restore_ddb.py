#!/usr/bin/env python3
"""Backup and restore the meter-identity DynamoDB table.

Usage:
  # Backup (before stack destroy)
  uv run --with boto3 python3 backup_restore_ddb.py backup

  # Delete the retained table (after stack destroy, before redeploy)
  uv run --with boto3 python3 backup_restore_ddb.py delete

  # Restore (after stack redeploy creates the empty table)
  uv run --with boto3 python3 backup_restore_ddb.py restore
"""

import argparse
import base64
import json
import time

import boto3

TABLE_NAME = "meter-identity"
BACKUP_FILE = "meter-identity-backup.json"
REGION = "eu-central-1"


class DynamoDBEncoder(json.JSONEncoder):
    """Handle DynamoDB Binary (bytes) values by base64-encoding them."""
    def default(self, obj):
        if isinstance(obj, bytes):
            return {"__b64__": base64.b64encode(obj).decode("ascii")}
        return super().default(obj)


def decode_b64(obj):
    """Decode base64-wrapped bytes back to bytes for DynamoDB."""
    if isinstance(obj, dict) and "__b64__" in obj:
        return base64.b64decode(obj["__b64__"])
    return obj


def restore_bytes(item):
    """Walk a DynamoDB item dict and restore base64-encoded bytes."""
    restored = {}
    for key, value in item.items():
        if isinstance(value, dict):
            if "__b64__" in value:
                # This was a raw bytes value — but in DynamoDB wire format,
                # Binary is {"B": <base64-string>}, so restore it properly
                restored[key] = {"B": base64.b64decode(value["__b64__"])}
            else:
                # Check nested values (e.g. {"S": "..."}, {"N": "..."}, {"B": bytes})
                inner = {}
                for k, v in value.items():
                    if isinstance(v, dict) and "__b64__" in v:
                        inner[k] = base64.b64decode(v["__b64__"])
                    else:
                        inner[k] = v
                restored[key] = inner
        else:
            restored[key] = value
    return restored


def backup(table_name: str, output_file: str):
    client = boto3.client("dynamodb", region_name=REGION)
    items = []
    scan_kwargs = {"TableName": table_name}

    while True:
        response = client.scan(**scan_kwargs)
        items.extend(response.get("Items", []))
        if "LastEvaluatedKey" not in response:
            break
        scan_kwargs["ExclusiveStartKey"] = response["LastEvaluatedKey"]

    with open(output_file, "w") as f:
        json.dump(items, f, indent=2, cls=DynamoDBEncoder)

    print(f"Backed up {len(items)} items to {output_file}")


def delete_table(table_name: str):
    client = boto3.client("dynamodb", region_name=REGION)
    try:
        client.describe_table(TableName=table_name)
    except client.exceptions.ResourceNotFoundException:
        print(f"Table {table_name} does not exist, nothing to delete")
        return

    print(f"Deleting table {table_name}...")
    client.delete_table(TableName=table_name)

    waiter = client.get_waiter("table_not_exists")
    waiter.wait(TableName=table_name)
    print(f"Table {table_name} deleted")


def restore(table_name: str, input_file: str):
    client = boto3.client("dynamodb", region_name=REGION)

    with open(input_file, "r") as f:
        raw_items = json.load(f)

    # Restore base64-encoded bytes back to binary
    items = [restore_bytes(item) for item in raw_items]

    if not items:
        print("No items to restore")
        return

    # Wait for table to be active
    print(f"Waiting for table {table_name} to be active...")
    waiter = client.get_waiter("table_exists")
    waiter.wait(TableName=table_name)

    # Batch write in chunks of 25
    total = 0
    for i in range(0, len(items), 25):
        batch = items[i : i + 25]
        request_items = {
            table_name: [{"PutRequest": {"Item": item}} for item in batch]
        }
        response = client.batch_write_item(RequestItems=request_items)

        # Handle unprocessed items
        unprocessed = response.get("UnprocessedItems", {})
        retries = 0
        while unprocessed and retries < 5:
            time.sleep(2**retries * 0.1)
            response = client.batch_write_item(RequestItems=unprocessed)
            unprocessed = response.get("UnprocessedItems", {})
            retries += 1

        total += len(batch)
        if total % 100 == 0:
            print(f"  Restored {total}/{len(items)} items...")

    print(f"Restored {total} items to {table_name}")


def main():
    parser = argparse.ArgumentParser(description="Backup/restore meter-identity DynamoDB table")
    parser.add_argument("action", choices=["backup", "delete", "restore"],
                        help="Action to perform")
    parser.add_argument("--table", default=TABLE_NAME, help="DynamoDB table name")
    parser.add_argument("--file", default=BACKUP_FILE, help="Backup file path")
    args = parser.parse_args()

    if args.action == "backup":
        backup(args.table, args.file)
    elif args.action == "delete":
        backup(args.table, args.file)  # safety backup before delete
        delete_table(args.table)
    elif args.action == "restore":
        restore(args.table, args.file)


if __name__ == "__main__":
    main()
