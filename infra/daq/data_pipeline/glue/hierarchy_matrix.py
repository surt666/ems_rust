"""Read a company's materialised coefficient matrix from hierarchy_new.

Holds NO domain rules. The recursion, the two defaults, derived-node detection and
the Unallocated arithmetic all live in crates/model/src/logic/formulas.rs and are
materialised into DynamoDB by the hierarchy service. This module is a GSI query.

The table lives in the hierarchy account (339712745226); the Glue job runs in the
DAQ account (891377204778) and reaches it by assuming HierarchyReaderRole.
"""


def reader_table(role_arn, region, table_name="hierarchy_new"):
    """A boto3 Table handle on hierarchy_new, via the cross-account reader role."""
    import boto3

    creds = boto3.client("sts").assume_role(
        RoleArn=role_arn, RoleSessionName="measurements-aggregate")["Credentials"]
    return boto3.resource(
        "dynamodb", region_name=region,
        aws_access_key_id=creds["AccessKeyId"],
        aws_secret_access_key=creds["SecretAccessKey"],
        aws_session_token=creds["SessionToken"]).Table(table_name)


def load_matrix(table, company_id):
    """Every (node_path, energy_type, purpose, sensor_id) -> coefficient for one company.

    `allocates` is deliberately dropped: the job never needs it, because Unallocated
    arrives as ordinary matrix rows rather than something PySpark has to subtract.
    """
    from boto3.dynamodb.conditions import Key

    rows = []
    kwargs = {
        "IndexName": "gsi1",
        "KeyConditionExpression": Key("gsi1pk").eq("W#HN2#%d" % int(company_id)),
    }
    while True:
        page = table.query(**kwargs)
        rows.extend({"node_path": r["node_path"],
                     "energy_type": r["energy_type"],
                     "purpose": r["purpose"],
                     "sensor_id": int(r["sensor_id"]),
                     "coefficient": float(r["coefficient"])}
                    for r in page.get("Items", []))
        if "LastEvaluatedKey" not in page:
            return rows
        kwargs["ExclusiveStartKey"] = page["LastEvaluatedKey"]
