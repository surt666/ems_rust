import random
from datetime import datetime, timedelta, timezone

from pyspark.context import SparkContext
from awsglue.context import GlueContext
from awsglue.job import Job
from pyspark.sql import Row
from pyspark.sql.types import (
    StructType,
    StructField,
    StringType,
    TimestampType,
    DoubleType,
    IntegerType,
)

sc = SparkContext()
glueContext = GlueContext(sc)
spark = glueContext.spark_session
job = Job(glueContext)
job.init("InsertMeterReadings", {})

spark.conf.set("spark.sql.defaultCatalog", "s3tables")
spark.conf.set("spark.sql.catalog.s3tables", "org.apache.iceberg.spark.SparkCatalog")
spark.conf.set(
    "spark.sql.catalog.s3tables.catalog-impl", "org.apache.iceberg.aws.glue.GlueCatalog"
)
spark.conf.set(
    "spark.sql.catalog.s3tables.glue.id", "891377204778:s3tablescatalog/measurements"
)
spark.conf.set("spark.sql.catalog.s3tables.warehouse", "s3://measurements/warehouse/")

table_ref = "all.logical_data"

# --- Configuration ---
partner_id = 1
company_id = 1
property_id = 1
building_ids = [1, 2]

# 4 meters: 2 per building, electricity (kWh) readings every 15 min
meters = [
    {"logical_id": "meter-001", "building_id": 1, "unit": "kWh"},
    {"logical_id": "meter-002", "building_id": 1, "unit": "kWh"},
    {"logical_id": "meter-003", "building_id": 2, "unit": "kWh"},
    {"logical_id": "meter-004", "building_id": 2, "unit": "kWh"},
]

# --- Generate one year of 15-min readings ---
start = datetime(2025, 1, 1, tzinfo=timezone.utc)
end = datetime(2026, 1, 1, tzinfo=timezone.utc)
interval_minutes = 15
now = datetime.now(timezone.utc)

rows = []
for meter in meters:
    ts = start
    random.seed(hash(meter["logical_id"]))  # reproducible per meter
    while ts < end:
        value = round(random.uniform(0.5, 5.0), 3)  # kWh per 15-min interval
        rows.append(
            Row(
                logical_id=meter["logical_id"],
                timestamp=ts,
                value=value,
                unit=meter["unit"],
                ingested_time=now,
                partner_id=partner_id,
                company_id=company_id,
                property_id=property_id,
                building_id=meter["building_id"],
                area_id=None,
                group_id=None,
            )
        )
        ts += timedelta(minutes=interval_minutes)

schema = StructType(
    [
        StructField("logical_id", StringType(), False),
        StructField("timestamp", TimestampType(), False),
        StructField("value", DoubleType(), False),
        StructField("unit", StringType(), False),
        StructField("ingested_time", TimestampType(), False),
        StructField("partner_id", IntegerType(), False),
        StructField("company_id", IntegerType(), False),
        StructField("property_id", IntegerType(), True),
        StructField("building_id", IntegerType(), True),
        StructField("area_id", IntegerType(), True),
        StructField("group_id", IntegerType(), True),
    ]
)

df = spark.createDataFrame(rows, schema)

# Sort to match the table's sort order before writing
df = df.orderBy("company_id", "property_id", "building_id", "logical_id", "timestamp")

df.writeTo(table_ref).append()

print(f"Inserted {df.count()} rows into {table_ref}")

job.commit()
